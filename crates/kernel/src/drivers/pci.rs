use super::xhci::XHCI;
use alloc::boxed::Box;
use bitflags::bitflags;
use core::{fmt::Debug, u32, u64};
use lazy_static::lazy_static;

use crate::{info, PhysAddr};

pub trait PCIDevice: Send + Sync + Debug {
    fn name(&self) -> &'static str;
    fn create(header: PCIHeader) -> Self
    where
        Self: Sized;
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
    struct PCICommandReg: u16 {
        const IO_SPACE = 1 << 0;
        const MEM_SPACE = 1 << 1;
        const BUS_MASTER = 1 << 2;
        const PARITY_ERR = 1 << 6;
        const SSER = 1 << 8;
        const INTERRUPT_MASK = 1 << 10;
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct CommonPCIHeader {
    vendor_id: u16,
    device_id: u16,
    // reg 1
    command: PCICommandReg,
    status: u16,
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
    common: CommonPCIHeader,
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
    General(&'a GeneralPCIHeader),
    Other(&'a CommonPCIHeader),
}

impl<'a> PCIHeader<'a> {
    fn common(&self) -> &CommonPCIHeader {
        match self {
            Self::Other(c) => c,
            Self::General(g) => &g.common,
        }
    }

    /// Unwraps into GeneralPCIHeader, panciks if it isn't a GeneralPCIHeader
    pub fn unwrap_general(&self) -> &GeneralPCIHeader {
        let header_type = self.common().header_type & 0x0F;
        match self {
            Self::General(g) => &g,
            _ => panic!("expected GeneralPCIHeader with header_type: 0x0, got {header_type:#x}"),
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

    fn get_common_header(&self, bus: u8, slot: u8, function: u8) -> &CommonPCIHeader {
        let bus = bus as usize;
        let slot = slot as usize;
        let function = function as usize;

        let ptr = self.base_ptr as *const _ as *const u8;
        let offset = ((bus * 256) + (slot * 8) + function) * 4096;
        unsafe {
            let ptr = ptr.add(offset);
            let ptr = ptr as *const CommonPCIHeader;
            &*ptr
        }
    }

    fn get_header(&self, bus: u8, slot: u8, function: u8) -> PCIHeader {
        let common = self.get_common_header(bus, slot, function);
        let ty = common.header_type & 0xF;
        unsafe {
            match ty {
                0 => PCIHeader::General(&*(common as *const _ as *const GeneralPCIHeader)),
                _ => PCIHeader::Other(common),
            }
        }
    }

    fn enum_device<F>(&self, bus: u8, device: u8, function: u8, f: &F) -> Option<PCIHeader>
    where
        F: Fn(&PCIHeader) -> bool,
    {
        let header = self.get_header(bus, device, function);
        if !header.is_valid() {
            return None;
        }

        if f(&header) {
            return Some(header);
        }
        if function == 0 && header.is_multifunction() {
            for function in 1..8 {
                if let r @ Some(_) = self.enum_device(bus, device, function, f) {
                    return r;
                }
            }
        }
        None
    }

    fn enum_all<F>(&self, f: &F) -> Option<PCIHeader>
    where
        F: Fn(&PCIHeader) -> bool,
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

    fn lookup(&self, class: u8, subclass: u8, prog_if: u8) -> Option<PCIHeader> {
        self.enum_all(&|header| {
            let common = header.common();
            common.class == class && common.subclass == subclass && common.prog_if == prog_if
        })
    }

    fn print(&self) {
        self.enum_all(&|header| {
            crate::serial!("PCI => {header:#x?}\n");
            false
        });
    }
}

struct PCIDeviceManager<const N: usize> {
    devices: heapless::Vec<Box<dyn PCIDevice>, N>,
}

impl<const N: usize> PCIDeviceManager<N> {
    pub const fn new() -> Self {
        let devices: heapless::Vec<Box<dyn PCIDevice>, N> = heapless::Vec::new();
        Self { devices }
    }

    pub fn try_add<T: PCIDevice + Sized + 'static>(&mut self, pci: &PCI) -> bool {
        let created = pci.create_device::<T>();
        if let Some(created) = created {
            self.devices.push(Box::new(created)).unwrap();
            true
        } else {
            false
        }
    }

    fn debug(&self) {
        for device in &*self.devices {
            info!("detected PCI Device: {}", device.name());
        }
    }
}

lazy_static! {
    pub static ref HOST_PCI: PCI = crate::arch::pci::init();
    static ref PCI_DEVICE_MANAGER: PCIDeviceManager<1> = {
        let mut manager = PCIDeviceManager::new();
        manager.try_add::<XHCI>(&*HOST_PCI);
        manager
    };
}

/// Initializes drivers and devices that uses the PCI
pub fn init() {
    HOST_PCI.print();
    PCI_DEVICE_MANAGER.debug();
}
