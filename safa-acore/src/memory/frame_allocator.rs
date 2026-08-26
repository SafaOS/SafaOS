use core::fmt::Debug;
use core::ptr::NonNull;

use crate::bootloader::MemoryType;
use crate::memory::phys_to_virt;
use crate::misc::Frame;
use crate::sync::SpinLockIrq;
use crate::{bootloader, logging};

use crate::misc::{PAGE_SIZE, PhysAddr};
#[cfg(test)]
mod test;

#[derive(Debug)]
struct RegionNode {
    base: PhysAddr,
    next: Option<NonNull<RegionNode>>,
    prev: Option<NonNull<RegionNode>>,
}

impl RegionNode {
    pub fn new(base: PhysAddr) -> Self {
        Self {
            base,
            next: None,
            prev: None,
        }
    }

    /// creates a new region node in the given frame
    /// # Safety
    /// the caller must ensure that the frame is not used anymore
    unsafe fn new_in(frame: Frame) -> NonNull<Self> {
        unsafe {
            let frame_addr = frame.addr();
            let region_pointer = phys_to_virt(frame_addr).into_ptr::<RegionNode>();

            *region_pointer = RegionNode::new(frame.addr());
            NonNull::new(region_pointer).expect("New node is null")
        }
    }

    /// Given a `frame` converts it to a region.
    unsafe fn from_frame(frame: Frame) -> NonNull<Self> {
        let frame_addr = frame.addr();
        let region_pointer = phys_to_virt(frame_addr).into_ptr::<RegionNode>();

        NonNull::new(region_pointer).expect("New node is null")
    }

    /// Removes regions going from `head` to `tail` from free list.
    #[inline(always)]
    unsafe fn cut_regions(
        mut head: NonNull<Self>,
        mut tail: NonNull<Self>,
        list_head: &mut Option<NonNull<Self>>,
        list_tail: &mut Option<NonNull<Self>>,
    ) {
        unsafe {
            if let Some(mut prev) = head.as_mut().prev {
                prev.as_mut().next = tail.as_mut().next;
            }

            if let Some(mut next) = tail.as_mut().next {
                next.as_mut().prev = head.as_mut().prev;
            }

            if list_head.is_none_or(|l| l == head) {
                *list_head = head.as_ref().next;
            }

            if list_tail.is_none_or(|l| l == tail) {
                *list_tail = head.as_ref().prev;
            }
        }
    }

    /// Removes a single region `region` from free list.
    unsafe fn cut_region(
        region: NonNull<Self>,
        list_head: &mut Option<NonNull<Self>>,
        list_tail: &mut Option<NonNull<Self>>,
    ) {
        unsafe { Self::cut_regions(region, region, list_head, list_tail) }
    }
}

#[derive(Debug)]
pub struct RegionListAllocator {
    head: Option<NonNull<RegionNode>>,
    tail: Option<NonNull<RegionNode>>,
    bitmap: Option<NonNull<[u8]>>,
    // metadata
    allocations: usize,
    usable_regions: usize,
    unusable_regions: usize,
}

unsafe impl Send for RegionListAllocator {}
unsafe impl Sync for RegionListAllocator {}

impl RegionListAllocator {
    /// Sets a given bit to a given value.
    fn bitmap_set_bit(&mut self, bitnum: usize, value: bool) {
        let idx = bitnum / 8;
        let bit_idx = bitnum % 8;

        // Ignore everything out of scope of study.
        if let Some(ref mut bitmap) = self.bitmap
            && idx < bitmap.len()
        {
            unsafe {
                let byte = &mut bitmap.as_mut()[idx];

                if value {
                    *byte |= 1 << (7 - bit_idx);
                } else {
                    *byte &= !(1 << (7 - bit_idx));
                }
            }
        }
    }

    /// Finds `bits_count` contiguous bits aligend to `bits_align` that are set false and returns their start bit number.
    fn bitmap_lookup_free(&self, bits_align: usize, bits_count: usize) -> Option<usize> {
        let bitmap = self.bitmap.as_ref()?;
        let bitmap = unsafe { bitmap.as_ref() };
        let max_bits = bitmap.len() * 8;

        if bits_count == 0 {
            return None;
        }

        let align = bits_align.max(1);

        #[inline(always)]
        fn bit_is_set(bitmap: &[u8], bitnum: usize) -> bool {
            let byte = bitmap[bitnum / 8];
            let bit = 7 - (bitnum % 8);
            (byte >> bit) & 1 != 0
        }

        let mut start = 0usize;
        // Do a bit level search not a byte one.
        while start.checked_add(bits_count)? <= max_bits {
            // start = 0
            let mut i = start;
            let end = start + bits_count;

            let mut found_used = None;

            while i < end {
                if bit_is_set(bitmap, i) {
                    found_used = Some(i);
                    break;
                }
                i += 1;
            }

            match found_used {
                None => return Some(start),
                Some(used_bit) => {
                    start = (used_bit + 1).next_multiple_of(align);
                }
            }
        }

        None
    }

    #[inline]
    /// Sets `bits_count` bits starting at `start_bit` to `value`.
    fn bitmap_set_bits(&mut self, start_bit: usize, bits_count: usize, value: bool) {
        if let Some(bitmap) = self.bitmap.as_mut() {
            _ = Self::bitmap_set_bits_inner(
                unsafe { bitmap.as_mut() },
                start_bit,
                bits_count,
                value,
            );
        }
    }
    #[inline]
    fn bitmap_set_bits_inner(
        bitmap: &mut [u8],
        start_bit: usize,
        bits_count: usize,
        value: bool,
    ) -> Option<((usize, usize), (usize, usize))> {
        let max_bits = bitmap.len() * 8;

        if start_bit >= max_bits {
            return None;
        }

        let end_bit = (start_bit.saturating_add(bits_count)).min(max_bits);

        for i in start_bit.div_ceil(8)..(end_bit / 8) {
            if value {
                bitmap[i] = 0xff;
            } else {
                bitmap[i] = 0x00;
            }
        }

        let start_off = start_bit % 8;
        let end_off = end_bit % 8;

        let start_idx = start_bit / 8;
        let end_idx = end_bit / 8;

        if start_idx != end_idx {
            if start_off != 0 {
                let start_mask = 0xff >> start_off;

                if value {
                    bitmap[start_idx] |= start_mask;
                } else {
                    bitmap[start_idx] &= !start_mask;
                }
            }

            if end_off != 0 {
                let end_mask = 0xff << (8 - end_off);

                if value {
                    bitmap[end_idx] |= end_mask;
                } else {
                    bitmap[end_idx] &= !end_mask;
                }
            }
        } else if start_off != end_off {
            let mask = (0xffu8 >> start_off) & (0xffu8 << (8 - end_off));

            if value {
                bitmap[start_idx] |= mask;
            } else {
                bitmap[start_idx] &= !mask;
            }
        } else {
            // range == 0
        }

        Some(((start_bit, end_bit), (start_idx, end_idx)))
    }

    fn mark_bitmap_used(phys_start: PhysAddr, phys_end: PhysAddr, bitmap: &mut [u8]) {
        assert!(
            phys_end >= phys_start,
            "end address must be bigger than or equal to start address"
        );

        let start_bit = phys_start.page_num();
        let end_bit = phys_end.page_num();

        let Some(((start_bit, end_bit), (start_idx, end_idx))) =
            Self::bitmap_set_bits_inner(bitmap, start_bit, end_bit - start_bit + 1, true)
        else {
            logging::trace!(
                RegionListAllocator,
                "range {phys_start:?}..{phys_end:?} start={start_bit}bit end={end_bit}bit is out of bitmap"
            );
            return;
        };

        logging::trace!(
            RegionListAllocator,
            "bitmap marked range: {phys_start:?}..{phys_end:?} start={start_bit:#x}bit end={end_bit:#x}bit, start idx={:#x} end idx={:#x}, start off={start_off} end off={end_off}, bitmap set: {:x?}",
            start_idx,
            end_idx,
            &bitmap[start_idx..=end_idx.min(bitmap.len() - 1)],
            start_off = start_bit % 8,
            end_off = end_bit % 8,
        );
    }

    #[inline(always)]
    fn add_region(
        head: &mut Option<NonNull<RegionNode>>,
        tail: &mut Option<NonNull<RegionNode>>,
        frame: Frame,
    ) {
        unsafe {
            let mut node = RegionNode::new_in(frame);

            if let Some(mut head) = head.take() {
                (head.as_mut()).prev = Some(node);
                (node.as_mut()).next = Some(head);
            }

            if tail.is_none() {
                *tail = Some(node);
            }

            *head = Some(node);
        }
    }

    #[cfg(test)]
    /// Loops through the list counting the available frames in the list, more expensive than [`usable_frames`] -  [`mapped_frames`], because these are O(1)
    #[inline(always)]
    fn count_frames_expensive(&self) -> usize {
        let mut current = self.head;
        let mut n = 0;

        // Safe because the allocator owns all the data in the linked list and it lives as long as the allocator does
        while let Some(curr_ptr) = current {
            n += 1;
            unsafe {
                current = curr_ptr.as_ref().next;
            }
        }

        n
    }

    #[inline(always)]
    /// Allocates a single frame using the free-list.
    fn allocate_frame(&mut self) -> Option<Frame> {
        let mut head = self.head.take()?;

        unsafe {
            self.head = head.as_mut().next.take();

            if let Some(mut next) = self.head {
                next.as_mut().prev = head.as_mut().prev.take();
            } else {
                self.tail = None;
            }

            self.allocations += 1;

            let base = head.as_ref().base;

            self.bitmap_set_bit(base.page_num(), true);
            Some(Frame::containing(base))
        }
    }

    #[inline(always)]
    /// Deallocates a single frame using the free list.
    fn deallocate_frame(&mut self, frame: Frame) {
        Self::add_region(&mut self.head, &mut self.tail, frame);
        self.allocations -= 1;
        self.bitmap_set_bit(frame.addr().page_num(), false);
    }

    #[inline(always)]
    /// Allocates `count` frames with `align`-frames alignment.
    fn allocate_frames(&mut self, align: usize, count: usize) -> Option<Frame> {
        if align == 1 && count == 1 {
            return self.allocate_frame();
        }
        if count == 0 {
            return None;
        }

        let pagenum = self.bitmap_lookup_free(align, count)?;
        let addr = PhysAddr::new(pagenum * PAGE_SIZE);

        let base_f = Frame::containing(addr);
        let end_f = Frame::containing(addr + (count * PAGE_SIZE));

        for frame in Frame::iter_frames(base_f, end_f) {
            let region = unsafe { RegionNode::from_frame(frame) };

            unsafe {
                RegionNode::cut_region(region, &mut self.head, &mut self.tail);
            }
        }
        self.bitmap_set_bits(pagenum, count, true);
        self.allocations += count;
        Some(base_f)
    }

    #[inline(always)]
    /// Deallocates `count` contiugous frames starting at `base`.
    fn deallocate_frames(&mut self, base: Frame, count: usize) {
        if count == 1 {
            return self.deallocate_frame(base);
        }
        assert_ne!(count, 0, "Cannot deallocate 0 frames");

        let pagenum = base.addr().page_num();
        self.bitmap_set_bits(pagenum, count, false);

        for frame in Frame::iter_frames(base, Frame::containing(base.addr() + (count * PAGE_SIZE)))
        {
            Self::add_region(&mut self.head, &mut self.tail, frame);
        }

        self.allocations -= count;
    }

    /// returns the number of frames mapped
    pub fn mapped_frames(&self) -> usize {
        self.allocations
    }
    /// returns the number of usable frames
    pub fn usable_frames(&self) -> usize {
        self.usable_regions
    }

    /// creates a new static RegionAllocator based on the memory map provided by the bootloader
    pub fn create() -> RegionListAllocator {
        let mut usable_regions = 0;
        let mut unusable_regions = 0;
        let mut max_usable_addr = PhysAddr::null();

        let mut head = None;
        let mut tail = None;

        for entry in bootloader::memory_map() {
            let end_addr = entry.base + entry.size;

            logging::trace!(
                RegionListAllocator,
                "Region at {:?}..{:?}: {:?}",
                entry.base,
                end_addr,
                entry.kind
            );

            if entry.kind == MemoryType::Usable {
                for frame in Frame::iter_addresses(entry.base, end_addr) {
                    usable_regions += 1;
                    Self::add_region(&mut head, &mut tail, frame);
                }

                let end_addr = entry.base + entry.size;
                if end_addr > max_usable_addr {
                    max_usable_addr = end_addr;
                }
            } else {
                unusable_regions += entry.size as usize / PAGE_SIZE;
            }
        }

        // Allocate the bitmap for the region list allocator.
        let bitmap_bits: usize = max_usable_addr.raw() / PAGE_SIZE;
        let bitmap_bytes: usize = bitmap_bits.div_ceil(8);

        let bitmap_area_pages = bitmap_bytes.div_ceil(PAGE_SIZE);

        logging::debug!(
            RegionListAllocator,
            "bitmap_area_pages: {}, total memory needed: {}.{}KiB, tail at: {:#x?}, head at: {:#x?}",
            bitmap_area_pages,
            bitmap_bytes / 1024,
            bitmap_bytes % 1024,
            tail,
            head
        );

        // Search from tail for N contiugous pages to allocate for the bitmap.
        let mut pages_left = bitmap_area_pages;

        let mut bitmap_base = None;
        let mut bitmap_tail = None;
        let mut search_current = tail;

        while let Some(frame) = search_current {
            let frame_base = unsafe { frame.as_ref().base };
            if pages_left == bitmap_area_pages {
                bitmap_base = Some(frame);
            }
            bitmap_tail = Some(frame);

            search_current = unsafe { frame.as_ref().prev };
            pages_left -= 1;

            if pages_left == 0 {
                break;
            }

            // Address is not contiguous with the previous frame, reset pages_left.
            if let Some(prev) = search_current
                && frame_base != unsafe { prev.as_ref().base - PAGE_SIZE }
            {
                logging::trace!(
                    RegionListAllocator,
                    "resetting bitmap search, pages_left: {pages_left} base was: {:?}",
                    frame_base
                );
                pages_left = bitmap_area_pages;
            }
        }

        let bitmap: Option<NonNull<[u8]>>;
        if let Some(mut bitmap_base) = bitmap_base {
            let mut bitmap_tail = bitmap_tail.unwrap();

            let bitmap_phys = unsafe { bitmap_base.as_ref().base };
            let bitmap_end_phys = unsafe { bitmap_tail.as_ref().base };

            logging::trace!(
                RegionListAllocator,
                "bitmap allocated at {:p}=>{:?}",
                bitmap_base,
                bitmap_phys
            );

            let mut bitmap_slice = NonNull::slice_from_raw_parts(
                NonNull::new(phys_to_virt(bitmap_phys).into_ptr::<u8>()).unwrap(),
                bitmap_bytes,
            );

            // remove bitmap base and tail from free list
            // and mark bitmap as unusable within itself
            unsafe {
                // Safety: tail is closer to head.
                RegionNode::cut_regions(bitmap_tail, bitmap_base, &mut head, &mut tail);
            }

            Self::mark_bitmap_used(bitmap_phys, bitmap_end_phys, unsafe {
                bitmap_slice.as_mut()
            });
            for entry in bootloader::memory_map() {
                if entry.kind != MemoryType::Usable {
                    Self::mark_bitmap_used(
                        entry.base,
                        entry.base + entry.size.saturating_sub(PAGE_SIZE),
                        unsafe { bitmap_slice.as_mut() },
                    );
                }
            }
            bitmap = Some(bitmap_slice);
        } else {
            logging::error!(
                RegionListAllocator,
                "failed to allocate a {}KiB bitmap for contiguous memory allocation, system drivers may fail",
                bitmap_bytes / 1024
            );

            bitmap = None;
        }

        RegionListAllocator {
            head,
            tail,
            bitmap,
            allocations: 0,
            usable_regions,
            unusable_regions,
        }
    }
}

static REGION_ALLOCATOR: SpinLockIrq<RegionListAllocator> = SpinLockIrq::new(RegionListAllocator {
    head: None,
    tail: None,
    bitmap: None,
    allocations: 0,
    usable_regions: 0,
    unusable_regions: 0,
});

#[inline]
pub fn init() {
    REGION_ALLOCATOR.lock_no_irq(|alloc| **alloc = RegionListAllocator::create())
}

/// Allocates `count` contiugous frames with `align`-frames alignment.
pub fn allocate_frames(align: usize, count: usize) -> Option<Frame> {
    REGION_ALLOCATOR.lock_no_irq(|alloc| alloc.allocate_frames(align, count))
}

#[inline(always)]
/// Allocates a single frame.
pub fn allocate_frame() -> Option<Frame> {
    allocate_frames(1, 1)
}

/// Deallocates `count` contiugous frames starting at `base`.
///
/// Safety: each frame starting at `base` to `base`+count must no longer be used and allocated using this allocator.
pub unsafe fn deallocate_frames(base: Frame, count: usize) {
    REGION_ALLOCATOR.lock_no_irq(|alloc| alloc.deallocate_frames(base, count))
}

#[inline(always)]
/// Deallocates a single frame `frame`.
///
/// Safety: `frame` must no longer be used and allocated using this allocator.
pub unsafe fn deallocate_frame(frame: Frame) {
    unsafe { deallocate_frames(frame, 1) }
}
