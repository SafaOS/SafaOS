use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::utils::locks::Mutex;
use lazy_static::lazy_static;

use super::{align_down, paging::PAGE_SIZE, PhysAddr, VirtAddr};

/// 1 KiB
pub const SIZE_1K: usize = 1024 * 1;
/// 64 KiB
pub const SIZE_64K: usize = SIZE_1K * 64;
// Pages worth 64 KiB
pub const SIZE_64K_PAGES: usize = SIZE_64K / PAGE_SIZE;

/// A pointer to some data in a physical frame that is mapped to a virtual address in the hddm
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FramePtr<T>(*mut T);
impl<T> FramePtr<T> {
    pub fn phys_addr(&self) -> PhysAddr {
        let virt_addr = VirtAddr::from_ptr(self.as_ptr());
        virt_addr.into_phys()
    }

    pub fn frame(&self) -> Frame {
        Frame(self.phys_addr())
    }

    pub const fn as_ptr(&self) -> *mut T {
        self.0
    }
}

impl<T> Deref for FramePtr<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0 }
    }
}

impl<T> DerefMut for FramePtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.0 }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Frame(PhysAddr);

impl Frame {
    #[inline(always)]
    // returns the frame that contains an address
    pub fn containing_address(address: PhysAddr) -> Self {
        let aligned = align_down(address.into_raw(), PAGE_SIZE);
        Self(PhysAddr::from(aligned))
    }

    #[inline]
    pub fn start_address(&self) -> PhysAddr {
        self.0
    }

    #[inline(always)]
    pub fn virt_addr(&self) -> VirtAddr {
        self.0.into_virt()
    }

    #[inline(always)]
    pub fn phys_addr(&self) -> PhysAddr {
        self.0
    }

    pub fn iter_frames(start: Frame, end: Frame) -> FrameIter {
        debug_assert!(start.start_address() <= end.start_address());
        FrameIter { start, end }
    }

    /// Converts a frame into a pointer to some data in that frame
    /// # Safety
    /// unsafe because the caller must ensure that the frame is valid and points to data containing [`T`]
    pub unsafe fn into_ptr<T>(self) -> FramePtr<T> {
        let addr = self.virt_addr();
        FramePtr(addr.into_ptr::<T>())
    }
}

pub struct FrameIter {
    start: Frame,
    end: Frame,
}

impl Iterator for FrameIter {
    type Item = Frame;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start.start_address() < self.end.start_address() {
            let frame = self.start;

            self.start.0 += PAGE_SIZE;
            Some(frame)
        } else {
            None
        }
    }
}

impl Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Frame")
            .field(&format_args!("{:#x}", self.start_address()))
            .finish()
    }
}

#[derive(Debug)]
struct RegionNode {
    start_address: PhysAddr,
    next: Option<*mut RegionNode>,
    prev: Option<*mut RegionNode>,
}

impl RegionNode {
    pub fn new(start_address: PhysAddr) -> Self {
        Self {
            start_address,
            next: None,
            prev: None,
        }
    }

    /// creates a new region node in the given frame
    /// # Safety
    /// the caller must ensure that the frame is not used anymore
    unsafe fn new_in(frame: Frame) -> *mut Self {
        let frame_addr = frame.virt_addr();
        let region_pointer = frame_addr.into_ptr::<RegionNode>();

        *region_pointer = RegionNode::new(frame.start_address());
        region_pointer
    }

    pub const fn page_num(&self) -> usize {
        self.start_address.into_raw() / PAGE_SIZE
    }
}

#[derive(Debug)]
pub struct RegionListAllocator {
    head: Option<*mut RegionNode>,
    tail: Option<*mut RegionNode>,
    // metadata
    allocations: usize,
    usable_regions: usize,
    unusable_regions: usize,
}

unsafe impl Send for RegionListAllocator {}
unsafe impl Sync for RegionListAllocator {}

impl RegionListAllocator {
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
            allocations: 0,
            usable_regions: 0,
            unusable_regions: 0,
        }
    }

    #[inline(always)]
    fn add_region(&mut self, frame: Frame) {
        unsafe {
            let node = RegionNode::new_in(frame);
            if let Some(head) = self.head.take() {
                (*head).prev = Some(node);
                (*node).next = Some(head);
            }

            if self.tail.is_none() {
                self.tail = Some(node);
            }

            self.head = Some(node);
        }
    }

    /// Loops through the list counting the available frames in the list, more expensive than [`usable_frames`] -  [`mapped_frames`], because these are O(1)
    #[inline(always)]
    fn count_frames_expensive(&self) -> usize {
        let mut current = &raw const self.head;
        let mut n = 0;

        // Safe because the allocator owns all the data in the linked list and it lives as long as the allocator does
        while let Some(curr_ptr) = unsafe { &*current } {
            unsafe {
                n += 1;
                current = &raw const (**curr_ptr).next;
            }
        }

        n
    }

    #[inline(always)]
    pub fn allocate_frame(&mut self) -> Option<Frame> {
        let head = self.head.take()?;

        unsafe {
            self.head = (*head).next.take();
            if let Some(next) = self.head {
                (*next).prev = (*head).prev.take();
            }
            self.allocations += 1;
            Some(Frame::containing_address((*head).start_address))
        }
    }

    /// Allocates `num_pages` contiguous Pages of Physical memory, aligned to `align_pages * PAGE_SIZE`
    /// returns the Frame containing the start Physical address, and the frame containing the end physical address
    /// creating an iter on these frames using [`Frame::iter_frames`] is going to return `num_pages - 1` frames
    #[inline(always)]
    pub fn allocate_contiguous(
        &mut self,
        num_pages: usize,
        align_pages: usize,
    ) -> Option<(Frame, Frame)> {
        if num_pages <= 1 {
            let frame = self.allocate_aligned(align_pages);
            return frame.map(|frame| (frame, frame));
        }

        let alignment = align_pages * PAGE_SIZE;

        let mut curr = &raw mut self.tail;

        let mut found_ptr = None;
        let mut found_end_ptr = None;

        let mut satisfied_frames = 0;
        let mut prev_page_num = None;

        while let Some(curr_ptr) = unsafe { &mut *curr } {
            unsafe {
                if satisfied_frames >= num_pages && found_ptr.is_some() {
                    break;
                }

                let curr_ref = &mut **curr_ptr;

                if prev_page_num.is_some_and(|page_num| curr_ref.page_num() != page_num + 1) {
                    found_ptr = None;
                    satisfied_frames = 0;
                    prev_page_num = None;
                }

                if curr_ref.start_address.is_multiple_of(alignment) && found_ptr.is_none() {
                    found_ptr = Some(curr_ptr as *mut *mut RegionNode);
                    satisfied_frames = 0;
                    prev_page_num = Some(curr_ref.page_num())
                }

                satisfied_frames += 1;
                prev_page_num
                    .as_mut()
                    .map(|page_num| *page_num = curr_ref.page_num());

                found_end_ptr = Some(curr_ptr as *mut *mut RegionNode);

                curr = &mut curr_ref.prev;
            }
        }

        if let Some(start_node_ptr) = found_ptr
            && satisfied_frames == num_pages
        {
            unsafe {
                let start_node_ref = &mut **start_node_ptr;
                let end_node_ref = &mut **found_end_ptr.unwrap();

                let start_frame = Frame::containing_address(start_node_ref.start_address);
                let end_frame = Frame::containing_address(
                    start_node_ref.start_address + (PAGE_SIZE * (num_pages - 1)),
                );

                // replace the pointer to the starting frame with the next frame, and set the .prev of the next frame to the prev of the current one
                // remember the end node is the one closer to the head
                let prev = end_node_ref.prev.take();
                let next = start_node_ref.next.take();

                if let Some(next) = next {
                    (*next).prev = prev;

                    if self.head.is_some_and(|x| x == end_node_ref) {
                        self.head = Some(next);
                    }
                }

                if let Some(prev) = prev {
                    (*prev).next = next;

                    if self.tail.is_some_and(|x| x == start_node_ref) {
                        self.tail = Some(prev);
                    }
                }

                self.allocations += num_pages;
                Some((start_frame, end_frame))
            }
        } else {
            None
        }
    }

    /// Allocates a Frame that's physical address is aligned to `PAGE_SIZE * align_pages`
    #[inline(always)]
    pub fn allocate_aligned(&mut self, align_pages: usize) -> Option<Frame> {
        if align_pages <= 1 {
            return self.allocate_frame();
        }

        let alignment = align_pages * PAGE_SIZE;
        let mut current = &raw mut self.head;

        // Safe because the allocator owns all the data in the linked list and it lives as long as the allocator does
        while let Some(curr_ptr) = unsafe { &mut *current } {
            unsafe {
                let curr = &mut **curr_ptr;

                if curr.start_address.is_multiple_of(alignment) {
                    // TAIL 0x2000 -> <- (0x3000) -> <- 0x1000 HEAD

                    // 0x2000 -> <- (0x3000) x 0x2000 <- 0x1000
                    if let Some(next) = curr.next {
                        (*next).prev = curr.prev;
                        if self.head == *current {
                            self.head = Some(next);
                        }
                    }

                    // 0x2000 -> 0x1000 x (0x3000) x 0x2000 <- 0x1000
                    if let Some(prev) = curr.prev {
                        (*prev).next = curr.next;
                        if self.tail == *current {
                            self.tail = Some(prev);
                        }
                    }

                    // 0x2000 -> <- 0x1000
                    self.allocations += 1;
                    return Some(Frame::containing_address(curr.start_address));
                }

                current = &raw mut curr.next;
            }
        }

        None
    }

    #[inline(always)]
    pub fn deallocate_frame(&mut self, frame: Frame) {
        self.add_region(frame);
        self.allocations -= 1;
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
        let mut allocator = RegionListAllocator::new();

        let mmap = crate::limine::mmap_request();

        let mut usable_regions = 0;
        let mut unusable_regions = 0;

        for entry in mmap.entries() {
            if entry.entry_type == limine::memory_map::EntryType::USABLE {
                let start_addr = PhysAddr::from(entry.base as usize);
                let end_addr = start_addr + (entry.length as usize);

                let frame = Frame::containing_address(start_addr);
                let end_frame = Frame::containing_address(end_addr);

                for frame in Frame::iter_frames(frame, end_frame) {
                    usable_regions += 1;
                    allocator.add_region(frame);
                }
            } else {
                unusable_regions += entry.length as usize / PAGE_SIZE;
            }
        }

        allocator.usable_regions = usable_regions;
        allocator.unusable_regions = unusable_regions;
        allocator
    }
}

lazy_static! {
    pub static ref REGION_ALLOCATOR: Mutex<RegionListAllocator> =
        Mutex::new(RegionListAllocator::create());
}

#[inline(always)]
pub fn allocate_frame() -> Option<Frame> {
    REGION_ALLOCATOR.lock().allocate_frame()
}

#[inline(always)]
pub fn allocate_aligned(align_pages: usize) -> Option<Frame> {
    REGION_ALLOCATOR.lock().allocate_aligned(align_pages)
}

#[inline(always)]
pub fn allocate_contiguous(align_pages: usize, num_pages: usize) -> Option<(Frame, Frame)> {
    REGION_ALLOCATOR
        .lock()
        .allocate_contiguous(num_pages, align_pages)
}

#[inline(always)]
pub fn deallocate_frame(frame: Frame) {
    REGION_ALLOCATOR.lock().deallocate_frame(frame)
}

/// returns the number of mapped frames
#[inline(always)]
pub fn mapped_frames() -> usize {
    REGION_ALLOCATOR.lock().mapped_frames()
}

#[inline(always)]
pub fn usable_frames() -> usize {
    REGION_ALLOCATOR.lock().usable_frames()
}

#[test_case]
fn allocate_many_test() {
    let mut frames = heapless::Vec::<_, 1024>::new();
    for _ in 0..frames.capacity() {
        frames.push(allocate_frame().unwrap()).unwrap();
    }

    for i in 1..frames.capacity() {
        assert_ne!(frames[i - 1].start_address(), frames[i].start_address());
    }

    let last_frame = frames[frames.len() - 1];
    for frame in frames.iter() {
        deallocate_frame(*frame);
    }
    let allocated = allocate_frame().unwrap();
    assert_eq!(allocated, last_frame);

    deallocate_frame(allocated);
}

#[test_case]
fn allocate_aligned_test() {
    let frame = allocate_aligned(SIZE_64K_PAGES).unwrap_or_else(|| {
        panic!(
            "failed to find a Frame with alignment {:#x}",
            SIZE_64K_PAGES * PAGE_SIZE
        )
    });

    assert!(frame.start_address().is_multiple_of(SIZE_64K));
    deallocate_frame(frame);

    let other_frame = allocate_aligned(SIZE_64K_PAGES).unwrap_or_else(|| {
        panic!(
            "failed to reallocate a Frame with alignment {:#x}",
            SIZE_64K_PAGES * PAGE_SIZE
        )
    });

    assert_eq!(other_frame, frame);
    deallocate_frame(other_frame);
    // 3 allocations to be extra sure nothing gets messed up
    let other_frame = allocate_aligned(SIZE_64K_PAGES).unwrap_or_else(|| {
        panic!(
            "failed to reallocate a Frame with alignment {:#x}",
            SIZE_64K_PAGES * PAGE_SIZE
        )
    });

    assert_eq!(other_frame, frame);
    deallocate_frame(other_frame);
}

fn allocate_contiguous_test_inner<const N: usize>(align_pages: usize) -> heapless::Vec<Frame, N> {
    let used_before = mapped_frames();
    let mut results = heapless::Vec::new();
    let (start, end) = allocate_contiguous(align_pages, N).expect("Failed to allocate contiguous");

    assert!(start
        .start_address()
        .is_multiple_of(align_pages * PAGE_SIZE));
    assert_eq!(used_before + N, mapped_frames());

    let iter = Frame::iter_frames(
        start,
        Frame::containing_address(end.start_address() + PAGE_SIZE),
    );

    for frame in iter {
        deallocate_frame(frame);
        results.push(frame).unwrap();
    }

    assert_eq!(used_before, mapped_frames());
    assert_eq!(results.len(), N);
    results
}

#[test_case]
fn allocate_contiguous_test() {
    let used_before = mapped_frames();

    let results = allocate_contiguous_test_inner::<0x10>(SIZE_64K_PAGES);

    let other_results = allocate_contiguous_test_inner::<0x30>(SIZE_64K_PAGES);
    for res in results {
        // as they were freed, they should be pushed to the top of the list. and allocate_contiguous starts from the tail of the list
        assert!(!other_results.contains(&res));
    }

    assert_eq!(used_before, mapped_frames());
}

// Thanks to the fact tests are executed alphabetically this test is executed last, maybe this shouldn't be relied upon....
// makes sure all the previous tests didn't mess up something with the linked list
#[test_case]
fn frame_count_verification_test() {
    let actual_frame_count = REGION_ALLOCATOR.lock().count_frames_expensive();
    assert_eq!(usable_frames() - mapped_frames(), actual_frame_count);
}
