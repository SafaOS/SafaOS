use crate::{
    PhysAddr,
    drivers::{interrupts::IntTrigger, pci::PCI},
    info,
    memory::{
        AlignToPage,
        paging::PAGE_SIZE,
        vmm::{self, VMMMFlags},
    },
};

use super::cpu;

pub fn init() -> Option<PCI> {
    let (start_phys_addr, size, bus_start, bus_end) = (*cpu::PCIE)?;

    info!("initializing PCI from bus: {bus_start:#x} to bus: {bus_end:#x}");

    let page_num = size.to_next_page() / PAGE_SIZE;

    let virt_addr = vmm::with_root(|vmm| {
        vmm.map_direct_phys(
            &"PCIE",
            None,
            start_phys_addr,
            page_num,
            VMMMFlags::WRITEABLE | VMMMFlags::UNCACHABLE,
        )
    })
    .expect("Failed to map memory for the PCIE");

    info!("mapped PCIe from {virt_addr:#x} with size {size:#x}");
    // FIXME: hardcoded bus numbers
    Some(PCI::new(virt_addr, bus_start as u8, bus_end as u8))
}

pub fn build_msi_data(vector: u32, trigger: IntTrigger) -> u32 {
    _ = trigger;
    vector
}
pub fn build_msi_addr() -> PhysAddr {
    super::gic::its::gits_translater_phys()
}
