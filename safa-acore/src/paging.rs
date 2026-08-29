use core::{fmt::Debug, ptr::NonNull};

use bitflags::bitflags;
use thiserror::Error;

use crate::{
    arch,
    memory::frame_allocator::PMMError,
    misc::{Frame, FrameIter, IterPage, Page},
};

/// Address Space identifier.
pub type ASID = u16;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MapToError {
    #[error("frame allocator: out of memory")]
    OutOfMemory,
    #[error("fatal: attempt to map an already mapped region")]
    AlreadyMapped,
    #[error("fatal: attempt to unmap an unmapped region")]
    NotMapped,
}

impl From<PMMError> for MapToError {
    fn from(value: PMMError) -> Self {
        match value {
            PMMError::OutOfMemory => Self::OutOfMemory,
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct PageEntryFlags: u32 {
        /// Page is read/write instead of read-only.
        const WRITE = 1;
        /// Userspace (Ring 0, EL0, etc) has access to this page.
        const USER_ACCESSIBLE = 1 << 1;
        /// No one is allowed to execute code from this page.
        const DISABLE_EXEC = 1 << 2;
        /// Usually indicates that caching is disabled.
        const DEVICE_UNCACHEABLE = 1 << 3;
        /// Usually indicates write-combining accelerated caching.
        const FRAMEBUFFER_CACHED = 1 << 4;
        /// This should probably be never passed manually, it indicates that when unmapping/changing page flags, some entries may still be invalid, and would be filled lazily.
        const _IS_LAZY = 1 << 5;
        /// This is a hint flag that the page we are mapping is shared globally and can be ignored by implementations.
        ///
        /// The use case is with x86_64 for example where the higher half is shared between different processes and you want the TLB to work more efficentlly with it,
        ///
        /// other architectures such as aarch64 may provide other solutions such as 2 tables for each halves which we support.
        const _GLOBAL_SHARED = 1 << 6;
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

/// Represents a Page Table context includes:
/// Page Table and ASID.
#[derive(Debug, Clone)]
pub struct PageTableContext<T: PageTableOps> {
    inner: NonNull<T>,
}
pub type PageTable = PageTableContext<arch::paging::ArchPageTable>;

impl PageTable {
    /// Returns a pointer to the current kernel page table.
    pub fn current() -> Self {
        let page_table = arch::paging::kernel_current();
        Self { inner: page_table }
    }

    /// Maps a single virtual `page` directly to a single physical `frame` with given `flags`.
    pub fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        unsafe {
            self.inner.as_mut().map_range(
                Page::iter_pages(page, page.next()),
                Frame::iter_frames(frame, frame.next()),
                flags,
            )
        }
    }
}
