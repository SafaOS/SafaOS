use core::sync::atomic::AtomicUsize;

use super::{
    frame_allocator::{Frame, FramePtr},
    paging::{EntryFlags, PAGE_SIZE},
    VirtAddr,
};
use ::limine::memory_map::EntryType;
use lazy_static::lazy_static;

use crate::{
    arch::{
        self,
        paging::{current_higher_root_table, set_current_higher_page_table},
    },
    debug,
    limine::{self, HHDM},
    memory::{
        align_up,
        frame_allocator::{self},
    },
};

use super::paging::{MapToError, Page, PageTable};

static HHDM_END: AtomicUsize = AtomicUsize::new(0);
lazy_static! {
    pub static ref HEAP: (usize, usize) = {
        let end = HHDM_END.load(core::sync::atomic::Ordering::Relaxed);

        (end, end + super::buddy_allocator::INIT_HEAP_SIZE)
    };
    pub static ref LARGE_HEAP: (usize, usize) = {
        let (_, end) = *HEAP;
        (end, 0xffffffff80000000)
    };
}

fn create_root_page_table() -> Result<FramePtr<PageTable>, MapToError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

    let mut table = unsafe { frame.into_ptr::<PageTable>() };
    table.zeroize();
    unsafe {
        let current = current_higher_root_table();

        let current = &*current;
        let dest = &mut *table;

        let hhdm_end = map_hhdm(dest)?;
        arch::paging::map_devices(dest)?;
        map_top_2gb(current, dest)?;
        HHDM_END.store(hhdm_end, core::sync::atomic::Ordering::Relaxed);
    }

    Ok(table)
}

unsafe fn map_hhdm(dest: &mut PageTable) -> Result<VirtAddr, MapToError> {
    debug!(
        PageTable,
        "mapping HHDM, limine's: {:#x}",
        limine::get_phy_offset()
    );
    let flags = EntryFlags::WRITE;
    let mut largest_addr: VirtAddr = 0;

    for entry in limine::mmap_request().entries() {
        if entry.entry_type != EntryType::BAD_MEMORY && entry.entry_type != EntryType::RESERVED {
            let start_addr = *HHDM + entry.base as usize;
            let end_addr = start_addr + entry.length as usize;
            let end_addr = align_up(end_addr, PAGE_SIZE);

            let start = Page::containing_address(start_addr);
            let end = Page::containing_address(end_addr);

            let page_iter = Page::iter_pages(start, end);
            for page in page_iter {
                let addr = page.start_address;
                if addr > largest_addr {
                    largest_addr = addr;
                }

                let frame_addr = addr - *HHDM;
                let frame = Frame::containing_address(frame_addr);
                dest.map_to(page, frame, flags)?;
            }
        }
    }
    debug!(
        PageTable,
        "mapped HHDM from {:#x} to {:#x}", *HHDM, largest_addr
    );
    Ok(largest_addr + PAGE_SIZE)
}

unsafe fn map_top_2gb(src: &PageTable, dest: &mut PageTable) -> Result<(), MapToError> {
    debug!(PageTable, "mapping kernel");
    let start = Page::containing_address(0xffffffff80000000);
    let end = Page::containing_address(0xffffffffffffffff);
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

pub fn init_page_table() {
    debug!(PageTable, "initializing root page table ... ");
    let previous_table = unsafe { super::paging::current_higher_root_table() };
    let table = create_root_page_table().unwrap();
    unsafe {
        set_current_higher_page_table(table);
    }
    // de-allocating the previous root table
    let frame = previous_table.frame();
    frame_allocator::deallocate_frame(frame)
}
