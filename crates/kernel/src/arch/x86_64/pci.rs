use super::acpi;
use crate::{
    arch::paging::{DEVICE_MAPPING_END, DEVICE_MAPPING_START},
    drivers::pci::PCI,
};

pub fn init() -> PCI {
    let mcfg = *acpi::MCFG_DESC;
    let entry = mcfg
        .nth(0)
        .expect("failed to get the PCIe configuration space base info");
    assert_eq!(entry.pci_sgn, 0);

    let addr = entry.physical_addr;
    assert!(addr >= DEVICE_MAPPING_START);
    assert!(addr <= DEVICE_MAPPING_END);

    PCI::new(addr, entry.pci_num0, entry.pci_num1)
}
