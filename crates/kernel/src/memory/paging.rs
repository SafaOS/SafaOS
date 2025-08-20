pub const PAGE_SIZE: usize = 4096;
use crate::{
    arch,
    drivers::vfs::FSError,
    memory::{AlignToPage, PhysAddr},
};
use bitflags::bitflags;
use core::{
    fmt::{Debug, LowerHex},
    ops::{Deref, DerefMut},
};
use safa_abi::errors::IntoErr;
use thiserror::Error;

use super::{
    VirtAddr,
    frame_allocator::{self, Frame, FramePtr},
};

pub use crate::arch::paging::{PageTable, current_higher_root_table, current_lower_root_table};

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

#[derive(Debug, Clone)]
pub struct IterPage {
    start: Page,
    end: Page,
}

impl Page {
    pub const fn containing_address(address: VirtAddr) -> Self {
        Self {
            start_address: address.to_previous_page(),
        }
    }

    pub const fn virt_addr(&self) -> VirtAddr {
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

impl PageTable {
    pub fn flush_cache(&mut self) {
        unsafe {
            arch::flush_cache();
        }
    }

    /// maps a virtual `Page` to physical `Frame`
    /// flushes the cache if necessary
    pub unsafe fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        unsafe {
            self.map_to_uncached(page, frame, flags)?;
            self.flush_cache();
            Ok(())
        }
    }

    /// maps a virtual `Page` to a new physical `Frame` filling the frame with zeros
    /// flushes the cache if necessary
    pub unsafe fn map_zeroed(&mut self, page: Page, flags: EntryFlags) -> Result<(), MapToError> {
        unsafe {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

            if let Err(e) = self.map_zeroed_to_uncached(page, frame, flags) {
                frame_allocator::deallocate_frame(frame);
                return Err(e);
            }

            self.flush_cache();
            Ok(())
        }
    }

    /// Maps a virtual `Page` to a physical `Frame` filling the frame with zeros
    ///
    /// Doesn't flush the cache
    pub unsafe fn map_zeroed_to_uncached(
        &mut self,
        page: Page,
        frame: Frame,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        unsafe {
            self.map_to_uncached(page, frame, flags)?;

            let addr = frame.virt_addr();
            let ptr = addr.into_ptr::<[u8; PAGE_SIZE]>();
            ptr.write_bytes(0, 1);
            Ok(())
        }
    }

    /// unmaps a page, flushes the cache if necessary
    pub unsafe fn unmap(&mut self, page: Page) {
        unsafe {
            self.unmap_uncached(page);
            self.flush_cache();
        }
    }

    /// Map `page_num` pages starting at `start_virt_addr` to frames starting at `start_phys_addr` and flushes cache if successful
    pub unsafe fn map_contiguous_pages(
        &mut self,
        start_virt_addr: VirtAddr,
        start_phys_addr: PhysAddr,
        page_num: usize,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        let size = page_num * PAGE_SIZE;
        let start_page = Page::containing_address(start_virt_addr);
        let start_frame = Frame::containing_address(start_phys_addr);
        let end_page = Page::containing_address(start_virt_addr + size);
        let end_frame = Frame::containing_address(start_phys_addr + size);

        let page_iter = Page::iter_pages(start_page, end_page);
        let frame_iter = Frame::iter_frames(start_frame, end_frame);
        let iter = page_iter.zip(frame_iter);
        for (page, frame) in iter {
            unsafe {
                self.map_to_uncached(page, frame, flags)?;
            }
        }

        self.flush_cache();
        Ok(())
    }

    /// maps virtual pages from Page `from` to Page `to` with `flags` in `self`
    /// returns Err if any of the frames couldn't be allocated
    /// the mapped pages are zeroed
    ///
    /// flushes the cache if successful
    ///
    /// returns the end virtual address aligned up to PAGE_SIZE
    #[must_use = "the actual end address is returned"]
    pub unsafe fn alloc_map(
        &mut self,
        from: VirtAddr,
        to: VirtAddr,
        flags: EntryFlags,
    ) -> Result<VirtAddr, MapToError> {
        let end_addr = to.to_next_page();

        let from_page = Page::containing_address(from);
        let to_page = Page::containing_address(end_addr);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
            let virt_addr = frame.virt_addr();
            unsafe {
                self.map_to_uncached(page, frame, flags)?;
            }

            unsafe {
                core::ptr::write_bytes(virt_addr.into_ptr::<u8>(), 0, PAGE_SIZE);
            }
        }

        self.flush_cache();
        Ok(end_addr)
    }

    /// Deallocates and unmaps pages from `from` to `to` then flushes the cache if necessary
    pub unsafe fn free_unmap(&mut self, from: VirtAddr, to: VirtAddr) {
        let from_page = Page::containing_address(from);
        let to_page = Page::containing_address(to);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            unsafe {
                self.free_unmap_uncached(page);
            }
        }
        self.flush_cache();
    }

    pub unsafe fn unmap_without_freeing(&mut self, from: VirtAddr, to: VirtAddr) {
        let from_page = Page::containing_address(from);
        let to_page = Page::containing_address(to);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            unsafe {
                self.unmap_uncached(page);
            }
        }
        self.flush_cache();
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MapToError {
    #[error("frame allocator: out of memory")]
    FrameAllocationFailed,
    #[error("fatal: attempt to map an already mapped region")]
    AlreadyMapped,
}

impl IntoErr for MapToError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::AlreadyMapped => safa_abi::errors::ErrorStatus::MMapError,
            Self::FrameAllocationFailed => safa_abi::errors::ErrorStatus::OutOfMemory,
        }
    }
}

impl From<MapToError> for FSError {
    fn from(value: MapToError) -> Self {
        match value {
            MapToError::AlreadyMapped => FSError::MMapError,
            MapToError::FrameAllocationFailed => FSError::OutOfMemory,
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct EntryFlags: u64 {
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

    table.zeroize();
    table.copy_higher_half();

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
            Self { inner }
        }
    }

    pub fn phys_addr(&self) -> PhysAddr {
        self.inner.phys_addr()
    }
}

impl Drop for PhysPageTable {
    fn drop(&mut self) {
        unsafe {
            self.free(4);
            // actually deallocating the page table
            let frame = self.inner.frame();
            frame_allocator::deallocate_frame(frame);
        }
    }
}

unsafe impl Send for PhysPageTable {}
