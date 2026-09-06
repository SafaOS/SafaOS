use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use bitflags::bitflags;
use thiserror::Error;

use crate::{
    arch::{self, paging::ArchPageTable},
    memory::{
        phys_to_virt,
        pmm::{self, PMMError},
        virt_to_phys,
    },
    misc::{Frame, FrameIter, IterPage, PAGE_SIZE, Page, PhysAddr, VirtAddr},
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
    /// Given an iterator of pages, unmap the pages, if `deallocate` is true may deallocate the unmapped frames.
    ///
    /// Unmapping is unsafe because you are changing existing mappings.
    ///
    /// If `lazy` is set unmap wouldn't be performed in case the page wasn't mapped, aka won't return an error.
    unsafe fn unmap_range(
        &mut self,
        pages: IterPage,
        deallocate: bool,
        lazy: bool,
    ) -> Result<(), MapToError>;
    /// Given a page, return the frame it is mapped to.
    fn get_frame_of(&self, page: Page) -> Option<Frame>;
}

/// Represents a Page Table context includes:
/// Page Table and ASID.
#[derive(Debug)]
pub struct PageTableContext<T: PageTableOps> {
    inner: NonNull<T>,
}
pub type PageTable = PageTableContext<arch::paging::ArchPageTable>;
unsafe impl Send for PageTable {}

impl PageTable {
    /// Returns a pointer to the current kernel page table.
    ///
    /// Safety: this will grant muttable access to the kernel page without sync.
    pub unsafe fn current() -> Self {
        let page_table = arch::paging::kernel_current();
        Self { inner: page_table }
    }

    /// Maps a single virtual `page` directly to a single physical `frame` with given `flags`.
    ///
    /// shouldn't shootdown the TLB or do any IPIs.
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

    /// Maps virtual pages from Page `from` to Page `to` with `flags` in `self`
    /// returns Err if any of the frames couldn't be allocated.
    ///
    /// the mapped pages are zeroed
    ///
    /// Returns the end virtual address aligned up to PAGE_SIZE.
    ///
    /// shouldn't invalidate or do any cache shootdown*
    #[must_use = "the actual end address is returned"]
    pub fn alloc_map(
        &mut self,
        from: VirtAddr,
        to: VirtAddr,
        flags: PageEntryFlags,
    ) -> Result<VirtAddr, MapToError> {
        let end_addr = to.next_page();

        let from_page = Page::containing(from);
        let to_page = Page::containing(end_addr);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            let frame = pmm::allocate_frame()?;
            let virt_addr = phys_to_virt(frame.addr());

            unsafe {
                core::ptr::write_bytes(virt_addr.into_ptr::<u8>(), 0, PAGE_SIZE);
                if let Err(err) = self.map_to(page, frame, flags) {
                    pmm::deallocate_frame(frame);
                    return Err(err);
                }
            }
        }

        Ok(end_addr)
    }

    /// Maps only pages that are not already present.
    ///
    /// This is used by lazy fault recovery.
    ///
    /// The local TLB should be manually shootdown after this operation.
    #[must_use = "the actual end address is returned"]
    pub unsafe fn alloc_map_missing(
        &mut self,
        from: VirtAddr,
        to: VirtAddr,
        flags: PageEntryFlags,
    ) -> Result<VirtAddr, MapToError> {
        let end_addr = to.next_page();

        let from_page = Page::containing(from);
        let to_page = Page::containing(end_addr);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            if unsafe { self.inner.as_ref() }.get_frame_of(page).is_some() {
                continue;
            }

            let frame = pmm::allocate_frame()?;
            let virt_addr = phys_to_virt(frame.addr());

            unsafe {
                core::ptr::write_bytes(virt_addr.into_ptr::<u8>(), 0, PAGE_SIZE);
                if let Err(err) = self.map_to(page, frame, flags) {
                    pmm::deallocate_frame(frame)
                        .expect("failed to deallocate just allocated frame on error");
                    return Err(err);
                }
            }
        }

        Ok(end_addr)
    }

    /// Unmaps `pages` pages starting at `addr` without deallocating the frames.
    ///
    /// May flush the TLB, and in x86_64 this requires a shootdown IPI to be sent and handled.
    pub unsafe fn unmap_no_dealloc(
        &mut self,
        addr: VirtAddr,
        pages: usize,
        lazy: bool,
    ) -> Result<(), MapToError> {
        let start = Page::containing(addr);
        let end = Page::containing(addr + (pages * PAGE_SIZE));
        let pages = Page::iter_pages(start, end);

        unsafe {
            self.inner.as_mut().unmap_range(pages, false, lazy)?;
            Ok(())
        }
    }

    /// Deallocates and unmaps pages from `from` to `from + (pages * PAGE_SIZE)`.
    ///
    /// May flush the TLB, and in x86_64 this requires a shootdown IPI to be sent and handled.
    ///
    /// Unsafe because it changes existing page table mapping.
    pub unsafe fn unmap_dealloc(
        &mut self,
        from: VirtAddr,
        pages: usize,
        lazy: bool,
    ) -> Result<(), MapToError> {
        let from_page = Page::containing(from);
        let to_page = Page::containing(from + (pages * PAGE_SIZE));

        let pages = Page::iter_pages(from_page, to_page);

        unsafe {
            self.inner.as_mut().unmap_range(pages, true, lazy)?;
            Ok(())
        }
    }

    /// Sets the flags of `pages` pages from `addr` to flags `flags`.
    ///
    /// May flush the TLB, and in x86_64 this requires a shootdown IPI to be sent and handled.
    ///
    /// Unsafe because it changes existing mapping.
    pub unsafe fn set_flags(
        &mut self,
        addr: VirtAddr,
        pages: usize,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        let start = Page::containing(addr);
        let end = Page::containing(addr + (pages * PAGE_SIZE));
        let pages = Page::iter_pages(start, end);

        unsafe { self.inner.as_mut().set_flags_range(pages, flags)? };
        Ok(())
    }

    /// Returns the physical address of this page table.
    pub fn phys_addr(&self) -> PhysAddr {
        virt_to_phys(VirtAddr::from_ptr(self.inner.as_ptr()))
    }
}

#[derive(Debug)]
/// A page table that is deallocated, alongside all it's entries on drop.
pub struct OwnedPageTable {
    table: PageTable,
}

impl OwnedPageTable {
    /// Creates an Owned PageTable that is destroyed on Drop.
    pub fn create() -> Result<Self, PMMError> {
        let frame = pmm::allocate_frame().expect("Failed to allocate memory for a page table");

        let ptr = phys_to_virt(frame.addr()).into_ptr::<ArchPageTable>();
        unsafe {
            (*ptr).zeroize();
        }

        Ok(Self {
            table: PageTableContext {
                inner: NonNull::new(ptr).unwrap(),
            },
        })
    }

    /// Converts self to a plain [`PageTable`] ignoring [`Drop`].
    #[inline(always)]
    pub fn into_table(self) -> PageTable {
        let table_ptr = self.table.inner;
        core::mem::forget(self);

        PageTableContext { inner: table_ptr }
    }

    /// Returns the physical address of an owned page table.
    #[inline(always)]
    pub fn phys_addr(&self) -> PhysAddr {
        self.table.phys_addr()
    }
}

impl Drop for OwnedPageTable {
    fn drop(&mut self) {
        unsafe { self.table.inner.as_mut().deallocate() };
        unsafe {
            pmm::deallocate_frame(Frame::containing(self.phys_addr()))
                .expect("Failed to deallocate an owned page table")
        }
    }
}

impl Deref for OwnedPageTable {
    type Target = PageTable;
    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl DerefMut for OwnedPageTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.table
    }
}
