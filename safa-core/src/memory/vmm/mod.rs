#[cfg(test)]
pub mod tests;

mod objects;

use core::{mem::MaybeUninit, ptr::NonNull};

use alloc::alloc::Allocator;

use crate::{
    PhysAddr, VirtAddr,
    arch::paging::PageTable,
    error, info,
    memory::{
        AlignToPage,
        frame_allocator::{self, Frame, FramePtr},
        paging::{EntryFlags, MapToError, PAGE_SIZE, Page},
        vmm::objects::{ObjectState, VMMObject, VMMObjectsPage},
    },
    utils::locks::SpinLock,
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
        const FRAMEBUFFER_CACHED = 1 << 4;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VMMAllocMode {
    /// Normal allocation mode
    ///
    /// The region is allocated immediately and mapped to the virtual address space, you don't control what it is mapped to.
    Normal,
    /// Lazy allocation mode
    ///
    /// unlike [`VMMAllocMode::Normal`], the region is allocated as needed on first access.
    Lazy,
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
    pub fn table_mut(&mut self) -> &mut PageTable {
        &mut *self.page_table
    }

    pub unsafe fn table_ptr(&self) -> FramePtr<PageTable> {
        self.page_table
    }

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
            if obj.addr() <= addr && obj.region_end() > addr {
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
            if obj.addr() == addr {
                assert!(
                    obj.allocated(),
                    "Attempt to free unallocated memory, this is a bug"
                );

                let old_state = core::mem::replace(&mut obj.state, ObjectState::Free);
                let size = obj.size();

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
                !(obj.addr() > addr && obj.region_end() > addr),
                "Attempt to free memory inside of the object's range and not at the start"
            );

            current = obj.next_mut();
        }

        unreachable!("Should find the address because it is within the VMM range")
    }

    fn allocate_at(
        &mut self,
        name: &'static &'static str,
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
            let is_tail = curr_obj.next().is_none();

            if curr_obj.addr() > start_addr {
                let prev = curr_obj.prev();

                crate::warn!(
                    VirtualMemoryManager,
                    "Request allocation area fragmented, detected on: addr={:?}, size={:#x}, state={:?}",
                    curr_obj.addr(),
                    curr_obj.size(),
                    curr_obj.state,
                );
                if let Some(prev) = prev {
                    crate::serial!(
                        "prev addr={:?}, prev size={:#x}, prev state={:?}\n",
                        prev.addr(),
                        prev.size(),
                        prev.state
                    );
                }
                return Err(VMMAllocError::AlreadyUsed);
            }

            if curr_obj.addr() <= start_addr && curr_obj.region_end() >= end_addr {
                if curr_obj.allocated() {
                    return Err(VMMAllocError::AlreadyUsed);
                }

                let offset = start_addr - curr_obj.addr();
                let (new_head, new_tail) = curr_obj
                    .split_at(offset, size)
                    .map_err(|()| VMMAllocError::OutOfMemory)?;
                curr_obj.state = obj_state;
                curr_obj.name = name;
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
        name: &'static &'static str,
        size: usize,
        allocation_state: ObjectState,
    ) -> Result<VirtAddr, VMMAllocError> {
        // Prefer higher addresses, that is why we reverse
        let mut current = Some(self.tail_mut());
        let mut is_tail = true;

        while let Some(curr_obj) = current {
            if !curr_obj.allocated() && curr_obj.size() >= size {
                let new_next = curr_obj
                    .split_to_fit(size)
                    .map_err(|()| VMMAllocError::OutOfMemory)?;

                curr_obj.state = allocation_state;
                curr_obj.name = name;

                let curr_addr = curr_obj.addr();

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

    pub fn debug_regions(&self) {
        crate::debug!(VirtualMemoryManager, "Memory Regions: ");
        let mut current = Some(self.head());

        while let Some(obj) = current {
            crate::debug!(
                VirtualMemoryManager,
                "{} at {:#x}: size = {:#x}, state = {:?}",
                obj.name,
                obj.addr(),
                obj.size(),
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
        name: &'static &'static str,
        start_addr: VirtAddr,
        size: usize,
        flags: VMMMFlags,
    ) -> Result<(), VMMAllocError> {
        self.allocate_at(name, start_addr, size, ObjectState::DMAAllocated(flags))
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

    /// Allocates a new memory region with size `size`, and maps it to newly allocated memory frames based on [`VMMAllocMode`].
    ///
    /// `size` must be a multiple of [`PAGE_SIZE`] or it panicks.
    pub fn map_new(
        &mut self,
        name: &'static &'static str,
        starting_addr: Option<VirtAddr>,
        size: usize,
        flags: VMMMFlags,
        mode: VMMAllocMode,
    ) -> Result<VirtAddr, VMMAllocError> {
        assert!(size.is_multiple_of(PAGE_SIZE));
        self.map_inner::<core::iter::Empty<Frame>>(name, starting_addr, size, flags, mode, None)
    }

    /// like [`Self::map_new`] but you provide the physical addresses that this region is mapped to.
    ///
    /// The provided frames total size must be equal to or more than the requested allocation size or it will return an error [`VMMAllocError::InvalidSize`].
    pub fn map_direct<I: Iterator<Item = Frame> + ExactSizeIterator>(
        &mut self,
        name: &'static &'static str,
        starting_addr: Option<VirtAddr>,
        size: usize,
        flags: VMMMFlags,
        frames: I,
    ) -> Result<VirtAddr, VMMAllocError> {
        assert!(size.is_multiple_of(PAGE_SIZE));

        self.map_inner(
            name,
            starting_addr,
            size,
            flags,
            VMMAllocMode::Normal,
            Some(frames),
        )
    }

    /// Variaint of [`Self::map_direct`]
    pub fn map_direct_phys(
        &mut self,
        name: &'static &'static str,
        start_addr: Option<VirtAddr>,
        start_phys: PhysAddr,
        page_count: usize,
        flags: VMMMFlags,
    ) -> Result<VirtAddr, VMMAllocError> {
        let size = page_count * PAGE_SIZE;
        let end_addr = start_phys + size;
        let frames = Frame::iter_frames(
            Frame::containing_address(start_phys),
            Frame::containing_address(end_addr),
        );

        self.map_direct(name, start_addr, size, flags, frames)
    }

    fn map_inner<I: Iterator<Item = Frame> + ExactSizeIterator>(
        &mut self,
        name: &'static &'static str,
        starting_addr: Option<VirtAddr>,
        size: usize,
        flags: VMMMFlags,
        mode: VMMAllocMode,
        frames: Option<I>,
    ) -> Result<VirtAddr, VMMAllocError> {
        let given_size = match frames {
            Some(ref i) => Some(i.len() * PAGE_SIZE),
            _ => None,
        };

        if let Some(given_size) = given_size
            && given_size < size
        {
            return Err(VMMAllocError::InvalidSize);
        }

        let obj_state = match (mode, &frames) {
            (VMMAllocMode::Normal, Some(_)) => ObjectState::DMAAllocated(flags),
            (VMMAllocMode::Normal, None) => ObjectState::Allocated(flags),
            (VMMAllocMode::Lazy, None) => ObjectState::LazyAllocated(flags),
            (VMMAllocMode::Lazy, Some(_)) => unreachable!("Cannot lazy allocate DMA memory"),
        };

        let allocated_start_addr = match starting_addr {
            Some(addr) => self
                .allocate_at(name, addr, size, obj_state)
                .map(|()| addr)?,
            None => self.allocate_next_region(name, size, obj_state)?,
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

        if flags.contains(VMMMFlags::FRAMEBUFFER_CACHED) {
            map_flags |= EntryFlags::FRAMEBUFFER_CACHED;
        }

        if flags.contains(VMMMFlags::USER_ACCESSIBLE) {
            map_flags |= EntryFlags::USER_ACCESSIBLE;
        }

        match (mode, frames) {
            (VMMAllocMode::Normal, Some(frames)) => unsafe {
                // Safety: We have got exclusive access to the whole address space we own, once a region is allocated,
                // we can safely map it, no one else can access it.
                self.page_table.map_contiguous_to_frames(
                    Page::containing_address(allocated_start_addr),
                    frames,
                    map_flags,
                )?;
            },
            (VMMAllocMode::Normal | VMMAllocMode::Lazy, None) => unsafe {
                self.page_table.alloc_map(
                    allocated_start_addr,
                    allocated_start_addr + size,
                    map_flags,
                )?;
            },
            _ => unreachable!(),
        }

        Ok(allocated_start_addr)
    }
}

unsafe impl Send for VirtualMemoryManager {}

static VMM: SpinLock<MaybeUninit<VirtualMemoryManager>> = SpinLock::new(MaybeUninit::uninit());

#[derive(Debug, Clone, Copy)]
pub struct VMMAlloc(&'static &'static str);

unsafe impl Allocator for VMMAlloc {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<NonNull<[u8]>, alloc::alloc::AllocError> {
        assert!(
            layout.align() <= PAGE_SIZE,
            "Alignment {} too big for VMM",
            layout.align()
        );
        let size = layout.size().to_next_page();

        let mut vmm_guard = VMM.lock();
        let vmm = unsafe { vmm_guard.assume_init_mut() };
        let addr = vmm
            .map_new(
                self.0,
                None,
                size,
                VMMMFlags::WRITEABLE,
                VMMAllocMode::Normal,
            )
            .map_err(|e| {
                error!(VirtualMemoryManager, "VMM map returned error: {e:?}");
                alloc::alloc::AllocError
            })?;

        Ok(NonNull::slice_from_raw_parts(
            NonNull::new(addr.into_ptr::<u8>()).expect("VMM map returned a null VirtAddr"),
            size,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        _ = layout;

        let mut vmm_guard = VMM.lock();
        let vmm = unsafe { vmm_guard.assume_init_mut() };

        let addr = VirtAddr::from_ptr(ptr.as_ptr());
        assert!(
            vmm.unmap(addr),
            "Attempt to VMM Deallocate an unallocated region."
        );
    }
}

/// Calls `f` with the higher half's [`VirtualMemoryManager`].
#[inline(always)]
pub fn with_root<F, R>(f: F) -> R
where
    F: FnOnce(&mut VirtualMemoryManager) -> R,
{
    let mut vmm_guard = VMM.lock();
    f(unsafe { vmm_guard.assume_init_mut() })
}

pub fn init(vmm: VirtualMemoryManager) {
    let mut vmm_guard = VMM.lock();
    let vmm = vmm_guard.write(vmm);
    info!(VirtualMemoryManager, "Initialized");
    vmm.debug_regions();
}
