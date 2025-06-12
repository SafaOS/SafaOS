use super::xhci::XHCI;
use bitflags::bitflags;
use core::{fmt::Debug, u32, u64};
use lazy_static::lazy_static;
use msi::{MSIXCap, MSIXInfo};

use crate::PhysAddr;
pub mod msi;

pub trait PCIDevice: Send + Sync + Debug {
    fn create(info: PCIDeviceInfo) -> Self
    where
        Self: Sized;
    /// Starts the PCI Device returning true if successful
    fn start(&'static self) -> bool;
    /// Returns the devices class, subclass and prog_if
    fn class() -> (u8, u8, u8)
    where
        Self: Sized;
}

pub struct PCI {
    base_ptr: *const (),
    start_bus: u8,
    end_bus: u8,
}

unsafe impl Send for PCI {}
unsafe impl Sync for PCI {}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct PCICommandReg: u16 {
        const IO_SPACE = 1 << 0;
        const MEM_SPACE = 1 << 1;
        const BUS_MASTER = 1 << 2;
        const PARITY_ERR = 1 << 6;
        const SSER = 1 << 8;
        const INTERRUPT_MASK = 1 << 10;
    }

    #[derive(Debug, Clone, Copy)]
    pub struct PCIStatusReg: u16 {
        const INT_STATUS = 1 << 3;
        const CAPS_LIST = 1 << 4;
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Captability {
    id: u8,
    next_off: u8,
}

struct CaptabilitiesIter {
    base_ptr: *const (),
    current: *const Captability,
}

impl CaptabilitiesIter {
    fn new(base_ptr: *const (), cap_off: u8) -> Self {
        let current = unsafe { base_ptr.byte_add(cap_off as usize) as *const Captability };
        Self { base_ptr, current }
    }

    fn empty() -> Self {
        Self {
            base_ptr: core::ptr::null(),
            current: core::ptr::null(),
        }
    }

    /// Find a captabilitiy with the id `id`
    fn find(self, id: u8) -> Option<*const Captability> {
        for cap_ptr in self {
            let cap = unsafe { *cap_ptr };
            if cap.id == id {
                return Some(cap_ptr);
            }
        }

        None
    }
    /// Find a captability with the `id` id and then casts it to a pointer of T
    fn find_cast<T>(self, id: u8) -> Option<*const T> {
        self.find(id).map(|ptr| ptr.cast())
    }
}

impl Iterator for CaptabilitiesIter {
    type Item = *const Captability;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            return None;
        }

        let next_off = unsafe { (*self.current).next_off };
        let results = self.current;

        if next_off == 0 {
            self.current = core::ptr::null();
        } else {
            self.current =
                unsafe { self.base_ptr.byte_add(next_off as usize) as *const Captability };
        }
        Some(results)
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct CommonPCIHeader {
    vendor_id: u16,
    device_id: u16,
    // reg 1
    pub command: PCICommandReg,
    status: PCIStatusReg,
    // reg 2
    revision: u8,
    prog_if: u8,
    subclass: u8,
    class: u8,
    // reg 3
    cache_line_sz: u8,
    latency_timer: u8,
    header_type: u8,
    bist: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct GeneralPCIHeader {
    pub common: CommonPCIHeader,
    bar0: u32,
    bar1: u32,
    bar2: u32,
    bar3: u32,
    bar4: u32,
    bar5: u32,
    ccp: u32,
    subsystem_vendor_id: u16,
    subsystem_id: u16,
    er_bar: u32,
    capabilities_ptr: u8,
    _reserved0: [u8; 3],
    _reserved1: u32,
    interrupt_line: u8,
    interrupt_pin: u8,
    min_grant: u8,
    max_latency: u8,
}

impl GeneralPCIHeader {
    /// Gets at most 6 base address registers addresses from the header and their sizes
    pub fn get_bars(&self) -> heapless::Vec<(PhysAddr, usize), 6> {
        let mut results = heapless::Vec::new();
        let bars_raw = [
            &raw const self.bar0,
            &raw const self.bar1,
            &raw const self.bar2,
            &raw const self.bar3,
            &raw const self.bar4,
            &raw const self.bar5,
        ];

        let mut raw_bars_iter = bars_raw.into_iter();
        while let Some(raw_bar_ptr) = raw_bars_iter.next() {
            let raw_bar = unsafe { *raw_bar_ptr };
            if raw_bar == 0 {
                continue;
            }

            let info_bits: u8 = raw_bar as u8 & 0xF;
            assert!(info_bits & 1 == 0, "I/O unimplemented");

            let locatable = (info_bits >> 1) & 0b11;

            let addr;
            let size: usize;

            // locatable is only 2 bits
            // 1 is valid but I don't handle it yet
            if locatable == 0 {
                // 32 bit address space
                unsafe {
                    // FIXME: not sure if this is safe with optimizations,
                    // maybe it is safer with a muttable reference
                    let bar_ptr = raw_bar_ptr as *mut u32;

                    let saved_bar = core::ptr::read_volatile(bar_ptr);
                    // to read the size we have to write all 1s to the BAR
                    core::ptr::write_volatile(bar_ptr, u32::MAX);
                    let neg_size = core::ptr::read_volatile(bar_ptr);
                    // write back the old value
                    core::ptr::write_volatile(bar_ptr, saved_bar);

                    addr = PhysAddr::from((saved_bar & 0xFFFFFFF0) as usize);
                    // size is basically -whatever_we_read with the information bits masked
                    size = ((!(neg_size & 0xFFFFFFF0)) + 1) as usize;
                }
            } else if locatable == 2 {
                // 64 bit address space
                // we actually need 2 bars in this case
                let _ = raw_bars_iter.next().unwrap();
                unsafe {
                    let bar_ptr = raw_bar_ptr as *mut u64;

                    let saved_bar = core::ptr::read_volatile(bar_ptr);
                    core::ptr::write_volatile(bar_ptr, u64::MAX);
                    let neg_size = core::ptr::read_volatile(bar_ptr);
                    core::ptr::write_volatile(bar_ptr, saved_bar);

                    addr = PhysAddr::from((saved_bar & 0xFFFFFFFFFFFFFFF0) as usize);
                    size = ((!(neg_size & 0xFFFFFFFFFFFFFFF0)) + 1) as usize;
                }
            } else {
                unimplemented!()
            };

            results.push((addr, size)).unwrap();
        }

        results
    }
}

#[derive(Debug)]
pub enum PCIHeader<'a> {
    General(&'a mut GeneralPCIHeader),
    Other(&'a mut CommonPCIHeader),
}

impl<'a> PCIHeader<'a> {
    fn caps_list(&self) -> CaptabilitiesIter {
        let common = self.common();
        if common.status.contains(PCIStatusReg::CAPS_LIST) {
            let base_ptr = common as *const CommonPCIHeader as *const ();
            unsafe {
                let cap_off_ptr = base_ptr.byte_add(0x34) as *const u8;
                let cap_off = *cap_off_ptr;
                CaptabilitiesIter::new(base_ptr, cap_off)
            }
        } else {
            CaptabilitiesIter::empty()
        }
    }

    fn common(&self) -> &CommonPCIHeader {
        match self {
            Self::Other(c) => c,
            Self::General(g) => &g.common,
        }
    }

    /// Unwraps into GeneralPCIHeader, panciks if it isn't a GeneralPCIHeader
    pub fn unwrap_general(&mut self) -> &mut GeneralPCIHeader {
        let header_type = self.common().header_type & 0x0F;
        match self {
            Self::General(g) => g,
            _ => panic!("expected GeneralPCIHeader with header_type: 0x0, got {header_type:#x}"),
        }
    }

    /// Gets at most 6 base address registers addresses from the header and their sizes
    pub fn get_bars(&self) -> heapless::Vec<(PhysAddr, usize), 6> {
        match self {
            Self::Other(_) => todo!(),
            Self::General(g) => g.get_bars(),
        }
    }

    fn is_valid(&self) -> bool {
        let vendor_id = self.common().vendor_id;
        vendor_id != 0xFFFF
    }

    fn is_multifunction(&self) -> bool {
        let header_type = self.common().header_type;
        (header_type & 0x80) != 0
    }

    fn get_msix_cap(&mut self, bus: u8, slot: u8, function: u8) -> Option<MSIXInfo> {
        let msix_cap_ptr = self.caps_list().find_cast::<MSIXCap>(0x11);
        msix_cap_ptr.map(|ptr| {
            let common = self.common();
            let bars = self.get_bars();
            MSIXInfo::new(
                ptr as *mut _,
                common.device_id,
                common.vendor_id,
                (bus as u32 * 256) + (slot as u32 * 8) + function as u32,
                &bars,
            )
        })
    }
}

pub struct PCIDeviceInfo<'a> {
    header: PCIHeader<'a>,
    bus: u8,
    device: u8,
    function: u8,
}

impl<'a> PCIDeviceInfo<'a> {
    fn new(header: PCIHeader<'a>, bus: u8, device: u8, function: u8) -> Self {
        Self {
            header,
            bus,
            device,
            function,
        }
    }

    pub fn get_msix_cap(&mut self) -> Option<MSIXInfo> {
        self.header
            .get_msix_cap(self.bus, self.device, self.function)
    }

    /// Gets at most 6 base address registers addresses from the header and their sizes
    pub fn get_bars(&self) -> heapless::Vec<(PhysAddr, usize), 6> {
        self.header.get_bars()
    }

    /// Unwraps into GeneralPCIHeader, panciks if it isn't a GeneralPCIHeader
    pub fn unwrap_general(&mut self) -> &mut GeneralPCIHeader {
        self.header.unwrap_general()
    }
}

impl PCI {
    /// Requires that `addr` is mapped in the HHDM
    /// TODO: For now only PCIe is implemented
    pub fn new(addr: PhysAddr, start_bus: u8, end_bus: u8) -> Self {
        Self {
            base_ptr: addr.into_virt().into_ptr::<()>(),
            start_bus,
            end_bus,
        }
    }

    fn create_device<T: PCIDevice + Sized>(&self) -> Option<T> {
        let (class, subclass, prog_if) = T::class();
        let header = self.lookup(class, subclass, prog_if);
        header.map(|header| T::create(header))
    }

    fn get_common_header(&self, bus: u8, slot: u8, function: u8) -> &mut CommonPCIHeader {
        let bus = bus as usize;
        let slot = slot as usize;
        let function = function as usize;

        let ptr = self.base_ptr as *const _ as *const u8;
        let offset = ((bus * 256) + (slot * 8) + function) * 4096;
        unsafe {
            let ptr = ptr.add(offset);
            let ptr = ptr as *mut CommonPCIHeader;
            &mut *ptr
        }
    }

    fn get_header<'s>(&'s self, bus: u8, slot: u8, function: u8) -> PCIHeader<'s> {
        let common = self.get_common_header(bus, slot, function);
        let ty = common.header_type & 0xF;
        unsafe {
            match ty {
                0 => PCIHeader::General(&mut *(common as *mut _ as *mut GeneralPCIHeader)),
                _ => PCIHeader::Other(common),
            }
        }
    }

    fn enum_device<'s, F>(
        &'s self,
        bus: u8,
        device: u8,
        function: u8,
        f: &F,
    ) -> Option<PCIDeviceInfo<'s>>
    where
        F: Fn(&PCIDeviceInfo) -> bool,
    {
        let header = self.get_header(bus, device, function);
        if !header.is_valid() {
            return None;
        }

        if function == 0 && header.is_multifunction() {
            for function in 1..8 {
                if let r @ Some(_) = self.enum_device(bus, device, function, f) {
                    return r;
                }
            }
        }

        let info = PCIDeviceInfo::new(header, bus, device, function);
        if f(&info) {
            return Some(info);
        }
        None
    }

    fn enum_all<'s, F>(&'s self, f: &F) -> Option<PCIDeviceInfo<'s>>
    where
        F: Fn(&PCIDeviceInfo) -> bool,
    {
        for bus in self.start_bus..self.end_bus {
            for device in 0..32 {
                if let r @ Some(_) = self.enum_device(bus, device, 0, f) {
                    return r;
                }
            }
        }
        None
    }

    fn lookup<'s>(&'s self, class: u8, subclass: u8, prog_if: u8) -> Option<PCIDeviceInfo<'s>> {
        self.enum_all(&|info| {
            let common = info.header.common();
            common.class == class && common.subclass == subclass && common.prog_if == prog_if
        })
    }

    fn print(&self) {
        self.enum_all(&|info| {
            crate::serial!(
                "PCI {}:{}:{} => {:#x?}\n",
                info.bus,
                info.device,
                info.function,
                info.header
            );

            for cap in info.header.caps_list() {
                crate::serial!("{:#x?}\n", unsafe { *cap });
            }
            false
        });
    }
}

lazy_static! {
    pub static ref HOST_PCI: PCI = crate::arch::pci::init();
    // No complicated device management necessary for now.
    pub static ref XHCI_DEVICE: Option<XHCI<'static>> =
        HOST_PCI.create_device::<XHCI>();
}

/// Initializes drivers and devices that uses the PCI
pub fn init() {
    HOST_PCI.print();
    XHCI_DEVICE.as_ref().map(|device| device.start());
}
