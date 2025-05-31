use core::sync::atomic::AtomicUsize;

use super::{
    frame_allocator::FramePtr,
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
    memory::{align_up, frame_allocator},
    PhysAddr,
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
        HHDM_END.store(hhdm_end.into_raw(), core::sync::atomic::Ordering::Relaxed);
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

    for entry in limine::mmap_request().entries() {
        let phys_addr = PhysAddr::from(entry.base as usize);
        let size_bytes = entry.length as usize;
        let size = align_up(size_bytes, PAGE_SIZE);
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
