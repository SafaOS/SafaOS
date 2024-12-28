//! bump allocator for large kernel allocations
//! it is still kinda slow for really large allocations it takes about 1 second to allocate 4 mbs
//! makes use of memmory mapping and `FrameAllocator` (TODO: check if these are possible reasons it
//! is slow)

use core::alloc::{AllocError, Allocator, GlobalAlloc};

use lazy_static::lazy_static;

use crate::{debug, utils::Locked};

use super::{
    align_up, frame_allocator,
    paging::{current_root_table, EntryFlags, IterPage, MapToError, Page, PAGE_SIZE},
    sorcery::ROOT_BINDINGS,
};

pub struct PageAllocator {
    heap_start: usize,
    heap_end: usize,
    last_allocation: (usize, usize),
    allocations: usize,
}

impl PageAllocator {
    pub fn new() -> Self {
        let (start, size) = ROOT_BINDINGS
            .get("LARGE_HEAP")
            .expect("failed to get LARGE_HEAP binding");
        debug!(PageAllocator, "initialized allocator");
        Self {
            heap_start: start as usize,
            heap_end: start as usize + size,
            last_allocation: (start as usize, start as usize),
            allocations: 0,
        }
    }

    /// allocates `page_count` number of contiguous pages
    /// returns a pointer to the start of the allocated memory, or an error if allocation fails.
    pub fn allocmut(&mut self, page_count: usize) -> Result<*mut u8, MapToError> {
        let start = self.last_allocation.1;

        let end = start + page_count * PAGE_SIZE;
        let start_page = Page::containing_address(start);

        if end > self.heap_end {
            return Err(MapToError::FrameAllocationFailed);
        }

        let iter = IterPage {
            start: start_page,
            end: Page::containing_address(end),
        };

        for page in iter {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
            unsafe {
                current_root_table().map_to(
                    page,
                    frame,
                    EntryFlags::PRESENT | EntryFlags::WRITABLE,
                )?;
            }
        }
        self.last_allocation = (start, end);
        self.allocations += 1;
        Ok(start_page.start_address as *mut u8)
    }

    unsafe fn deallocmut(&mut self, ptr: *mut u8, size: usize) {
        let start = ptr as usize;
        let end = start + size;
        let iter = IterPage {
            start: Page::containing_address(start),
            end: Page::containing_address(end),
        };
        for page in iter {
            unsafe {
                current_root_table().unmap(page);
            }
        }

        self.allocations -= 1;
        if self.allocations == 0 {
            self.last_allocation = (self.heap_start, self.heap_start);
        }
    }
}

unsafe impl GlobalAlloc for Locked<PageAllocator> {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        self.inner
            .lock()
            .allocmut(layout.size().div_ceil(PAGE_SIZE))
            .unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.inner.lock().deallocmut(ptr, layout.size())
    }
}

unsafe impl Allocator for Locked<PageAllocator> {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, core::alloc::AllocError> {
        unsafe {
            let ptr = self.alloc(layout);
            if ptr.is_null() {
                return Err(AllocError);
            }

            let length = align_up(layout.size(), PAGE_SIZE);

            let slice = core::ptr::slice_from_raw_parts_mut(ptr, length);
            Ok(core::ptr::NonNull::new(slice).unwrap_unchecked())
        }
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        self.dealloc(ptr.as_ptr(), layout);
    }
}

lazy_static! {
    pub static ref GLOBAL_PAGE_ALLOCATOR: Locked<PageAllocator> = Locked::new(PageAllocator::new());
}

pub type PageAlloc = &'static Locked<PageAllocator>;
