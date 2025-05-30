use lazy_static::lazy_static;

use crate::{limine::HHDM, PhysAddr};

pub struct PCI {
    base_ptr: *const (),
    start_bus: u8,
    end_bus: u8,
}

unsafe impl Send for PCI {}
unsafe impl Sync for PCI {}

#[derive(Debug)]
#[repr(C)]
struct CommonPCIHeader {
    vendor_id: u16,
    device_id: u16,
    // reg 1
    command: u16,
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
struct GeneralPCIHeader {
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

#[derive(Debug)]
enum PCIHeader<'a> {
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
            base_ptr: (addr | *HHDM) as *const (),
            start_bus,
            end_bus,
        }
    }

    /// Initializes and registers drivers that uses PCI
    pub fn init_pci_devices(&self) {
        self.print();
        let xhci = self.lookup(0xc, 0x3, 0x30);
        let xhci = xhci.map(|header| {
            let PCIHeader::General(g) = header else {
                unreachable!();
            };
            g
        });
        crate::serial!("XHCI is {xhci:#x?}\n");
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

lazy_static! {
    pub static ref HOST_PCI: PCI = crate::arch::pci::init();
}

/// Initializes drivers and devices that uses the PCI
pub fn init() {
    HOST_PCI.init_pci_devices();
}
