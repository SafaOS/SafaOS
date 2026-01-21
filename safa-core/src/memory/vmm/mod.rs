#[cfg(test)]
pub mod tests;

mod objects;

use core::{cell::SyncUnsafeCell, mem::MaybeUninit, ptr::NonNull};

use alloc::alloc::{AllocError, Allocator};
use safa_abi::errors::IntoErr;

use crate::{
    PhysAddr, VirtAddr,
    arch::without_interrupts,
    debug, error,
    memory::{
        AlignToPage,
        frame_allocator::{self, Frame, FramePtr},
        paging::{MapToError, PAGE_SIZE, Page, PageEntryFlags, PageTable, SyncPageTable},
        vmm::objects::{ObjectState, VMMObject, VMMObjectsPage},
    },
    thread,
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
        const ZEROED = 1 << 5;
    }
}

impl VMMMFlags {
    pub fn to_entry_flags(self) -> PageEntryFlags {
        let mut map_flags = PageEntryFlags::empty();

        if self.contains(VMMMFlags::WRITEABLE) {
            map_flags.insert(PageEntryFlags::WRITE);
        }

        if !self.contains(VMMMFlags::EXECUTABLE) {
            map_flags.insert(PageEntryFlags::DISABLE_EXEC);
        }

        if self.contains(VMMMFlags::UNCACHABLE) {
            map_flags.insert(PageEntryFlags::DEVICE_UNCACHEABLE);
        }

        if self.contains(VMMMFlags::FRAMEBUFFER_CACHED) {
            map_flags.insert(PageEntryFlags::FRAMEBUFFER_CACHED);
        }

        if self.contains(VMMMFlags::USER_ACCESSIBLE) {
            map_flags.insert(PageEntryFlags::USER_ACCESSIBLE);
        }

        map_flags
    }
}

/// Describes a VMM Location request
#[derive(Debug, Clone, Copy)]
pub enum Location {
    /// Address is just a hint, picked location would be after it or at it.
    Hint(VirtAddr),
    /// Address is fixed, the location would be picked at it.
    Fixed(VirtAddr),
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
    Used,
    InvalidSize,
}

impl From<MapToError> for VMMAllocError {
    fn from(value: MapToError) -> Self {
        match value {
            MapToError::FrameAllocationFailed => VMMAllocError::OutOfMemory,
            MapToError::AlreadyMapped => {
                unreachable!("VMM shouldn't try to map an already mapped region")
            }
            MapToError::NotMapped => unreachable!("VMM Shouldn't try to unmap an unmapped region"),
        }
    }
}

impl IntoErr for VMMAllocError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::Used => safa_abi::errors::ErrorStatus::AddressAlreadyInUse,
            Self::InvalidSize => safa_abi::errors::ErrorStatus::InvalidSize,
            Self::OutOfMemory => safa_abi::errors::ErrorStatus::OutOfMemory,
            Self::OutOfRange => safa_abi::errors::ErrorStatus::InvalidOffset,
        }
    }
}

#[derive(Debug)]
struct VMMInner {
    start_addr: VirtAddr,
    size: usize,
    root_objects_set: FramePtr<VMMObjectsPage>,
    head: NonNull<VMMObject>,
    tail: NonNull<VMMObject>,
    len: usize,
}

unsafe impl Send for VMMInner {}

impl VMMInner {
    fn tail_mut(&mut self) -> &mut VMMObject {
        unsafe { self.tail.as_mut() }
    }

    fn head_mut(&mut self) -> &mut VMMObject {
        unsafe { self.head.as_mut() }
    }

    fn head(&self) -> &VMMObject {
        unsafe { self.head.as_ref() }
    }

    #[cfg(test)]
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Lookup a VMM Object that contains the given address.
    pub fn lookup_addr_mut(&mut self, addr: VirtAddr) -> Option<&mut VMMObject> {
        if self.start_addr > addr || self.start_addr + self.size <= addr {
            return None;
        }

        let mut current = Some(self.head_mut());
        while let Some(obj) = current {
            if obj.addr() <= addr && obj.region_end() > addr {
                return Some(obj);
            }
            current = obj.next_mut();
        }

        unreachable!("Should find the address because it is within the VMM range")
    }

    fn grow_region(
        &mut self,
        addr: VirtAddr,
        extra_bytes: usize,
    ) -> Result<(VirtAddr, ObjectState, usize), VMMAllocError> {
        let Some(obj) = self.lookup_addr_mut(addr) else {
            return Err(VMMAllocError::OutOfRange);
        };

        assert!(obj.allocated(), "Attempt to grow an unallocated object");

        let (new_right, grew) = obj.try_grow(extra_bytes);

        let state = obj.state;
        let addr = obj.addr();
        let size = obj.size();

        if grew {
            // We absobed tail
            if new_right.is_none() {
                self.tail = obj.as_non_null();
            }
            Ok((addr, state, size))
        } else {
            Err(VMMAllocError::Used)
        }
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
                return Err(VMMAllocError::Used);
            }

            if curr_obj.addr() <= start_addr && curr_obj.region_end() >= end_addr {
                if curr_obj.allocated() {
                    return Err(VMMAllocError::Used);
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
        hint: Option<VirtAddr>,
        size: usize,
        allocation_state: ObjectState,
    ) -> Result<VirtAddr, VMMAllocError> {
        // Prefer higher addresses, that is why we reverse
        let mut current = Some(if hint.is_none() {
            self.tail_mut()
        } else {
            self.head_mut()
        });

        while let Some(curr_obj) = current {
            let is_tail = curr_obj.next().is_none();

            if hint.is_none_or(|h| curr_obj.addr() >= h || (curr_obj.region_end() >= (h + size)))
                && !curr_obj.allocated()
                && curr_obj.size() >= size
            {
                // If we are in the region hint describes, steal it and fragment, otherwise hint is out of the region or hint is at exactly that region.
                if let Some(h) = hint
                    && curr_obj.addr() < h
                    && curr_obj.region_end() >= (h + size)
                {
                    // FIXME: More efficient implementation
                    return self
                        .allocate_at(name, h, size, allocation_state)
                        .map(|()| h);
                }

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

            current = if hint.is_none() {
                curr_obj.prev_mut()
            } else {
                curr_obj.next_mut()
            };
        }
        Err(VMMAllocError::OutOfMemory)
    }

    fn debug_regions(&self) {
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
                "Attempt to free memory inside of the object's range and not at the start, addr: {:#x}, obj: {:#x}-{:#x}, {:?}",
                addr,
                obj.addr(),
                obj.region_end(),
                { self.debug_regions() }
            );

            current = obj.next_mut();
        }

        unreachable!("Should find the address because it is within the VMM range")
    }
}

impl Drop for VMMInner {
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

#[derive(Debug)]
pub struct VirtualMemoryManager {
    page_table: SyncPageTable,
    inner: SpinLock<VMMInner>,
}

impl VirtualMemoryManager {
    pub unsafe fn table_mut(&mut self) -> &mut PageTable {
        unsafe { &mut *self.page_table.inner_ptr_mut() }
    }

    pub unsafe fn table_ptr(&self) -> FramePtr<PageTable> {
        unsafe { *self.page_table.inner_ptr() }
    }

    pub fn new_user(page_table: FramePtr<PageTable>) -> Self {
        Self::new(VirtAddr::null(), usize::MAX / 2, page_table)
    }

    pub fn new(start_addr: VirtAddr, size: usize, page_table: FramePtr<PageTable>) -> Self {
        let mut objects =
            VMMObjectsPage::allocate().expect("Failed to allocate memory for storing VMM objects");
        let object = VMMObject::new_free(start_addr, size);
        let object_ptr = objects.add_object(object).expect("Failed to insert object");

        VirtualMemoryManager {
            inner: SpinLock::new(VMMInner {
                start_addr,
                size,
                root_objects_set: objects,
                head: object_ptr,
                tail: object_ptr,
                len: 1,
            }),

            page_table: unsafe { SyncPageTable::new(page_table) },
        }
    }

    pub fn grow_map(&self, addr: VirtAddr, needed: usize) -> Result<(), VMMAllocError> {
        let mut inner = self.inner.lock();
        let (addr, state, size) = inner.grow_region(addr, needed)?;

        let unmapped_addr = addr + (size - needed);
        let unmapped_size = needed;
        match state {
            ObjectState::Allocated(s) => {
                let mut op = self.page_table.begin();
                drop(inner);
                op.alloc_map(
                    unmapped_addr,
                    unmapped_addr + unmapped_size,
                    s.to_entry_flags(),
                )?;
            }
            ObjectState::LazyAllocated(_) => {}

            ObjectState::DMAAllocated(_) => unreachable!("Grew a DMA Allocated region"),
            ObjectState::Free => unreachable!(),
        }

        Ok(())
    }

    pub fn debug_regions(&self) {
        crate::debug!(VirtualMemoryManager, "Memory Regions: ");
        let inner = self.inner.lock();
        let mut current = Some(inner.head());

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

    #[must_use = "Returns whether or not a region was found and unmapped"]
    /// Unmaps the region starting at `start_addr`, returning whether or not it was found, if it wasn't it is likely a kernel bug.
    pub fn unmap(&self, start_addr: VirtAddr) -> bool {
        let mut inner = self.inner.lock();
        let Some((deallocated, del_size)) = inner.deallocate_at(start_addr) else {
            return false;
        };

        match deallocated {
            ObjectState::Free => unreachable!("Attempt to deallocate an unallocated object."),
            ObjectState::Allocated(_) | ObjectState::LazyAllocated(_) /* TODO: Proper Lazy Allocation implementation */ => unsafe {
                let mut op = self.page_table.begin();
                drop(inner);
                op.unmap_dealloc(start_addr, del_size.div_ceil(PAGE_SIZE)).expect("Failed to unmap VMM Allocated memory");
            },
            /* DMA is responsible for itself */
            ObjectState::DMAAllocated(_) => {
                let mut op = self.page_table.begin();
                drop(inner);
                unsafe { op.unmap(start_addr, del_size.div_ceil(PAGE_SIZE)).expect("Failed to unmap VMM Allocated memory") };
            },
        }

        true
    }

    /// Allocates a new memory region with size `size`, and maps it to newly allocated memory frames based on [`VMMAllocMode`].
    ///
    /// `size` must be a multiple of [`PAGE_SIZE`] or it panicks.
    pub fn map_new(
        &self,
        name: &'static &'static str,
        starting_addr: Option<Location>,
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
    pub fn map_direct<I: Iterator<Item = Frame> + Clone>(
        &self,
        name: &'static &'static str,
        starting_addr: Option<Location>,
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
        &self,
        name: &'static &'static str,
        start_addr: Option<Location>,
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

    fn map_inner<I: Iterator<Item = Frame> + Clone>(
        &self,
        name: &'static &'static str,
        starting_addr: Option<Location>,
        size: usize,
        flags: VMMMFlags,
        mode: VMMAllocMode,
        frames: Option<I>,
    ) -> Result<VirtAddr, VMMAllocError> {
        let given_size = match frames {
            Some(ref i) => Some(i.clone().count() * PAGE_SIZE),
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

        let mut inner = self.inner.lock();
        let allocated_start_addr = match starting_addr {
            Some(Location::Fixed(addr)) => inner
                .allocate_at(name, addr, size, obj_state)
                .map(|()| addr)?,
            None => inner.allocate_next_region(name, None, size, obj_state)?,
            Some(Location::Hint(hint)) => {
                inner.allocate_next_region(name, Some(hint), size, obj_state)?
            }
        };

        let map_flags = flags.to_entry_flags();

        let mut op = self.page_table.begin();
        drop(inner);
        match (mode, frames) {
            (VMMAllocMode::Normal, Some(frames)) => unsafe {
                // Safety: We have got exclusive access to the whole address space we own, once a region is allocated,
                // we can safely map it, no one else can access it.
                op.map_contiguous_to_frames(
                    Page::containing(allocated_start_addr),
                    frames,
                    map_flags,
                )?;
            },
            (VMMAllocMode::Normal | VMMAllocMode::Lazy, None) => {
                // FIXME: alloc_map zeroizes frames by default
                op.alloc_map(allocated_start_addr, allocated_start_addr + size, map_flags)?;
            }
            _ => unreachable!(),
        }

        Ok(allocated_start_addr)
    }
}

unsafe impl Send for VirtualMemoryManager {}

static VMM: SyncUnsafeCell<MaybeUninit<VirtualMemoryManager>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());

#[derive(Debug, Clone, Copy)]
pub struct VMMAlloc(&'static &'static str, Option<VirtAddr>, VMMMFlags);

impl VMMAlloc {
    #[inline]
    pub const fn new(name: &'static &'static str) -> Self {
        Self(name, None, VMMMFlags::WRITEABLE)
    }

    #[inline]
    pub const fn with_hint(mut self, hint: VirtAddr) -> Self {
        self.1 = Some(hint);
        self
    }

    fn allocate_new(
        self,
        vmm: &VirtualMemoryManager,
        size: usize,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let addr = vmm
            .map_new(
                self.0,
                self.1.map(|h| Location::Hint(h)),
                size,
                self.2,
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
}

unsafe impl Allocator for VMMAlloc {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<NonNull<[u8]>, alloc::alloc::AllocError> {
        debug_assert!(
            layout.align() <= PAGE_SIZE,
            "Alignment {} too big for VMM",
            layout.align()
        );
        let size = layout.size().to_next_page();

        with_root(|vmm| self.allocate_new(vmm, size))
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: core::alloc::Layout,
        new_layout: core::alloc::Layout,
    ) -> Result<NonNull<[u8]>, alloc::alloc::AllocError> {
        debug_assert!(
            new_layout.size() >= old_layout.size(),
            "`new_layout.size()` must be greater than or equal to `old_layout.size()`"
        );
        debug_assert!(
            new_layout.align() <= PAGE_SIZE,
            "Alignment {} too big for VMM",
            new_layout.align()
        );

        let new_size = new_layout.size().to_next_page();
        let old_size = old_layout.size().to_next_page();

        if new_size == old_size {
            return Ok(NonNull::slice_from_raw_parts(ptr, new_size));
        }

        let needed = new_size - old_size;

        let addr = VirtAddr::from_ptr(ptr.as_ptr());

        with_root(|vmm| {
            let try_grow = vmm.grow_map(addr, needed);
            match try_grow {
                Ok(()) => {
                    return Ok(NonNull::slice_from_raw_parts(ptr, new_size));
                }
                Err(VMMAllocError::Used) => {
                    let new_memory = self.allocate_new(vmm, new_size)?;
                    unsafe {
                        new_memory
                            .cast::<u8>()
                            .copy_from_nonoverlapping(ptr.cast::<u8>(), old_layout.size())
                    };

                    assert!(
                        vmm.unmap(VirtAddr::from_ptr(ptr.as_ptr())),
                        "Attempt to grow an unallocated region"
                    );
                    Ok(new_memory)
                }
                Err(VMMAllocError::OutOfMemory) => {
                    // TODO: Handle OOM
                    error!(VirtualMemoryManager, "OOM!!!!");
                    return Err(AllocError);
                }
                Err(VMMAllocError::OutOfRange) => {
                    unreachable!("Attempt to grow an unallocated region")
                }
                Err(_) => unreachable!(),
            }
        })
    }

    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: core::alloc::Layout,
        new_layout: core::alloc::Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let results = unsafe { self.grow(ptr, old_layout, new_layout)? };

        let to_zeroize = results.len() - old_layout.size();
        let zeroize_begin = unsafe { results.cast::<u8>().add(old_layout.size()) };
        unsafe { zeroize_begin.write_bytes(0, to_zeroize) };
        Ok(results)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        _ = layout;
        let addr = VirtAddr::from_ptr(ptr.as_ptr());

        with_root(|vmm| {
            assert!(
                vmm.unmap(addr),
                "Attempt to VMM Deallocate an unallocated region."
            );
        })
    }
}

/// Calls `f` with the higher half's [`VirtualMemoryManager`].
#[inline(always)]
pub fn with_root<F, R>(f: F) -> R
where
    F: FnOnce(&VirtualMemoryManager) -> R,
{
    without_interrupts(|| {
        let vmm_guard = unsafe { &mut *VMM.get() };
        f(unsafe { vmm_guard.assume_init_ref() })
    })
}

#[inline(always)]
pub fn with_user_vmm<F, R>(f: F) -> R
where
    F: FnOnce(&VirtualMemoryManager) -> R,
{
    without_interrupts(|| unsafe {
        thread::with_current_unsafe(|thread| f(&(*thread).process().vmm))
    })
}

/// Safety: VMM must not be initialized yet, this function is not thread-safe.
pub unsafe fn init(vmm: VirtualMemoryManager) {
    let vmm_guard = unsafe { &mut *VMM.get() };
    let vmm = vmm_guard.write(vmm);
    debug!(VirtualMemoryManager, "Initialized");
    vmm.debug_regions();
}
