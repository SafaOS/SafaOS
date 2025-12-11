use core::{hint::unreachable_unchecked, ptr::NonNull};

use crate::{
    VirtAddr,
    arch::paging::PageTable,
    memory::{
        AlignToPage,
        frame_allocator::{self, Frame, FramePtr},
        paging::{EntryFlags, MapToError, PAGE_SIZE, Page},
    },
};

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct VMMMFlags: u8 {
        /// By default the region is read-only.
        const WRITEABLE = 1 << 0;
        /// By default the region is not executable.
        const EXECUTABLE = 1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const UNCACHABLE = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VMMAllocMode<I: Iterator<Item = Frame> + ExactSizeIterator = core::iter::Empty<Frame>> {
    /// Normal allocation mode
    ///
    /// The region is allocated immediately and mapped to the virtual address space, you don't control what it is mapped to.
    Normal,
    /// Lazy allocation mode
    ///
    /// unlike [`VMMAllocMode::Normal`], the region is allocated as needed on first access.
    Lazy,
    /// Direct mapping mode for DMA or mapping resources.
    ///
    /// You provide the physical addresses that this region is mapped to.
    ///
    /// When using this mode, the provided frames total size must be equal to or more than the requested allocation size.
    DirectMapping(I),
}

#[derive(Debug)]
enum ObjectsEntry {
    Empty,
    Taken(VMMObject),
}

#[derive(Debug)]
struct VMMObjectsPage {
    /// Bitmap of taken allocated objects in this page.
    bitmap: u128,
    /// Unordered set of objects in this page.
    ///
    /// This allows for fast insertion, and removal because we don't need to maintain order,
    /// instead the order is maintained by the objects themselves in a linked list.
    objects: [ObjectsEntry; 101],
    next: Option<FramePtr<VMMObjectsPage>>,
}

impl VMMObjectsPage {
    const fn new() -> Self {
        Self {
            bitmap: 0,
            objects: [const { ObjectsEntry::Empty }; 101],
            next: None,
        }
    }

    fn allocate() -> Result<FramePtr<Self>, ()> {
        frame_allocator::allocate_frame()
            .map(|frame| {
                let mut ptr = unsafe { frame.into_ptr::<Self>() };
                *ptr = Self::new();
                ptr
            })
            .ok_or(())
    }

    fn index_of(&self, obj: *const VMMObject) -> usize {
        unsafe { (obj).offset_from(self.objects.as_ptr() as *const VMMObject) as usize }
    }

    fn remove_object(&mut self, obj: *const VMMObject) -> VMMObject {
        let index = self.index_of(obj);
        match core::mem::replace(&mut self.objects[index], ObjectsEntry::Empty) {
            ObjectsEntry::Taken(obj) => {
                self.bitmap &= !(1 << index);
                obj
            }
            ObjectsEntry::Empty => panic!("Attempted to remove an object that was not present"),
        }
    }

    fn add_object(&mut self, obj: VMMObject) -> Result<NonNull<VMMObject>, ()> {
        // find a free index in the bitmap
        let index = self.bitmap.trailing_ones() as usize;
        if index >= self.objects.len() {
            // Try to push to sister objects page
            return if let Some(ref mut next) = self.next {
                next.add_object(obj)
            } else {
                let next = Self::allocate()?;
                self.next.insert(next).add_object(obj)
            };
        }

        self.bitmap |= 1 << index;
        self.objects[index] = ObjectsEntry::Taken(obj);
        match &self.objects[index] {
            ObjectsEntry::Taken(obj) => Ok(NonNull::from_ref(obj)),
            ObjectsEntry::Empty => unsafe { unreachable_unchecked() },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ObjectState {
    Free,
    Allocated(VMMMFlags),
    DMAAllocated(VMMMFlags),
    LazyAllocated(VMMMFlags),
}

#[derive(Debug, Clone, Copy)]
pub struct VMMObject {
    addr: VirtAddr,
    state: ObjectState,
    /// Size of the region in bytes.
    ///
    /// although we always want to use pages.
    size: usize,

    next: Option<NonNull<VMMObject>>,
    prev: Option<NonNull<VMMObject>>,
}

impl VMMObject {
    pub fn prev_mut(&mut self) -> Option<&mut VMMObject> {
        self.prev.map(|mut ptr| unsafe { ptr.as_mut() })
    }

    pub fn prev(&self) -> Option<&VMMObject> {
        self.prev.map(|ptr| unsafe { ptr.as_ref() })
    }

    pub fn next_mut(&mut self) -> Option<&mut VMMObject> {
        self.next.map(|mut ptr| unsafe { ptr.as_mut() })
    }

    pub fn next(&self) -> Option<&VMMObject> {
        self.next.map(|ptr| unsafe { ptr.as_ref() })
    }

    #[inline(always)]
    pub const fn as_non_null(&self) -> NonNull<Self> {
        NonNull::from_ref(self)
    }

    pub fn insert_next(&mut self, mut new_next: NonNull<VMMObject>) {
        let new_next_ref = unsafe { new_next.as_mut() };
        let old_next = self.next.replace(new_next);

        new_next_ref.next = old_next;
        if let Some(mut old_next) = old_next {
            unsafe { old_next.as_mut().prev = Some(new_next) };
        }

        new_next_ref.prev = Some(NonNull::from_mut(self));
    }

    pub fn insert_prev(&mut self, mut new_prev: NonNull<VMMObject>) {
        let new_prev_ref = unsafe { new_prev.as_mut() };
        let old_prev = self.prev.replace(new_prev);

        new_prev_ref.prev = old_prev;
        if let Some(mut old_prev) = old_prev {
            unsafe { old_prev.as_mut().next = Some(new_prev) };
        }

        new_prev_ref.next = Some(self.as_non_null());
    }

    /// Attempts to absorb the next object into this one.
    ///
    /// returns a ptr to the new object if successful, otherwise and if there is no next None.
    pub fn try_absorb_right(&mut self) -> (Option<NonNull<Self>>, bool) {
        if let Some(ref right_ptr) = self.next {
            let right = {
                if unsafe { right_ptr.as_ref().allocated() } {
                    return (None, false);
                }
                unsafe { Self::remove_in_place(*right_ptr) }
            };

            self.size += right.size;
            self.next = right.next;

            if let Some(mut next) = self.next {
                unsafe { next.as_mut().prev = Some(self.as_non_null()) };
            }

            (self.next, true)
        } else {
            (None, false)
        }
    }

    /// Attempts to absorb the previous object into this one.
    ///
    /// returns a ptr to the new object if successful, otherwise and if there is no previous None.
    fn try_absorb_left(&mut self) -> (Option<NonNull<Self>>, bool) {
        if let Some(ref left_ptr) = self.prev {
            let left = {
                if unsafe { left_ptr.as_ref().allocated() } {
                    return (None, false);
                }
                unsafe { Self::remove_in_place(*left_ptr) }
            };

            self.size += left.size;
            self.addr = left.addr;
            self.prev = left.prev;

            if let Some(mut prev) = self.prev {
                unsafe { prev.as_mut().next = Some(self.as_non_null()) };
            }
            (self.prev, true)
        } else {
            (None, false)
        }
    }

    #[inline(always)]
    fn objects_page(&self) -> NonNull<VMMObjectsPage> {
        unsafe {
            NonNull::new_unchecked(
                ((self as *const VMMObject) as usize).to_previous_page() as *mut VMMObjectsPage
            )
        }
    }

    fn objects_page_mut(&mut self) -> &mut VMMObjectsPage {
        unsafe { self.objects_page().as_mut() }
    }

    /// Deallocates the given `VMMObject`.
    ///
    /// Safety: `ptr` must be a valid pointer to a `VMMObject`, it becomes invalid after this call.
    pub unsafe fn remove_in_place(mut ptr: NonNull<Self>) -> VMMObject {
        let this = unsafe {
            ptr.as_mut()
                .objects_page()
                .as_mut()
                .remove_object(ptr.as_ptr())
        };
        this
    }

    #[inline]
    pub const fn allocated(&self) -> bool {
        matches!(
            self.state,
            ObjectState::Allocated(_)
                | ObjectState::DMAAllocated(_)
                | ObjectState::LazyAllocated(_)
        )
    }

    const fn new_free(addr: VirtAddr, size: usize) -> Self {
        Self {
            addr,
            state: ObjectState::Free,
            size,
            next: None,
            prev: None,
        }
    }

    #[inline(always)]
    pub const fn region_end(&self) -> VirtAddr {
        self.addr + self.size
    }

    /// Spilt the region so that it starts at self.region_start + `offset` and has `size` bytes.
    ///
    /// Returns a tuple of the newly created left and right objects, if they exist.
    fn split_at(
        &mut self,
        offset: usize,
        size: usize,
    ) -> Result<(Option<NonNull<Self>>, Option<NonNull<Self>>), ()> {
        if offset == 0 {
            let results = self.split_to_fit(size)?;
            return Ok((None, results));
        }

        // Spilt into 3 regions
        // - left
        // - this
        // - right
        let left_obj = VMMObject::new_free(self.addr, offset);

        let right_addr = self.addr + offset + size;
        let right_size = self.size - size - offset;
        let right_obj = (right_size != 0).then(|| VMMObject::new_free(right_addr, right_size));

        // Allocate left and right
        let left_ptr = self.objects_page_mut().add_object(left_obj)?;
        let right_ptr = match right_obj.map(|obj| self.objects_page_mut().add_object(obj)) {
            None => None,
            Some(Ok(ptr)) => Some(ptr),
            Some(Err(_)) => {
                unsafe { Self::remove_in_place(left_ptr) };
                return Err(());
            }
        };

        self.addr += offset;
        self.size = size;
        self.insert_prev(left_ptr);
        if let Some(right) = right_ptr {
            self.insert_next(right);
        }
        Ok((Some(left_ptr), right_ptr))
    }

    /// Splits the region so it has `fit` size of bytes, the rest is put into a new object, if that object was created and inserted a pointer to it will be returned.
    pub fn split_to_fit(&mut self, fit: usize) -> Result<Option<NonNull<Self>>, ()> {
        // Split the region if it's larger than the requested size
        if self.size > fit {
            let rest_size = self.size - fit;
            let rest_addr = self.addr + fit;

            let rest_meta = VMMObject::new_free(rest_addr, rest_size);

            self.size -= rest_size;
            let rest_ptr = self.objects_page_mut().add_object(rest_meta)?;
            self.insert_next(rest_ptr);
            Ok(Some(rest_ptr))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMMAllocError {
    OutOfMemory,
    OutOfRange,
    AlreadyUsed,
    InvalidSize,
}

impl From<MapToError> for VMMAllocError {
    fn from(value: MapToError) -> Self {
        match value {
            MapToError::FrameAllocationFailed => VMMAllocError::OutOfMemory,
            MapToError::AlreadyMapped => {
                unreachable!("VMM shouldn't try to map an already mapped region")
            }
        }
    }
}

#[derive(Debug)]
pub struct VirtualMemoryManager {
    page_table: FramePtr<PageTable>,
    start_addr: VirtAddr,
    size: usize,
    root_objects_set: FramePtr<VMMObjectsPage>,
    head: NonNull<VMMObject>,
    tail: NonNull<VMMObject>,
    len: usize,
    next_vmm: Option<&'static VirtualMemoryManager>,
}

impl Drop for VirtualMemoryManager {
    fn drop(&mut self) {
        // Deallocate all the [`VMMObjectsPage`]s
        let mut curr = Some(self.root_objects_set);
        while let Some(mut page) = curr {
            let next = page.next.take();
            // TODO: unmap memory?
            frame_allocator::deallocate_frame(page.frame());
            curr = next;
        }
    }
}

impl VirtualMemoryManager {
    pub fn new(start_addr: VirtAddr, size: usize, page_table: FramePtr<PageTable>) -> Self {
        let mut objects =
            VMMObjectsPage::allocate().expect("Failed to allocate memory for storing VMM objects");
        let object = VMMObject::new_free(start_addr, size);
        let object_ptr = objects.add_object(object).expect("Failed to insert object");

        VirtualMemoryManager {
            start_addr,
            size,
            root_objects_set: objects,
            head: object_ptr,
            tail: object_ptr,
            page_table,
            len: 1,
            next_vmm: None,
        }
    }

    fn tail_mut(&mut self) -> &mut VMMObject {
        unsafe { self.tail.as_mut() }
    }

    fn head_mut(&mut self) -> &mut VMMObject {
        unsafe { self.head.as_mut() }
    }

    fn head(&self) -> &VMMObject {
        unsafe { self.head.as_ref() }
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Lookup a VMM Object that contains the given address.
    pub fn lookup_addr(&self, addr: VirtAddr) -> Option<&VMMObject> {
        if self.start_addr > addr || self.start_addr + self.size <= addr {
            return None;
        }

        let mut current = Some(self.head());
        while let Some(obj) = current {
            if obj.addr <= addr && obj.region_end() > addr {
                return Some(obj);
            }
            current = obj.next();
        }

        unreachable!("Should find the address because it is within the VMM range")
    }

    fn deallocate_at(&mut self, addr: VirtAddr) -> Option<(ObjectState, usize)> {
        if self.start_addr > addr || self.start_addr + self.size <= addr {
            return None;
        }

        let mut current = Some(self.head_mut());
        while let Some(obj) = current.take() {
            if obj.addr == addr {
                assert!(
                    obj.allocated(),
                    "Attempt to free unallocated memory, this is a bug"
                );

                let old_state = core::mem::replace(&mut obj.state, ObjectState::Free);
                let size = obj.size;

                let (new_right, right_removed) = obj.try_absorb_right();
                let (new_left, left_removed) = obj.try_absorb_left();
                let obj_ptr = obj.as_non_null();

                if right_removed {
                    self.len -= 1;
                    if new_right.is_none() {
                        // We absorbed the tail
                        self.tail = obj_ptr;
                    }
                }
                if left_removed {
                    self.len -= 1;
                    if new_left.is_none() {
                        // We absorbed the head
                        self.head = obj_ptr;
                    }
                }

                return Some((old_state, size));
            }

            assert!(
                !(obj.addr > addr && obj.region_end() > addr),
                "Attempt to free memory inside of the object's range and not at the start"
            );

            current = obj.next_mut();
        }

        unreachable!("Should find the address because it is within the VMM range")
    }

    fn allocate_at(
        &mut self,
        start_addr: VirtAddr,
        size: usize,
        obj_state: ObjectState,
    ) -> Result<(), VMMAllocError> {
        let end_addr = start_addr + size;
        if self.start_addr > start_addr || end_addr >= self.start_addr + self.size {
            return Err(VMMAllocError::OutOfRange);
        }

        let mut current = Some(self.head_mut());
        let mut is_head = true;

        while let Some(curr_obj) = current {
            let is_tail = curr_obj.next.is_none();

            if curr_obj.addr > start_addr {
                let prev = curr_obj.prev();

                crate::warn!(
                    VirtualMemoryManager,
                    "Request allocation area fragmented, detected on: addr={:?}, size={:#x}, state={:?}",
                    curr_obj.addr,
                    curr_obj.size,
                    curr_obj.state,
                );
                if let Some(prev) = prev {
                    crate::serial!(
                        "prev addr={:?}, prev size={:#x}, prev state={:?}\n",
                        prev.addr,
                        prev.size,
                        prev.state
                    );
                }
                return Err(VMMAllocError::AlreadyUsed);
            }

            if curr_obj.addr <= start_addr && curr_obj.region_end() >= end_addr {
                if curr_obj.allocated() {
                    return Err(VMMAllocError::AlreadyUsed);
                }

                let offset = start_addr - curr_obj.addr;
                let (new_head, new_tail) = curr_obj
                    .split_at(offset, size)
                    .map_err(|()| VMMAllocError::OutOfMemory)?;
                curr_obj.state = obj_state;

                if let Some(new_head) = new_head {
                    self.len += 1;
                    if is_head {
                        self.head = new_head;
                    }
                }
                if let Some(new_tail) = new_tail {
                    self.len += 1;
                    if is_tail {
                        self.tail = new_tail;
                    }
                }
                return Ok(());
            }

            current = curr_obj.next_mut();
            is_head = false;
        }

        unreachable!("The region should be in range")
    }

    fn allocate_next_region(
        &mut self,
        size: usize,
        allocation_state: ObjectState,
    ) -> Result<VirtAddr, VMMAllocError> {
        // Prefer higher addresses, that is why we reverse
        let mut current = Some(self.tail_mut());
        let mut is_tail = true;

        while let Some(curr_obj) = current {
            if !curr_obj.allocated() && curr_obj.size >= size {
                let new_next = curr_obj
                    .split_to_fit(size)
                    .map_err(|()| VMMAllocError::OutOfMemory)?;

                curr_obj.state = allocation_state;
                let curr_addr = curr_obj.addr;

                if let Some(new_next) = new_next {
                    if is_tail {
                        self.tail = new_next;
                    }
                    self.len += 1;
                }
                return Ok(curr_addr);
            }

            current = curr_obj.prev_mut();
            // Praying to the gods of optimizations to do this
            is_tail = false;
        }
        Err(VMMAllocError::OutOfMemory)
    }

    fn debug_regions(&self) {
        crate::debug!(VirtualMemoryManager, "Memory Regions: ");
        let mut current = Some(self.head());

        while let Some(obj) = current {
            crate::debug!(
                VirtualMemoryManager,
                "Region at {:#x}: size = {:#x}, state = {:?}",
                obj.addr,
                obj.size,
                obj.state
            );
            current = obj.next();
        }
    }

    /// Marks a region as used by this VMM as DMA, even if it isn't mapped.
    ///
    /// Behaves the same as [`Self::map`] but without mapping or touching the region.
    pub fn mark_used(
        &mut self,
        start_addr: VirtAddr,
        size: usize,
        flags: VMMMFlags,
    ) -> Result<(), VMMAllocError> {
        self.allocate_at(start_addr, size, ObjectState::DMAAllocated(flags))
    }

    #[must_use = "Returns whether or not a region was found and unmapped"]
    /// Unmaps the region starting at `start_addr`, returning whether or not it was found, if it wasn't it is likely a kernel bug.
    pub fn unmap(&mut self, start_addr: VirtAddr) -> bool {
        let Some((deallocated, del_size)) = self.deallocate_at(start_addr) else {
            return false;
        };

        let end_addr = start_addr + del_size;
        match deallocated {
            ObjectState::Free => unreachable!("Attempt to deallocate an unallocated object."),
            ObjectState::Allocated(_) | ObjectState::LazyAllocated(_) /* TODO: Proper Lazy Allocation implementation */ => unsafe {
                self.page_table.free_unmap(start_addr, end_addr);
            },
            /* DMA is responsible for itself */
            ObjectState::DMAAllocated(_) => {},
        }

        true
    }

    pub fn map<I: Iterator<Item = Frame> + ExactSizeIterator>(
        &mut self,
        starting_addr: Option<VirtAddr>,
        size: usize,
        flags: VMMMFlags,
        mode: VMMAllocMode<I>,
    ) -> Result<VirtAddr, VMMAllocError> {
        let given_size = match mode {
            VMMAllocMode::DirectMapping(ref i) => Some(i.len() * PAGE_SIZE),
            _ => None,
        };

        if let Some(given_size) = given_size
            && given_size < size
        {
            return Err(VMMAllocError::InvalidSize);
        }

        let obj_state = match mode {
            VMMAllocMode::DirectMapping(_) => ObjectState::DMAAllocated(flags),
            VMMAllocMode::Normal => ObjectState::Allocated(flags),
            VMMAllocMode::Lazy => ObjectState::LazyAllocated(flags),
        };

        let allocated_start_addr = match starting_addr {
            Some(addr) => self.allocate_at(addr, size, obj_state).map(|()| addr)?,
            None => self.allocate_next_region(size, obj_state)?,
        };

        let mut map_flags = EntryFlags::empty();

        if flags.contains(VMMMFlags::WRITEABLE) {
            map_flags |= EntryFlags::WRITE;
        }

        if !flags.contains(VMMMFlags::EXECUTABLE) {
            map_flags |= EntryFlags::DISABLE_EXEC;
        }

        if flags.contains(VMMMFlags::UNCACHABLE) {
            map_flags |= EntryFlags::DEVICE_UNCACHEABLE;
        }

        if flags.contains(VMMMFlags::USER_ACCESSIBLE) {
            map_flags |= EntryFlags::USER_ACCESSIBLE;
        }

        match mode {
            VMMAllocMode::DirectMapping(frames) => unsafe {
                // Safety: We have got exclusive access to the whole address space we own, once a region is allocated,
                // we can safely map it, no one else can access it.
                self.page_table.map_contiguous_to_frames(
                    Page::containing_address(allocated_start_addr),
                    frames,
                    map_flags,
                )?;
            },
            VMMAllocMode::Normal | VMMAllocMode::Lazy => unsafe {
                self.page_table.alloc_map(
                    allocated_start_addr,
                    allocated_start_addr + size,
                    map_flags,
                )?;
            },
        }

        Ok(allocated_start_addr)
    }
}

#[test_case]
fn allocate_random_regions() {
    use crate::memory::paging::PhysPageTable;
    use crate::timer::{DurationFmt, SystemInstant};
    const RUNS: usize = 1000;
    let pseudo_page_table = PhysPageTable::create().expect("Failed to create a pseudo page table");
    let mut vmm = VirtualMemoryManager::new(
        VirtAddr::from(0x1000),
        0xFFFFFFFFFFF,
        pseudo_page_table.frame_ptr(),
    );

    let mut curr_i = 0;
    let size_choices = [1024, 2048, 4096, 8192];
    let mut results = heapless::Vec::<VirtAddr, { RUNS }>::new();

    let start_instant = SystemInstant::now();
    for _ in 0..RUNS {
        let size = size_choices[curr_i % size_choices.len()];
        let addr = vmm
            .allocate_next_region(size, ObjectState::Allocated(VMMMFlags::empty()))
            .expect("Allocations ran out of memory");
        results.push(addr).expect("Failed to push address");
        curr_i += 1;
    }

    let time_taken = start_instant.elapsed();
    crate::test_log!(
        "Time taken to allocate {} regions: {}",
        RUNS,
        DurationFmt::new(time_taken),
    );

    assert_eq!(
        vmm.len(),
        RUNS + 1, /* free region */
        "Not all regions allocated"
    );

    // ======== Deallocation ========
    // deallocating random regions

    let start_instant = SystemInstant::now();
    for index in 0..RUNS {
        let cpu_cycles = crate::arch::utils::cpu_cycles() as usize;
        let random_i = (index + cpu_cycles) % results.len();
        let addr = results.swap_remove(random_i);
        vmm.deallocate_at(addr)
            .expect("Failed to deallocate a region");
    }
    let time_taken = start_instant.elapsed();

    crate::test_log!(
        "Time taken to deallocate {} regions: {}",
        RUNS,
        DurationFmt::new(time_taken),
    );

    assert_eq!(vmm.len(), 1, "Failed to deallocate and combine all regions");
    vmm.debug_regions();
}

#[test_case]
fn allocate_random_regions_advanced() {
    #[derive(Clone, Copy)]
    enum Instruction {
        AllocateRandom(usize),
        NextSpecificAllocation,
    }

    use crate::memory::paging::PhysPageTable;
    use crate::timer::{DurationFmt, SystemInstant};

    const RUNS: usize = 1000;

    let pseudo_page_table = PhysPageTable::create().expect("Failed to create a pseudo page table");
    let mut vmm = VirtualMemoryManager::new(
        VirtAddr::from(0x1000),
        0xFFFFFFFFFFF,
        pseudo_page_table.frame_ptr(),
    );

    let mut curr_i = 0;

    let mut specific_allocations = heapless::Vec::<(usize, usize), 12>::from_slice(&[
        (0x5000, 0x1000),
        (0xA000, 0x1000),
        (0x10000, 0x1000),
        (0xB000000, 0x1000),
        (0xF21000, 0x1000),
        (0xAFAF000, 0x1000),
        (0x12345000, 0x1000),
        (0x1f1000, 0x1000),
        (0x30000000, 0x1000),
        (0x20000000, 0x1000),
        (0x20001000, 0x1000),
    ])
    .expect("Failed to construct instructions");
    let instructions = [
        Instruction::AllocateRandom(1024),
        Instruction::AllocateRandom(2048),
        Instruction::AllocateRandom(4096),
        Instruction::AllocateRandom(8192),
        Instruction::NextSpecificAllocation,
    ];

    let mut results = heapless::Vec::<VirtAddr, { RUNS }>::new();

    let start_instant = SystemInstant::now();
    for _ in 0..RUNS {
        let instruction = instructions[curr_i % instructions.len()];
        curr_i += 1;

        let addr = match instruction {
            Instruction::AllocateRandom(size) => vmm
                .allocate_next_region(size, ObjectState::Allocated(VMMMFlags::empty()))
                .expect("Allocations ran out of memory"),
            Instruction::NextSpecificAllocation => {
                let Some((addr, size)) = specific_allocations.pop() else {
                    continue;
                };
                let addr = VirtAddr::from(addr);
                if let Err(err) =
                    vmm.allocate_at(addr, size, ObjectState::Allocated(VMMMFlags::empty()))
                {
                    panic!(
                        "Failed to allocate specific region: {:#?}, addr: {:#?}, size: {}",
                        err, addr, size
                    );
                }
                addr
            }
        };
        results.push(addr).expect("Failed to push address");
    }
    let time_taken = start_instant.elapsed();

    crate::test_log!(
        "Time taken to allocate {} regions: {} us",
        results.len(),
        DurationFmt::new(time_taken),
    );

    assert!(
        vmm.len() >= results.len(),
        "VMM has {} objects, but expected at least {}",
        vmm.len(),
        results.len()
    );

    // ======== Deallocation ========
    // deallocating random regions

    let to_deallocate = results.len();
    let start_instant = SystemInstant::now();
    for index in 0..to_deallocate {
        let cpu_cycles = crate::arch::utils::cpu_cycles() as usize;
        let random_i = (index + cpu_cycles) % results.len();
        let addr = results.swap_remove(random_i);
        vmm.deallocate_at(addr)
            .expect("Failed to deallocate a region");
    }
    let time_taken = start_instant.elapsed();

    crate::test_log!(
        "Time taken to deallocate {} regions: {}",
        to_deallocate,
        DurationFmt::new(time_taken),
    );

    assert_eq!(vmm.len(), 1, "Failed to deallocate and combine all regions");
    vmm.debug_regions();
}
