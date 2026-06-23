pub const PAGE_SIZE: usize = 4096;
use crate::{
    arch,
    drivers::vfs::FSError,
    memory::{AlignToPage, PhysAddr, frame_allocator::FrameIter},
    utils::locks::{SpinLock, SpinLockGuard},
};
use bitflags::bitflags;
use core::{
    fmt::{Debug, LowerHex},
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};
use safa_abi::errors::IntoErr;
use thiserror::Error;

use super::{
    VirtAddr,
    frame_allocator::{self, Frame, FramePtr},
};

pub use crate::arch::paging::current_lower_root_table;

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
    unsafe fn unmap_range<F>(&mut self, pages: IterPage, with_each: F) -> Result<(), MapToError>
    where
        F: FnMut(Page, Frame);
    /// Does a TLB Invalidation/Makes other CPUs see unmapping changes.
    ///
    /// Safe because it only invalidates the TLB, not changing any mappings.
    fn finish_ops(&mut self, pages: IterPage);
    /// Given a page, return the frame it is mapped to.
    fn get_frame_of(&self, page: Page) -> Option<Frame>;
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Page {
    start_address: VirtAddr,
}

impl Debug for Page {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Page({:#x})", self.start_address)
    }
}

impl LowerHex for Page {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", self.start_address)
    }
}

impl Page {
    pub const fn containing(address: VirtAddr) -> Self {
        Self {
            start_address: address.to_previous_page(),
        }
    }

    pub const fn addr(&self) -> VirtAddr {
        self.start_address
    }

    /// Returns the page next to "after" `self`
    pub const fn next(&self) -> Self {
        Self {
            start_address: self.start_address + PAGE_SIZE,
        }
    }

    /// creates an iterator'able struct
    /// requires that start.start_address is smaller then end.start_address
    pub fn iter_pages(start: Page, end: Page) -> IterPage {
        assert!(start.start_address <= end.start_address);
        IterPage { start, end }
    }
}

#[derive(Debug, Clone)]
pub struct IterPage {
    start: Page,
    end: Page,
}

impl IterPage {
    #[inline(always)]
    pub const fn current(&self) -> Page {
        self.start
    }

    #[inline(always)]
    pub const fn end(&self) -> Page {
        self.end
    }

    #[inline(always)]
    pub const fn current_addr(&self) -> VirtAddr {
        self.current().start_address
    }

    #[inline(always)]
    pub const fn end_addr(&self) -> VirtAddr {
        self.end().start_address + PAGE_SIZE
    }
}

impl Iterator for IterPage {
    type Item = Page;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start < self.end {
            let page = self.start;

            self.start = self.start.next();
            Some(page)
        } else {
            None
        }
    }
}

/// Creates a new page table context.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct PageTableContext<Ops: PageTableOps> {
    ops: Ops,
}
pub type PageTable = PageTableContext<arch::paging::ArchPageTable>;

impl<Ops: PageTableOps> PageTableContext<Ops> {
    pub fn inner_mut(&mut self) -> &mut Ops {
        &mut self.ops
    }

    #[inline]
    pub unsafe fn initialize(&mut self) {
        unsafe {
            self.ops.zeroize();
            self.ops.sync_higher_half();
        }
    }

    #[inline]
    pub fn map_range_to(
        &mut self,
        pages: IterPage,
        frames: FrameIter,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        self.ops.map_range(pages, frames, flags)
    }

    #[inline]
    pub unsafe fn unmap_range<F>(&mut self, pages: IterPage, with_each: F) -> Result<(), MapToError>
    where
        F: FnMut(Page, Frame),
    {
        unsafe { self.ops.unmap_range(pages, with_each) }
    }

    #[inline]
    pub fn get_frame_of(&self, page: Page) -> Option<Frame> {
        self.ops.get_frame_of(page)
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MapToError {
    #[error("frame allocator: out of memory")]
    FrameAllocationFailed,
    #[error("fatal: attempt to map an already mapped region")]
    AlreadyMapped,
    #[error("fatal: attempt to unmap an unmapped region")]
    NotMapped,
    #[error("fatal: unknown error")]
    #[allow(dead_code)]
    Other,
}

impl IntoErr for MapToError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::AlreadyMapped => safa_abi::errors::ErrorStatus::MMapError,
            Self::FrameAllocationFailed => safa_abi::errors::ErrorStatus::OutOfMemory,
            Self::NotMapped => safa_abi::errors::ErrorStatus::MMapError,
            Self::Other => safa_abi::errors::ErrorStatus::Generic,
        }
    }
}

impl From<MapToError> for FSError {
    fn from(value: MapToError) -> Self {
        match value {
            MapToError::AlreadyMapped => FSError::MMapError,
            MapToError::FrameAllocationFailed => FSError::OutOfMemory,
            MapToError::NotMapped => FSError::MMapError,
            MapToError::Other => FSError::MMapError,
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct PageEntryFlags: u64 {
        const WRITE = 1;
        const USER_ACCESSIBLE = 1 << 1;
        const DISABLE_EXEC = 1 << 2;
        const DEVICE_UNCACHEABLE = 1 << 3;
        const FRAMEBUFFER_CACHED = 1 << 4;
    }
}

/// allocates a pml4 and returns its physical address
fn allocate_pml4<'a>() -> Result<FramePtr<PageTable>, MapToError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
    let mut table: FramePtr<PageTable> = unsafe { frame.into_ptr() };

    unsafe { table.initialize() };

    Ok(table)
}

#[repr(C)]
/// A wrapper around a Physically allocated page table
/// when the PhysPageTable is dropped it will free the whole page table so be careful with it
#[derive(Debug)]
pub struct PhysPageTable {
    inner: FramePtr<PageTable>,
}

impl Deref for PhysPageTable {
    type Target = PageTable;
    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl DerefMut for PhysPageTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

impl PhysPageTable {
    pub fn create() -> Result<Self, MapToError> {
        let inner = allocate_pml4()?;
        Ok(Self { inner })
    }

    /// creates a new PhysPageTable from the current pml4 table
    /// takes ownership of the current lower half root page table meaning it will free it when the PhysPageTable is dropped
    pub unsafe fn from_current() -> Self {
        unsafe {
            let inner = current_lower_root_table();
            Self {
                inner: inner.cast_sized(),
            }
        }
    }

    pub fn phys_addr(&self) -> PhysAddr {
        self.inner.phys_addr()
    }

    pub fn frame_ptr(&self) -> FramePtr<PageTable> {
        self.inner
    }
}

impl Drop for PhysPageTable {
    fn drop(&mut self) {
        unsafe { self.ops.deallocate() };
        // actually deallocating the page table
        let frame = self.inner.frame();
        frame_allocator::deallocate_frame(frame);
    }
}
unsafe impl Send for PhysPageTable {}

#[derive(Debug)]
/// A wrapper to perform safe Synced operations on a [`FramePtr<PageTable>`].
pub struct SyncPageTable(SpinLock<FramePtr<PageTable>>);
impl SyncPageTable {
    #[inline(always)]
    pub const unsafe fn new(ptr: FramePtr<PageTable>) -> Self {
        SyncPageTable(SpinLock::new(ptr))
    }

    #[allow(unused)]
    pub unsafe fn inner_ptr_mut(&mut self) -> &mut FramePtr<PageTable> {
        unsafe { &mut *self.0.data_ptr() }
    }

    pub unsafe fn inner_ptr(&self) -> &FramePtr<PageTable> {
        unsafe { &*self.0.data_ptr() }
    }

    /// Begin an operation or more on this Page Table.
    pub fn begin<'a>(&'a self) -> PendingOp<'a> {
        PendingOp {
            guard: ManuallyDrop::new(self.0.lock()),
            pending_unmaps: PendingUnmaps::new(),
        }
    }
}

/// A PMM Allocated buffer of frames to flush.
struct PendingFlushesPage {
    frames: heapless::Vec<Frame, 510>,
    next: Option<FramePtr<Self>>,
}

impl PendingFlushesPage {
    pub const fn new() -> Self {
        Self {
            frames: heapless::Vec::new(),
            next: None,
        }
    }

    /// Pushes a frame to the pending frames buffer, expanding the buffer if necessary.
    ///
    /// TODO: Keep track of end?
    pub fn push(&mut self, frame: Frame) -> Result<Option<FramePtr<Self>>, MapToError> {
        if let Err(frame) = self.frames.push(frame) {
            match self.next {
                Some(mut next) => Ok(next.push(frame)?.or(Some(next))),
                None => {
                    let frame_of_next = frame_allocator::allocate_frame()
                        .ok_or(MapToError::FrameAllocationFailed)?;
                    let mut ptr = unsafe { frame_of_next.into_ptr::<PendingFlushesPage>() };
                    unsafe { core::ptr::write(ptr.as_ptr(), Self::new()) };
                    self.next = Some(ptr);
                    Ok(ptr.push(frame)?.or(Some(ptr)))
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Executes a function on each frame in the pending frames buffer.
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(Frame),
    {
        for frame in &self.frames {
            f(*frame);
        }
        if let Some(next) = self.next {
            next.for_each(f);
        }
    }
}

impl Drop for PendingFlushesPage {
    fn drop(&mut self) {
        let next = self.next.take();
        if let Some(next) = next {
            let frame = next.frame();

            unsafe { core::ptr::drop_in_place(next.as_ptr()) };
            frame_allocator::deallocate_frame(frame);
        };
    }
}

const _: () = assert!(size_of::<PendingFlushesPage>() == PAGE_SIZE);

struct PendingUnmaps {
    unmap_range: Option<(Page, Page)>,
    flushes_page: PendingFlushesPage,
    flushes_tail: Option<FramePtr<PendingFlushesPage>>,
}

impl PendingUnmaps {
    fn new() -> Self {
        Self {
            unmap_range: None,
            flushes_page: PendingFlushesPage::new(),
            flushes_tail: None,
        }
    }

    #[inline(always)]
    fn on_each_unmap(&mut self, frame: Frame) {
        if let Some(tail) = self.flushes_tail.as_mut() {
            if let Some(new_tail) = tail
                .push(frame)
                .expect("Failed to alloc memory for flushing unmaps")
            {
                self.flushes_tail = Some(new_tail);
            }
        } else {
            if let Some(new_tail) = self
                .flushes_page
                .push(frame)
                .expect("Failed to alloc memory for flushing unmaps")
            {
                self.flushes_tail = Some(new_tail);
            }
        }
    }

    fn after_unmap(&mut self, start_page: Page, end_page: Page) {
        if let Some((mut start, mut end)) = self.unmap_range {
            if start_page < start {
                start = start_page;
            }

            if end_page > end {
                end = end_page;
            }

            self.unmap_range = Some((start, end));
        } else {
            self.unmap_range = Some((start_page, end_page));
        }
    }

    pub unsafe fn apply(&mut self, table: &mut PageTable) {
        if let Some((start, end)) = self.unmap_range {
            debug_assert!(start <= end);

            let pages = Page::iter_pages(start, end.next());
            table.ops.finish_ops(pages);

            self.flushes_page
                .for_each(|frame| frame_allocator::deallocate_frame(frame));
        }
    }
}

// A pending Page Table operation, not completed until dropped, while this is alive a lock is held on the Page Table pointer.
//
// Because some bad hardware implementations may require interrupts (ahem ahem x86_64) in other CPUs for TLB Invalidation to be done, one must not hold another SpinLock before dropping this.
pub struct PendingOp<'a> {
    guard: ManuallyDrop<SpinLockGuard<'a, FramePtr<PageTable>>>,
    pending_unmaps: PendingUnmaps,
}

impl<'a> PendingOp<'a> {
    /// Returns a reference to the page table.
    ///
    /// Safety: Map and Unmap operations must be done with [`PendingOp`] methods.
    pub unsafe fn page_table_mut(&mut self) -> &mut PageTable {
        &mut self.guard
    }

    #[inline]
    /// Maps a virtual `Page` to physical `Frame`.
    pub fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        self.guard.map_range_to(
            Page::iter_pages(page, page.next()),
            Frame::iter_frames(
                frame,
                Frame::containing_address(frame.phys_addr() + PAGE_SIZE),
            ),
            flags,
        )?;
        Ok(())
    }

    /// Unmaps `pages` pages starting at `addr` without deallocating the frames.
    pub unsafe fn unmap(&mut self, addr: VirtAddr, pages: usize) -> Result<(), MapToError> {
        unsafe {
            let start = Page::containing(addr);
            let end = Page::containing(addr + (pages * PAGE_SIZE));
            let pages = Page::iter_pages(start, end);

            self.guard.unmap_range(pages, |_, _| {})?;
            self.pending_unmaps.after_unmap(start, end);
            Ok(())
        }
    }

    /// Sets the flags of `pages` pages from `addr` to flags `flags`.
    pub unsafe fn set_flags(
        &mut self,
        addr: VirtAddr,
        pages: usize,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        unsafe {
            let start = Page::containing(addr);
            let end = Page::containing(addr + (pages * PAGE_SIZE));
            let pages = Page::iter_pages(start, end);

            self.guard.ops.set_flags_range(pages, flags)?;
            // TODO:
            // This is valid, after_unmap and pending_unmaps need a rename.
            self.pending_unmaps.after_unmap(start, end);
            Ok(())
        }
    }

    /// maps virtual pages from Page `from` to Page `to` with `flags` in `self`
    /// returns Err if any of the frames couldn't be allocated
    /// the mapped pages are zeroed
    ///
    /// flushes the cache if successful
    ///
    /// returns the end virtual address aligned up to PAGE_SIZE
    #[must_use = "the actual end address is returned"]
    pub fn alloc_map(
        &mut self,
        from: VirtAddr,
        to: VirtAddr,
        flags: PageEntryFlags,
    ) -> Result<VirtAddr, MapToError> {
        let end_addr = to.to_next_page();

        let from_page = Page::containing(from);
        let to_page = Page::containing(end_addr);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
            let virt_addr = frame.virt_addr();
            self.map_to(page, frame, flags)?;

            unsafe {
                core::ptr::write_bytes(virt_addr.into_ptr::<u8>(), 0, PAGE_SIZE);
            }
        }

        Ok(end_addr)
    }

    #[inline]
    /// Maps a contiguous range of pages to frames from an iterator.
    /// `start_page` is the first page to map, and `frames` is an iterator over the frames to map to.
    pub unsafe fn map_contiguous_to_frames<I: Iterator<Item = Frame>>(
        &mut self,
        start_page: Page,
        frames: I,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        let mut current_page = start_page;
        for frame in frames {
            self.map_to(current_page, frame, flags)?;
            current_page = current_page.next();
        }
        Ok(())
    }

    /// Deallocates and unmaps pages from `from` to `from + (pages * PAGE_SIZE)`.
    pub unsafe fn unmap_dealloc(&mut self, from: VirtAddr, pages: usize) -> Result<(), MapToError> {
        let from_page = Page::containing(from);
        let to_page = Page::containing(from + (pages * PAGE_SIZE));

        let pages = Page::iter_pages(from_page, to_page);

        unsafe {
            self.guard.unmap_range(pages, |_, frame| {
                self.pending_unmaps.on_each_unmap(frame);
            })?;
            self.pending_unmaps.after_unmap(from_page, to_page);
            Ok(())
        }
    }
}

impl<'a> Drop for PendingOp<'a> {
    fn drop(&mut self) {
        let mut inner_ptr = **self.guard;
        unsafe {
            ManuallyDrop::drop(&mut self.guard);
            self.pending_unmaps.apply(&mut *inner_ptr)
        };
    }
}
