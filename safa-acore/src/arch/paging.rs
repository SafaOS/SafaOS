use core::ptr::NonNull;

pub use super::inner::paging::ArchPageTable;
use crate::paging::PageTable;

/// Returns a pointer to the kernel higher half page table.
///
/// For the user page table use [`user_current`], although on some implementations the kernel and the user page tables would be the same (x86_64),
pub fn kernel_current() -> NonNull<PageTable> {
    todo!()
}

/// Returns a pointer to the user lower half page table.
///
/// For the kernel page table use [`kernel_current`], although on some implementations the kernel and the user page tables would be the same (x86_64),
pub fn user_current() -> NonNull<PageTable> {
    todo!()
}
