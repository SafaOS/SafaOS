use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::utils::locks::Mutex;
use lazy_static::lazy_static;

use super::{align_down, paging::PAGE_SIZE, PhysAddr, VirtAddr};

/// A pointer to some data in a physical frame that is mapped to a virtual address in the hddm
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FramePtr<T>(*mut T);
impl<T> FramePtr<T> {
    pub fn phys_addr(&self) -> PhysAddr {
        let virt_addr = VirtAddr::from_ptr(self.0);
        virt_addr.into_phys()
    }

    pub fn frame(&self) -> Frame {
        Frame(self.phys_addr())
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
struct RegionNode<'a> {
    start_address: PhysAddr,
    next: Option<&'a mut RegionNode<'a>>,
}

impl<'a> RegionNode<'a> {
    pub fn new(start_address: PhysAddr) -> Self {
        Self {
            start_address,
            next: None,
        }
    }

    /// creates a new region node in the given frame
    /// # Safety
    /// the caller must ensure that the frame is not used anymore
    unsafe fn new_in(frame: Frame) -> &'a mut Self {
        let frame_addr = frame.virt_addr();
        let region_pointer = frame_addr.into_ptr::<RegionNode>();

        *region_pointer = RegionNode::new(frame.start_address());
        unsafe { &mut *region_pointer }
    }

    #[inline(always)]
    pub fn insert(&mut self, next: &'a mut Self) {
        self.next = Some(next);
    }
}

#[derive(Debug)]
pub struct RegionListAllocator<'a> {
    head: Option<&'a mut RegionNode<'a>>,
    // metadata
    allocations: usize,
    usable_regions: usize,
    unusable_regions: usize,
}

impl<'a> RegionListAllocator<'a> {
    pub fn new() -> Self {
        Self {
            head: None,
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
                node.insert(head);
            }

            self.head = Some(node);
        }
    }

    #[inline(always)]
    pub fn allocate_frame(&mut self) -> Option<Frame> {
        let head = self.head.take()?;

        self.head = head.next.take();
        self.allocations += 1;
        Some(Frame::containing_address(head.start_address))
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
    pub fn create() -> RegionListAllocator<'static> {
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
    pub static ref REGION_ALLOCATOR: Mutex<RegionListAllocator<'static>> =
        Mutex::new(RegionListAllocator::create());
}
#[inline(always)]
pub fn allocate_frame() -> Option<Frame> {
    REGION_ALLOCATOR.lock().allocate_frame()
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
fn frame_allocator_test() {
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
