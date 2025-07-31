use super::{
    VirtAddr,
    frame_allocator::FramePtr,
    paging::{EntryFlags, PAGE_SIZE},
};
use ::limine::memory_map::EntryType;

use crate::{
    PhysAddr,
    arch::{
        self,
        paging::{current_higher_root_table, set_current_higher_page_table},
    },
    debug,
    limine::{self, HHDM},
    memory::{AlignToPage, frame_allocator},
};

use super::paging::{MapToError, Page, PageTable};

pub const HEAP: (VirtAddr, VirtAddr) = {
    // assuming HHDM starts at 0xffff000000000000
    // this allows for 224 TiBs of HHDM
    // assuming it starts at 0xffff800000000000
    // this allows for 96 TiBs of HHDM meaning you don't really have to worry`
    let end = VirtAddr::from(0xffffe00000000000);
    // 2 TiB from end
    (end, end + (0x100000000000 / 8))
};

pub const LARGE_HEAP: (VirtAddr, VirtAddr) = {
    let (_, end) = HEAP;
    // 4 TiB from end
    (end, end + (0x100000000000 / 4))
};

fn create_root_page_table() -> Result<FramePtr<PageTable>, MapToError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

    let mut table = unsafe { frame.into_ptr::<PageTable>() };
    table.zeroize();
    unsafe {
        let current = current_higher_root_table();

        let current = &*current;
        let dest = &mut *table;

        map_hhdm(dest)?;
        arch::paging::map_devices(dest)?;
        map_top_2gb(current, dest)?;
    }

    Ok(table)
}

unsafe fn map_hhdm(dest: &mut PageTable) -> Result<VirtAddr, MapToError> {
    debug!(
        PageTable,
        "mapping HHDM, limine's: {:#x}",
        limine::get_phy_offset()
    );
    let flags = EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE;

    for entry in limine::mmap_request().entries() {
        let phys_addr = PhysAddr::from(entry.base as usize);
        let size_bytes = entry.length as usize;
        let size = size_bytes.to_next_page();

        if entry.entry_type != EntryType::BAD_MEMORY && entry.entry_type != EntryType::RESERVED {
            let virt_addr = phys_addr.into_virt();
            let page_num = size / PAGE_SIZE;

            unsafe {
                dest.map_contiguous_pages(virt_addr, phys_addr, page_num, flags)?;
            }
        }
    }

    // last possible virtual HHDM address
    // FIXME: hardcoded because if I rely on the memory map there are still some stuff out of the range of the last entry
    let largest_addr_virt = VirtAddr::from(*HHDM | 0x10000000000);
    debug!(
        PageTable,
        "mapped HHDM from {:#x} to {:?}", *HHDM, largest_addr_virt
    );
    Ok(largest_addr_virt + PAGE_SIZE)
}

unsafe fn map_top_2gb(src: &PageTable, dest: &mut PageTable) -> Result<(), MapToError> {
    unsafe {
        debug!(PageTable, "mapping kernel");
        let start = Page::containing_address(VirtAddr::from(0xffffffff80000000));
        let end = Page::containing_address(VirtAddr::from(0xffffffffffffffff));
        let iter = Page::iter_pages(start, end);
        let flags = EntryFlags::WRITE;

        for page in iter {
            let Some(frame) = src.get_frame(page) else {
                break;
            };
            dest.map_to(page, frame, flags)?;
        }
        debug!(PageTable, "mapped kernel");
        Ok(())
    }
}

pub fn init_page_table() {
    debug!(PageTable, "initializing root page table ... ");
    let _ = unsafe { super::paging::current_higher_root_table() };
    let table = create_root_page_table().unwrap();
    unsafe {
        set_current_higher_page_table(table);
    }
    // de-allocating the previous root table
    // FIXME: could still be used by other cpus so i don't free it for now
    // let frame = previous_table.frame();
    // frame_allocator::deallocate_frame(frame)
}
