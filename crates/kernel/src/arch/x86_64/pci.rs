use crate::{
    arch::paging::{DEVICE_MAPPING_END, DEVICE_MAPPING_START},
    limine::HHDM,
    PhysAddr,
};

use super::acpi;

struct PCI {
    base_ptr: *const (),
    start_bus: u8,
    end_bus: u8,
}

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

impl PCI {
    fn new(addr: PhysAddr, start_bus: u8, end_bus: u8) -> Self {
        Self {
            base_ptr: (addr | *HHDM) as *const (),
            start_bus,
            end_bus,
        }
    }

    fn get_header(&self, bus: u8, slot: u8, function: u8) -> &CommonPCIHeader {
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

    fn print(&self) {
        fn print(this: &PCI, bus: u8, slot: u8, function: u8) {
            let header = this.get_header(bus, slot, function);
            if header.vendor_id == 0xFFFF {
                return;
            }

            crate::serial!("PCI bus {bus}, slot {slot}, function {function} => {header:#x?}\n");
            if function == 0 && (header.header_type & 0x80) != 0 {
                for function in 1..8 {
                    print(this, bus, slot, function)
                }
            }
        }
        for bus in self.start_bus..self.end_bus {
            for slot in 0..32 {
                print(self, bus, slot, 0);
            }
        }
    }
}

pub fn init() {
    let mcfg = *acpi::MCFG_DESC;
    let entry = mcfg
        .nth(0)
        .expect("failed to get the PCIe configuration space base info");
    assert_eq!(entry.pci_sgn, 0);

    let addr = entry.physical_addr;
    assert!(addr >= DEVICE_MAPPING_START);
    assert!(addr <= DEVICE_MAPPING_END);

    let pci = PCI::new(addr, entry.pci_num0, entry.pci_num1);
    pci.print();
}
