use lazy_static::lazy_static;

use super::{
    acpi,
    interrupts::apic::{LAPIC_ID, LAPIC_PHYS_ADDR},
};
use crate::{
    arch::{paging::PageTable, x86_64::acpi::MCFGEntry},
    drivers::{interrupts::IntTrigger, pci::PCI},
    memory::paging::{EntryFlags, MapToError},
    PhysAddr,
};

lazy_static! {
    pub static ref PCI_MCFG_ENTRY: MCFGEntry = {
        let mcfg = *acpi::MCFG_DESC;
        let entry = mcfg
            .nth(0)
            .expect("failed to get the PCIe configuration space base info");
        entry
    };
}
/// Maps PCIe to the `dest` page table
pub unsafe fn map_pcie(dest: &mut PageTable) -> Result<(), MapToError> {
    let flags = EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE;

    let pci_entry = *PCI_MCFG_ENTRY;
    let pci_phys = pci_entry.physical_addr;
    // bus count * slot count * 4096 = size
    // page num = size / 4096
    let pci_page_num = (pci_entry.pci_num1 - pci_entry.pci_num0) as usize * 256;

    unsafe {
        dest.map_contiguous_pages(pci_phys.into_virt(), pci_phys, pci_page_num, flags)?;
    }
    Ok(())
}

pub fn init() -> PCI {
    let entry = *PCI_MCFG_ENTRY;
    assert_eq!(entry.pci_sgn, 0);

    let addr = entry.physical_addr;
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
