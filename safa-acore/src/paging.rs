use core::{fmt::Debug, ptr::NonNull};

use bitflags::bitflags;
use thiserror::Error;

use crate::{
    arch,
    misc::{Frame, FrameIter, IterPage, Page},
};

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MapToError {
    #[error("frame allocator: out of memory")]
    FrameAllocationFailed,
    #[error("fatal: attempt to map an already mapped region")]
    AlreadyMapped,
    #[error("fatal: attempt to unmap an unmapped region")]
    NotMapped,
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct PageEntryFlags: u64 {
        const WRITE = 1;
        const USER_ACCESSIBLE = 1 << 1;
        const DISABLE_EXEC = 1 << 2;
        const DEVICE_UNCACHEABLE = 1 << 3;
        const FRAMEBUFFER_CACHED = 1 << 4;
        const IS_LAZY = 1 << 5;
    }
}

/// Describes a Page Table.
pub trait PageTableOps: Debug {
    /// Sync the higher half of the page table with the current page table.
    ///
    /// Unsafe because it modifies the higher half of its entries.
    unsafe fn sync_higher_half(&mut self);
    /// Fills the page table with zeros.
    ///
    /// Unsafe because it modifies all of its entries.
    unsafe fn zeroize(&mut self);
    /// Deallocates a page table including it's entries, doesn't deallocate the higher half!
    ///
    /// Unsafe because it deallocates the page table and modifies all of its entries.
    unsafe fn deallocate(&mut self);
    /// Given an iterator of pages and frames, map the pages to the frames.
    ///
    /// Mapping is safe because you aren't changing existing mappings.
    fn map_range(
        &mut self,
        pages: IterPage,
        frames: FrameIter,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError>;
    /// Sets the entry flags for every page in `pages` to `flags`.
    ///
    /// A [`Self::finish_ops`] call is required afterwards.
    unsafe fn set_flags_range(
        &mut self,
        pages: IterPage,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError>;
    /// Given an iterator of pages, unmap the pages, and on each unmapped (page, frame), call the `with_each` function.
    ///
    /// Unmap operations are pending until [`Self::flush_unmap_ops`] is called.
    ///
    /// Unmapping is unsafe because you are changing existing mappings.
    ///
    /// If `lazy` is set unmap wouldn't be performed in case the page wasn't mapped, aka won't return an error.
    unsafe fn unmap_range<F>(
        &mut self,
        pages: IterPage,
        with_each: F,
        lazy: bool,
    ) -> Result<(), MapToError>
    where
        F: FnMut(Page, Frame);
    /// Does a TLB Invalidation/Makes other CPUs see unmapping changes.
    ///
    /// Safe because it only invalidates the TLB, not changing any mappings.
    fn finish_ops(&mut self, pages: IterPage);
    /// Given a page, return the frame it is mapped to.
    fn get_frame_of(&self, page: Page) -> Option<Frame>;
}

/// Creates a new page table context.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct PageTableContext<Ops: PageTableOps> {
    ops: Ops,
}
pub type PageTable = PageTableContext<arch::paging::ArchPageTable>;

impl PageTable {
    pub fn current() -> NonNull<Self> {
        arch::paging::kernel_current()
    }
}
