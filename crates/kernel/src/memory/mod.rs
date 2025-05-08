pub mod buddy_allocator;
pub mod frame_allocator;
pub mod page_allocator;
pub mod paging;
pub mod sorcery;

// types for better code reability
pub type VirtAddr = usize;
pub type PhysAddr = usize;

use paging::{current_root_table, Page, PageTable, PAGE_SIZE};

#[inline(always)]
pub const fn align_up(address: usize, alignment: usize) -> usize {
    (address + alignment - 1) & !(alignment - 1)
}

#[inline(always)]
pub const fn align_down(x: usize, alignment: usize) -> usize {
    x & !(alignment - 1)
}

#[inline(always)]
pub fn copy_to_userspace(page_table: &mut PageTable, addr: VirtAddr, obj: &[u8]) {
    let pages_required = obj.len().div_ceil(PAGE_SIZE) + 1;
    let mut copied = 0;
    let mut to_copy = obj.len();

    for i in 0..pages_required {
        let page = Page::containing_address(addr + copied);
        let diff = if i == 0 { addr - page.start_address } else { 0 };
        let will_copy = if (to_copy + diff) >= PAGE_SIZE {
            PAGE_SIZE - diff
        } else {
            to_copy
        };

        let frame = page_table.get_frame(page).unwrap();

        let virt_addr = frame.virt_addr() + diff;
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
