use crate::{
    drivers::{interrupts::IntTrigger, pci::PCI},
    info,
    memory::{
        align_up,
        paging::{EntryFlags, PAGE_SIZE},
    },
    PhysAddr,
};

use super::{cpu, paging::current_higher_root_table};

pub fn init() -> PCI {
    let (start_phys_addr, size, bus_start, bus_end) = *cpu::PCIE;
    let start_virt_addr = start_phys_addr.into_virt();

    info!("initializing PCI from bus: {bus_start:#x} to bus: {bus_end:#x}");

    let page_num = align_up(size, PAGE_SIZE) / PAGE_SIZE;
    unsafe {
        current_higher_root_table()
            .map_contiguous_pages(
                start_virt_addr,
                start_phys_addr,
                page_num,
                EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE,
            )
            .expect("failed to map PCIe");
    }
    info!("mapped PCIe from {start_virt_addr:#x} with size {size:#x}");
    // FIXME: hardcoded bus numbers
    PCI::new(start_phys_addr, bus_start as u8, bus_end as u8)
}

pub fn build_msi_data(vector: u8, trigger: IntTrigger) -> u32 {
    todo!()
}
pub fn build_msi_addr() -> PhysAddr {
    todo!()
}
