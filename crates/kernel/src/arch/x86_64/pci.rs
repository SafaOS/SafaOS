use super::{
    acpi,
    interrupts::apic::{LAPIC_ID, LAPIC_PHYS_ADDR},
};
use crate::{
    arch::paging::{DEVICE_MAPPING_END, DEVICE_MAPPING_START},
    drivers::{interrupts::IntTrigger, pci::PCI},
    PhysAddr,
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

pub fn build_msi_data(irq_num: u32, trigger: IntTrigger) -> u32 {
    let (trigger, assert) = match trigger {
        IntTrigger::Edge => (0, 0),
        IntTrigger::LevelDeassert => (1, 0),
        IntTrigger::LevelAssert => (1, 1),
    };

    let results = irq_num | /* TODO: Delivery */ 0 | assert << 14 | trigger << 15;
    results
}
pub fn build_msi_addr() -> PhysAddr {
    let lapic_base = (*LAPIC_PHYS_ADDR).into_raw();
    let lapic_id = *LAPIC_ID;
    let msi_addr = lapic_base | ((lapic_id as usize) << 12);
    PhysAddr::from(msi_addr)
}
