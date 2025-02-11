//! bump allocator for large kernel allocations
//! it is still kinda slow for really large allocations it takes about 1 second to allocate 4 mbs
//! makes use of memmory mapping and `FrameAllocator` (TODO: check if these are possible reasons it
//! is slow)

use core::alloc::{AllocError, Allocator};

use alloc::vec;
use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::{debug, utils::Locked};

use super::{
    align_up, frame_allocator,
    paging::{current_root_table, EntryFlags, MapToError, Page, PAGE_SIZE},
    sorcery::ROOT_BINDINGS,
};

/// a bitmap page allocator which allocates contiguous virtual memory pages
/// good for allocating large amounts of memory which won't be freed or reallocated much
/// slower than [`super::buddy_allocator::BuddyAllocator`]
pub struct PageAllocator {
    heap_start: usize,
    heap_end: usize,
    /// bitmap is used to keep track of which pages are used and which are free
    /// the number of bytes it contain should be aligned to usize bytes
    bitmap: Vec<usize>,
    next_large_allocation_index: usize,
    next_small_allocation_index: usize,
}

impl PageAllocator {
    pub fn new() -> Self {
        let (start, size) = ROOT_BINDINGS
            .get("LARGE_HEAP")
            .expect("failed to get LARGE_HEAP binding");

        debug!(PageAllocator, "initialized allocator",);
        Self {
            heap_start: start as usize,
            heap_end: start as usize + size,
            bitmap: vec![0; 8],
            next_large_allocation_index: 0,
            next_small_allocation_index: 0,
        }
    }
    #[inline(always)]
    fn get_addr(&self, index: usize, bit: usize) -> *mut u8 {
        let computed_addr = index * usize::BITS as usize + bit;
        (self.heap_start + (computed_addr * PAGE_SIZE)) as *mut u8
    }

    #[inline(always)]
    fn get_location(&self, addr: *mut u8) -> (usize, usize) {
        let start = addr as usize - self.heap_start;
        let abs_index = start / PAGE_SIZE;

        let index = abs_index / usize::BITS as usize;
        let bit = abs_index % usize::BITS as usize;

        (index, bit)
    }
    /// finds `page_count` number of contiguous pages
    /// returns a pointer to the start of the allocated memory, or None if allocation fails.
    /// sets the allocated pages as used in the bitmap
    pub fn find_pages(&mut self, page_count: usize) -> Option<(*mut u8, usize)> {
        let bitmap = self.bitmap.as_mut_slice();

        if page_count < usize::BITS as usize {
            let iter = bitmap
                .iter_mut()
                .enumerate()
                .skip(self.next_small_allocation_index);
            let mask = (1 << page_count) - 1;

            for (i, bytes) in iter {
                let mut byte_ref = *bytes;

                if byte_ref == 0 {
                    *bytes = mask;
                    return Some((self.get_addr(i, 0), page_count));
                }

                let mut bit = 0;

                while byte_ref & mask != 0 && bit < usize::BITS as usize {
                    byte_ref >>= page_count;
                    bit += page_count;
                }

                if bit < usize::BITS as usize {
                    *bytes |= mask << bit;

                    if self.next_small_allocation_index < i + 1 {
                        self.next_small_allocation_index = i + 1;
                    }

                    let addr = self.get_addr(i, bit);
                    return Some((addr, page_count));
                }
            }
        } else {
            let bytes = page_count.div_ceil(usize::BITS as usize);
            let mut iter = bitmap
                .iter_mut()
                .enumerate()
                .skip(self.next_large_allocation_index);

            'outer: loop {
                let mut start_index = None;
                let mut final_index = 0;
                let mut count = 0;

                while let Some((i, byte)) = iter.next() {
                    if start_index.is_none() {
                        start_index = Some(i);
                    }

                    if !(*byte == 0) {
                        continue 'outer;
                    }

                    final_index = i;
                    count += 1;
                    if count >= bytes {
                        break;
                    }
                }

                if count < bytes {
                    return None;
                }

                let start_index = start_index.unwrap();
                bitmap[start_index..final_index + 1].fill(usize::MAX);

                if self.next_large_allocation_index < final_index + 1 {
                    self.next_large_allocation_index = final_index + 1;
                }

                let addr = self.get_addr(start_index, 0);
                return Some((addr, bytes * usize::BITS as usize));
            }
        }

        None
    }

    pub fn try_find_pages(&mut self, page_count: usize) -> Option<(*mut u8, usize)> {
        match self.find_pages(page_count) {
            Some(ptr) => Some(ptr),
            None => {
                let bitcount = page_count.div_ceil(usize::BITS as usize);

                if page_count * PAGE_SIZE > self.heap_end - self.heap_start {
                    return None;
                }

                self.bitmap.reserve(bitcount);
                self.bitmap.resize(self.bitmap.len() + bitcount, 0);

                Some(self.find_pages(page_count).unwrap())
            }
        }
    }

    /// allocates `page_count` number of contiguous pages
    /// returns a pointer to the start of the allocated memory, or an error if allocation fails.
    pub fn allocmut(&mut self, page_count: usize) -> Result<(*mut u8, usize), MapToError> {
        let (ptr, pages) = self
            .try_find_pages(page_count)
            .ok_or(MapToError::FrameAllocationFailed)?;

        let start_page = Page::containing_address(ptr as usize);
        let end_page = Page::containing_address(ptr as usize + pages * PAGE_SIZE);

        for page in Page::iter_pages(start_page, end_page) {
            unsafe {
                current_root_table().map_to(
                    page,
                    frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?,
                    EntryFlags::PRESENT | EntryFlags::WRITABLE,
                )?
            }
        }
        Ok((ptr, pages))
    }

    unsafe fn deallocmut(&mut self, ptr: *mut u8, size: usize) {
        let page_count = size.div_ceil(PAGE_SIZE);

        let page_count = if page_count > usize::BITS as usize {
            align_up(page_count, usize::BITS as usize)
        } else {
            page_count
        };

        let size = page_count * PAGE_SIZE;

        let start = ptr as usize;
        let end = start + size;

        let start = Page::containing_address(start);
        let end = Page::containing_address(end);

        for page in Page::iter_pages(start, end) {
            unsafe {
                current_root_table().unmap(page);
            }
        }

        let usizes = page_count / usize::BITS as usize;

        let (start_index, start_bit) = self.get_location(ptr);

        // if we have more than 1 usizes then allocated page_count is a multiple of usize::BITS
        // else it is less then usize::BITS so we need to find the actual index
        let mask = if usizes > 1 {
            if self.next_large_allocation_index > start_index {
                self.next_large_allocation_index = start_index;
            }

            usize::MAX
        } else {
            if self.next_small_allocation_index > start_index {
                self.next_small_allocation_index = start_index;
            }

            ((1usize << page_count) - 1) << start_bit
        };
        self.bitmap[start_index] &= !mask;

        for i in start_index + 1..start_index + usizes {
            self.bitmap[i] = 0;
        }
    }
}

unsafe impl Allocator for Locked<PageAllocator> {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, core::alloc::AllocError> {
        unsafe {
            let page_count = layout.size().div_ceil(PAGE_SIZE);
            let (ptr, pages) = self
                .lock()
                .allocmut(page_count)
                .unwrap_or((core::ptr::null_mut(), 0));

            if ptr.is_null() {
                return Err(AllocError);
            }

            let length = pages * PAGE_SIZE;

            let slice = core::ptr::slice_from_raw_parts_mut(ptr, length);
            Ok(core::ptr::NonNull::new(slice).unwrap_unchecked())
        }
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        self.lock().deallocmut(ptr.as_ptr(), layout.size());
    }

    unsafe fn grow(
        &self,
        ptr: core::ptr::NonNull<u8>,
        old_layout: core::alloc::Layout,
        new_layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, AllocError> {
        if old_layout.size() % PAGE_SIZE != 0 {
            let actual_size = align_up(old_layout.size(), PAGE_SIZE);
            if actual_size >= new_layout.size() {
                let slice = core::ptr::slice_from_raw_parts_mut(ptr.as_ptr(), new_layout.size());
                return Ok(core::ptr::NonNull::new_unchecked(slice));
            }
        }

        let new_ptr = self.allocate(new_layout)?;
        core::ptr::copy_nonoverlapping(
            ptr.as_ptr(),
            new_ptr.as_ptr() as *mut u8,
            old_layout.size(),
        );
        self.deallocate(ptr, old_layout);

        Ok(new_ptr)
    }

    unsafe fn shrink(
        &self,
        ptr: core::ptr::NonNull<u8>,
        _: core::alloc::Layout,
        new_layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, AllocError> {
        let slice = core::ptr::slice_from_raw_parts_mut(ptr.as_ptr(), new_layout.size());
        Ok(core::ptr::NonNull::new_unchecked(slice))
    }
}

lazy_static! {
    pub static ref GLOBAL_PAGE_ALLOCATOR: Locked<PageAllocator> = Locked::new(PageAllocator::new());
}

pub type PageAlloc = &'static Locked<PageAllocator>;
