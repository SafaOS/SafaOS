use crate::memory::VirtAddr;
use crate::memory::paging::PageTable;
use core::arch::global_asm;

global_asm!(
    "
.global flush_cache_range_inner
flush_cache_range_inner:
    lsr x0, x0, #12
    lsr x1, x1, #12
    dsb ish
.Loop:
    TLBI vaae1is, x0

    add x0, x0, 1
    cmp x0, x1
    b.lo .Loop

    dsb ish
    isb
    ret
"
);

unsafe extern "C" {
    /// Performs a TLB shootdown for addresses starting at `start`, and ending at `end`.
    pub fn flush_cache_range_inner(start: VirtAddr, end: VirtAddr);
}

/// Performs a TLB shootdown for addresses starting at `start`, and ending at `end`.
///
/// Exclusive but if start == end, it is going to flush start.
///
/// # Safety
/// Current implementations for all current architectures are safe however, you must make sure `start` and `end` are a multiple of [`PAGE_SIZE`], and end > start.
pub unsafe fn flush_cache_range(page_table: &PageTable, start: VirtAddr, end: VirtAddr) {
    _ = page_table;
    super::without_interrupts(|| unsafe { flush_cache_range_inner(start, end) })
}
