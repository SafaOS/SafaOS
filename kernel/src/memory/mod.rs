pub mod buddy_allocator;
pub mod frame_allocator;
pub mod page_allocator;
pub mod paging;
pub mod sorcery;

// types for better code reability
pub type VirtAddr = usize;
pub type PhysAddr = usize;

use paging::{current_root_table, EntryFlags, MapToError, Page, PageTable, PAGE_SIZE};

use crate::{globals::global_allocator, hddm, serial};

fn p4_index(addr: VirtAddr) -> usize {
    (addr >> 39) & 0x1FF
}
fn p3_index(addr: VirtAddr) -> usize {
    (addr >> 30) & 0x1FF
}
fn p2_index(addr: VirtAddr) -> usize {
    (addr >> 21) & 0x1FF
}
fn p1_index(addr: VirtAddr) -> usize {
    (addr >> 12) & 0x1FF
}

pub fn translate(addr: VirtAddr) -> (usize, usize, usize, usize) {
    (
        p1_index(addr),
        p2_index(addr),
        p3_index(addr),
        p4_index(addr),
    )
}

#[inline(always)]
pub const fn align_up(address: usize, alignment: usize) -> usize {
    (address + alignment - 1) & !(alignment - 1)
}

#[inline(always)]
pub const fn align_down(x: usize, alignment: usize) -> usize {
    x & !(alignment - 1)
}

pub const INIT_HEAP_SIZE: usize = (1024 * 1024) / 2;

// TODO: make the memory module more generic for different architectures; for now we can only support x86_64 because of the bootloader crate so take into account making our own bootloader for aarch64
/// unsafe because `heap_start`..`INIT_HEAP_SIZE` must be unmapped
unsafe fn init_heap(heap_start: usize) -> Result<(), MapToError> {
    serial!(
        "initing the heap... 0x{:x}..0x{:x}\n",
        heap_start,
        heap_start + INIT_HEAP_SIZE
    );
    let page_range = {
        let heap_end = heap_start + INIT_HEAP_SIZE;
        let heap_start_page = Page::containing_address(heap_start);
        let heap_end_page = Page::containing_address(heap_end);
        Page::iter_pages(heap_start_page, heap_end_page)
    };

    serial!("Iter created!\n");

    let flags = EntryFlags::PRESENT | EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE;

    for page in page_range {
        let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

        unsafe {
            current_root_table().map_to(page, frame, flags)?;
        };
    }

    global_allocator()
        .lock()
        .assume_init_mut()
        .init(heap_start, INIT_HEAP_SIZE);
    serial!("init done\n");
    Ok(())
}

pub fn init(heap_start: usize) {
    let attempt = unsafe { init_heap(heap_start) };
    if let Err(err) = attempt {
        match err {
            MapToError::FrameAllocationFailed => {
                panic!("frame allocation failure while attempting to init the heap")
            }
        }
    }
}

#[inline(always)]
pub fn copy_to_userspace(page_table: &mut PageTable, addr: VirtAddr, obj: &[u8]) {
    let pages_required = obj.len().div_ceil(PAGE_SIZE) + 1;
    let mut copied = 0;
    let mut to_copy = obj.len();

    for i in 0..pages_required {
        let page = Page::containing_address(addr + copied);
        let diff = if i == 0 { addr - page.start_address } else { 0 };
        let will_copy = if to_copy > PAGE_SIZE {
            PAGE_SIZE - diff
        } else {
            to_copy
        };

        let frame = page_table.get_frame(page).unwrap();

        let phys_addr = frame.start_address() + diff;
        let virt_addr = phys_addr | hddm();
        unsafe {
            core::ptr::copy_nonoverlapping(
                obj.as_ptr().byte_add(copied),
                virt_addr as *mut u8,
                will_copy,
            );
        }

        copied += will_copy;
        to_copy -= will_copy;
    }
}
