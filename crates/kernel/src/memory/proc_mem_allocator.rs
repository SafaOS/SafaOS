//! Describes the allocator for thread/process based things such as the stack, thread local storage, arguments and environment variables
//! This allocator manages an architecture specific area of memory and is per process

use core::hint::unlikely;

use safa_abi::ffi::slice::Slice;

use crate::{
    VirtAddr,
    arch::paging::PageTable,
    memory::{
        AlignTo, AlignToPage,
        paging::{EntryFlags, MapToError, PAGE_SIZE, Page},
    },
};

/// An allocator to allocate memory and pass data to processes, allowing tracking allocated memory and passing data from other address spaces
/// This allocator is stack based, it grows downwards
#[derive(Debug)]
pub struct ProcessMemAllocator {
    page_table: *mut PageTable,
    allocations_head: VirtAddr,
    next_allocation_end: VirtAddr,
}

/// A tracker for an allocation done using the [`ProcessMemoryAllocator`]
///
/// on drop this memory is freed
#[derive(Debug)]
pub struct TrackedAllocation {
    page_table: *mut PageTable,
    start_addr: VirtAddr,
    end_addr: VirtAddr,
}

impl TrackedAllocation {
    pub const fn end(&self) -> VirtAddr {
        self.end_addr
    }

    pub const fn start(&self) -> VirtAddr {
        self.start_addr
    }
}

impl Drop for TrackedAllocation {
    fn drop(&mut self) {
        let page_table = unsafe { &mut *self.page_table };
        unsafe {
            page_table.free_unmap(self.start_addr.to_next_page(), self.end_addr);
        }
    }
}

impl ProcessMemAllocator {
    pub const fn new(page_table: *mut PageTable, start_addr: VirtAddr, size_bytes: usize) -> Self {
        Self {
            page_table,
            allocations_head: start_addr,
            next_allocation_end: start_addr + size_bytes,
        }
    }

    fn allocate_inner(
        &mut self,
        size: usize,
        alignment: usize,
        guard_pages_count: usize,
    ) -> Result<(VirtAddr, VirtAddr), MapToError> {
        let end_addr = self.next_allocation_end.to_previous_multiple_of(alignment);
        let end_addr = end_addr - guard_pages_count * PAGE_SIZE;

        let unmapped_end_page = Page::containing_address(end_addr);

        let start_addr = end_addr - size.to_next_multiple_of(alignment);

        let unmapped_start_page = Page::containing_address(start_addr);
        // overflows return null
        if unlikely(unmapped_start_page.virt_addr() < self.allocations_head) {
            return Err(MapToError::FrameAllocationFailed);
        }

        let root_page_table = unsafe { &mut *self.page_table };

        let flags = EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE;

        let mut map_current = unmapped_start_page;
        while map_current <= unmapped_end_page {
            match unsafe { root_page_table.try_alloc_map_single_uncached(map_current, flags) } {
                Err(e) => return Err(e),
                Ok(true) => map_current = map_current.next(),
                Ok(false) => break,
            }
        }

        root_page_table.flush_cache();
        self.next_allocation_end = start_addr;
        Ok((start_addr, end_addr))
    }

    /// Allocates a "tracked" area with the size `size and N `guard_pages_count` of unmapped memory before the start (because the allocator grows downwards)
    /// tracked allocations are always page aligned (alignment is aligned to PAGE_SIZE)
    ///
    /// the area is freed as soon as it is dropped, the area depends on the page table this allocator borrows a pointer to, so it lives as long as this page table lives
    /// the area is meant to be used by the same address space it was allocated for this means it should live as long as it maximumly, so it is safe since no one else would use this
    pub fn allocate_tracked_guraded(
        &mut self,
        size: usize,
        alignment: usize,
        guard_pages_count: usize,
    ) -> Result<TrackedAllocation, MapToError> {
        let align = alignment.to_next_multiple_of(PAGE_SIZE);
        let (start_addr, end_addr) = self.allocate_inner(size, align, guard_pages_count)?;
        self.next_allocation_end -= guard_pages_count * PAGE_SIZE;

        Ok(TrackedAllocation {
            page_table: self.page_table,
            start_addr,
            end_addr,
        })
    }

    /// Allocates a region with size `data.len()` and alignment `alignment` then copies `data` to this region
    /// even if we are running in another address space than the one the allocator lives in
    pub fn allocate_filled_with(
        &mut self,
        data: &[u8],
        alignment: usize,
    ) -> Result<(VirtAddr, VirtAddr), MapToError> {
        let (start, end) = self.allocate_inner(data.len(), alignment, 0)?;

        unsafe {
            crate::memory::copy_to_userspace(&mut *self.page_table, start, data);
        }

        Ok((start, end))
    }

    /// Allocate a region for the slices `slices` to live in, and fills them in
    ///
    /// currently there is no equivalent tracked version because there is no use case for that for now
    /// # Returns
    /// - Ok((region_start_addr, region_end_addr, slices_fat_pointers_start_address)) if successful
    ///
    /// The allocated region looks like this
    /// from region_start_addr:
    ///
    /// - `0`..`8` (size_of::<usize>): length of `slices` encoded in native byte order
    /// - `8`..+(sum of for slice in slices slice.len() + 1): this is where the slices data actually live, null terminated for C compatibility
    /// - previous end aligned to 0x10..+(slices.len() * 0x10): this is where `slices_fat_pointers_start_address` is, basically contains a bunch of [`Slice<u8>`]s that points to each slices raw data, (FFI Safe version of &[u8]), the length is the slice length minus the null terminator
    pub fn allocate_filled_with_slices(
        &mut self,
        slices: &[&[u8]],
        alignment: usize,
    ) -> Result<(VirtAddr, VirtAddr, VirtAddr), MapToError> {
        let mut total_len = 0;
        for slice in slices {
            total_len += slice.len() + 1;
        }

        let size =
            /* argv or envv themselves (aligned) */ ((slices.len() + 1) * size_of::<Slice<u8>>())
            + (size_of::<usize>() /* argc or envc */ + total_len).to_next_multiple_of(size_of::<Slice<u8>>());

        let (start, end) = self.allocate_inner(size, alignment, 0)?;

        let page_table = unsafe { &mut *self.page_table };
        let mut copied = 0;

        macro_rules! copy_bytes {
            ($bytes: expr) => {{
                let data = $bytes;
                crate::memory::copy_to_userspace(page_table, start + copied, data);
                copied += data.len();
            }};
        }

        copy_bytes!(&slices.len().to_ne_bytes());

        let slices_data_area_start = start + copied;
        for slice in slices {
            copy_bytes!(slice);
            copy_bytes!(&[0]);
        }

        copied = copied.to_next_multiple_of(size_of::<Slice<u8>>());
        let pointers_start = start + copied;
        let mut current_slice_data_ptr = slices_data_area_start;

        for slice in slices {
            let raw_slice_fat = unsafe {
                Slice::from_raw_parts(current_slice_data_ptr.into_ptr::<u8>(), slice.len())
            };
            let bytes: [u8; size_of::<Slice<u8>>()] =
                unsafe { core::mem::transmute(raw_slice_fat) };

            copy_bytes!(&bytes);
            current_slice_data_ptr += slice.len() + 1;
        }

        Ok((start, end, pointers_start))
    }
}
