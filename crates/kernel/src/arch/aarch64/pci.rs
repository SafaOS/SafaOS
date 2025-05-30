use crate::{
    drivers::pci::PCI,
    info,
    limine::HHDM,
    memory::{
        frame_allocator::Frame,
        paging::{EntryFlags, Page},
    },
};

use super::{cpu, paging::current_higher_root_table};

pub fn init() -> PCI {
    let (start, size, bus_start, bus_end) = *cpu::PCIE;
    info!("initializing PCI from bus: {bus_start:#x} to bus: {bus_end:#x}");
    let end_addr = start + size;

    let start_page = Page::containing_address(start | *HHDM);
    let end_page = Page::containing_address(end_addr | *HHDM);
    let pages = Page::iter_pages(start_page, end_page);
    for page in pages {
        let frame = Frame::containing_address(page.start_address - *HHDM);
        unsafe {
            current_higher_root_table()
                .map_to(
                    page,
                    frame,
                    EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE,
                )
                .expect("failed to map PCIe")
        }
    }
    info!("mapped PCIe from {start_page:#x} to {end_page:#x}");
    // FIXME: hardcoded bus numbers
    PCI::new(start, bus_start as u8, bus_end as u8)
}
