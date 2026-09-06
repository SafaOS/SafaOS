use core::ptr::NonNull;

pub use super::inner::paging::ArchPageTable;
use crate::misc::PhysAddr;

/// Returns a pointer to the kernel higher half page table.
///
/// For the user page table use [`user_current`], although on some implementations the kernel and the user page tables would be the same (x86_64),
pub fn kernel_current() -> NonNull<ArchPageTable> {
    super::inner::paging::current_kernel()
}

/// Returns a pointer to the user lower half page table.
///
/// For the kernel page table use [`kernel_current`], although on some implementations the kernel and the user page tables would be the same (x86_64),
pub fn user_current() -> NonNull<ArchPageTable> {
    super::inner::paging::current_user()
}

/// Sets the [`Self::kernel_current`] page table to `addr`.
///
/// Safety: Litearlly changes the page table of the current execution context, addr must be a valid pointer to a page table and that page table should have all the memory required to continue mapped.
pub unsafe fn set_kernel_current(addr: PhysAddr) {
    unsafe { super::inner::paging::set_current_kernel(addr) }
}
