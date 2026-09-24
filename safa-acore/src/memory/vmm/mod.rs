#[cfg(test)]
pub mod tests;

pub mod tree;
use core::{cell::SyncUnsafeCell, mem::MaybeUninit};

use alloc::alloc::AllocError;

use crate::{
    logging,
    memory::{
        pmm::PMMError,
        vmm::tree::{VMAEntry, VMAState, VMATree},
    },
    misc::{Frame, PAGE_SIZE, Page, PhysAddr, VirtAddr},
    paging::{MapToError, PageEntryFlags, PageTable},
    sync::IntSpinLock,
};

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct VMMMFlags: u8 {
        /// By default the region is read-only.
        const WRITABLE = 1 << 0;
        /// By default the region is not executable.
        const EXECUTABLE = 1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const UNCACHABLE = 1 << 3;
        const FRAMEBUFFER_CACHED = 1 << 4;
        const ZEROED = 1 << 5;
        const _GLOBAL_HINT = 1 << 6;
    }
}

impl VMMMFlags {
    pub fn to_entry_flags(self) -> PageEntryFlags {
        let mut map_flags = PageEntryFlags::empty();

        if self.contains(VMMMFlags::WRITABLE) {
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

        if self.contains(VMMMFlags::_GLOBAL_HINT) {
            map_flags.insert(PageEntryFlags::_GLOBAL_SHARED);
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
    UsedBy {
        at: VirtAddr,
        size: usize,
        flags: VMMMFlags,
    },
    Used,
    InvalidSize,
}

impl From<PMMError> for VMMAllocError {
    fn from(value: PMMError) -> Self {
        match value {
            PMMError::OutOfMemory => Self::OutOfMemory,
        }
    }
}

impl From<AllocError> for VMMAllocError {
    fn from(_: AllocError) -> Self {
        Self::OutOfMemory
    }
}

impl From<MapToError> for VMMAllocError {
    fn from(value: MapToError) -> Self {
        match value {
            MapToError::OutOfMemory => VMMAllocError::OutOfMemory,
            MapToError::AlreadyMapped => {
                unreachable!("VMM shouldn't try to map an already mapped region")
            }
            MapToError::NotMapped => unreachable!("VMM Shouldn't try to unmap an unmapped region"),
        }
    }
}

#[derive(Debug)]
pub struct VirtualMemoryManager {
    page_table: IntSpinLock<PageTable>,
    inner: IntSpinLock<VMATree>,
}

impl VirtualMemoryManager {
    #[inline(always)]
    pub fn table_inner(&self) -> &IntSpinLock<PageTable> {
        &self.page_table
    }

    #[inline]
    pub fn table_addr(&self) -> PhysAddr {
        // Safety: This should never change.
        unsafe { (*self.table_inner().get()).phys_addr() }
    }

    pub fn new_user(page_table: PageTable) -> Self {
        Self::new(VirtAddr::null(), usize::MAX / 2, page_table)
    }

    pub const fn new(start_addr: VirtAddr, size: usize, page_table: PageTable) -> Self {
        let tree = VMATree::new(start_addr, size);
        VirtualMemoryManager {
            inner: IntSpinLock::new(tree),

            page_table: IntSpinLock::new(page_table),
        }
    }

    /// Prints debug information about the regions in the VMA tree.
    pub fn debug_regions(&self) {
        let inner = self.inner.lock();
        let mut iter = inner.iter();
        let mut last_desc = None;

        while let Some(mut desc) = iter.next() {
            if let Some(last) = last_desc {
                let mut n = 0;

                while desc == last {
                    n += 1;

                    if let Some(next) = iter.next() {
                        desc = next;
                        continue;
                    }

                    break;
                }

                if n > 0 {
                    logging::debug!(VMM, "{}: repeated {n} times", last.name());
                }

                if desc == last {
                    break;
                }
            }

            logging::debug!(
                VMM,
                "{}: addr={:?},size={:#x},state={:?},flags={:?}",
                desc.name(),
                desc.addr(),
                desc.size(),
                desc.state(),
                desc.flags()
            );
            last_desc = Some(desc);
        }
    }

    #[must_use = "Returns whether or not a region was found and unmapped"]
    /// Unmaps the neighbor regions starting at `start_addr` and ending at `start_addr+size`, if no such contiuguos regions were found returns false.
    ///
    /// size and start_addr has to be a multiple of `PAGE_SIZE`, or else it panicks.
    pub fn unmap_contiugous(&self, start_addr: VirtAddr, size: usize) -> bool {
        assert!(
            size.is_multiple_of(PAGE_SIZE),
            "Invalid size passed to unmap"
        );
        assert!(
            start_addr.raw().is_multiple_of(PAGE_SIZE),
            "Invalid start address passed to unmap"
        );

        let mut inner = self.inner.lock();
        inner.remove_contiguous_with(start_addr, size, |desc| {
            let mut op = self.page_table.lock();
            match desc.state() {
                state @ VMAState::Normal | state @ VMAState::Lazy => unsafe {
                    op.unmap_dealloc(
                        desc.addr(),
                        desc.size().div_ceil(PAGE_SIZE),
                        state == VMAState::Lazy,
                    )
                    .expect("Failed to unmap a VMM range")
                },
                VMAState::DMA => unsafe {
                    op.unmap_no_dealloc(desc.addr(), desc.size().div_ceil(PAGE_SIZE), false)
                        .expect("Failed to unmap VMM Allocated memory")
                },
            }
        })
    }

    #[must_use = "Returns whether or not a region was found and unmapped"]
    /// Unmaps the region starting at `start_addr`, returning whether or not it was found, if it wasn't it is likely a kernel bug.
    pub fn unmap(&self, start_addr: VirtAddr) -> bool {
        let mut inner = self.inner.lock();
        let Some(desc) = inner.remove_containing(start_addr) else {
            return false;
        };

        let state = desc.state();
        match state {
            VMAState::Normal | VMAState::Lazy => unsafe {
                let mut op = self.page_table.lock();
                drop(inner);
                op.unmap_dealloc(
                    start_addr,
                    desc.size().div_ceil(PAGE_SIZE),
                    matches!(state, VMAState::Lazy),
                )
                .expect("Failed to unmap VMM Allocated memory");
            },
            /* DMA is responsible for itself */
            VMAState::DMA => {
                let mut op = self.page_table.lock();
                drop(inner);
                unsafe {
                    op.unmap_no_dealloc(start_addr, desc.size().div_ceil(PAGE_SIZE), false)
                        .expect("Failed to unmap VMM Allocated memory")
                };
            }
        }

        true
    }

    #[must_use = "Returns whether or not a region was found"]
    pub fn set_page_flags(&self, start_addr: VirtAddr, flags: VMMMFlags) -> bool {
        let mut inner = self.inner.lock();
        let Some((key, value)) = inner.lookup_mut(start_addr) else {
            return false;
        };

        let addr = key.addr();
        let size = key.size();
        let state = value.state();

        *value.flags_mut() = flags;

        let mut new_page_flags = flags.to_entry_flags();
        if state == VMAState::Lazy {
            new_page_flags |= PageEntryFlags::_IS_LAZY;
        }

        let mut op = self.page_table.lock();
        drop(inner);
        unsafe {
            op.set_flags(addr, size.div_ceil(PAGE_SIZE), new_page_flags)
                .expect("VMM failed to change the flags of a page, should never happen");
        }

        true
    }

    #[must_use = "Returns whether or not a region was found"]
    pub fn set_page_flags_contiguous(
        &self,
        start_addr: VirtAddr,
        size: usize,
        flags: VMMMFlags,
    ) -> bool {
        assert!(
            size.is_multiple_of(PAGE_SIZE),
            "Invalid size passed to set page flags"
        );
        assert!(
            start_addr.is_multiple_of(PAGE_SIZE),
            "Invalid start address passed to set page flags"
        );

        let mut inner = self.inner.lock();
        let mut modified = false;

        inner
            .modify_contiguous_or_remove(start_addr, size, |addr, size, old_ent| {
                modified = true;
                let mut new_ent = *old_ent;
                let mut op = self.page_table.lock();

                let state = new_ent.state();
                *new_ent.flags_mut() = flags;

                let mut new_page_flags = flags.to_entry_flags();
                if state == VMAState::Lazy {
                    new_page_flags |= PageEntryFlags::_IS_LAZY;
                }

                unsafe { op.set_flags(addr, size.div_ceil(PAGE_SIZE), new_page_flags) }
                    .expect("VMM failed to change the flags of a page, should never happen");

                Some(new_ent)
            })
            .expect("Out Of Memory while modifying page flags");

        modified
    }
    /// Attempts to map the given `addr` on demand returning wheither it was successful.
    ///
    /// And if it wasn't returns whether an Object containing address was found or not.
    pub fn try_on_demand_map(&self, addr: VirtAddr) -> Result<(), Option<(VirtAddr, usize)>> {
        let mut inner = self.inner.lock();
        let obj = inner.lookup(addr).ok_or(None)?;
        let state = obj.state();
        let start_addr = obj.addr();
        let size = obj.size();
        let flags = obj.flags();

        match state {
            VMAState::Lazy => {}
            VMAState::Normal | VMAState::DMA => {
                logging::debug!(
                    VirtualMemoryManager,
                    "Attempt to recover from non-lazy region: at: {start_addr:?} with size {size} => {state:?}, vmm page table: {:?}",
                    self.page_table.get()
                );
                drop(inner);
                self.debug_regions();
                return Err(Some((start_addr, size)));
            }
        };

        let diff = addr - start_addr;
        let pages_left = (size - diff).div_ceil(PAGE_SIZE);

        debug_assert_ne!(pages_left, 0);
        // Maps 4 pages at a time to account for my kinda slow pagefault and lookup process.
        let pages_to_map = 4.min(pages_left);

        let mut op = self.page_table.lock();
        drop(inner);
        match unsafe {
            op.alloc_map_missing(
                addr,
                addr + (pages_to_map * PAGE_SIZE),
                flags.to_entry_flags(),
            )
        } {
            Ok(_) | Err(MapToError::AlreadyMapped) => Ok(()),
            Err(MapToError::OutOfMemory) => {
                logging::error!(
                    VirtualMemoryManager,
                    "OOM while trying to lazy allocate address: {addr:?}, of a VMM memory allocation at: {start_addr:?} with size: {size} and state: {state:?}",
                );
                Err(Some((start_addr, size)))
            }
            Err(MapToError::NotMapped) => unreachable!(),
        }
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
        let frames = Frame::iter_addresses(start_phys, end_addr);

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

        let vma_state = match (mode, &frames) {
            (VMMAllocMode::Normal, Some(_)) => VMAState::DMA,
            (VMMAllocMode::Normal, None) => VMAState::Normal,
            (VMMAllocMode::Lazy, None) => VMAState::Lazy,
            (VMMAllocMode::Lazy, Some(_)) => unreachable!("Cannot lazy allocate DMA memory"),
        };

        let mut inner = self.inner.lock();
        let allocated_start_addr =
            inner.allocate_gap(starting_addr, size, VMAEntry::new(name, vma_state, flags))?;

        let map_flags = flags.to_entry_flags();

        match (mode, frames) {
            (VMMAllocMode::Normal, Some(frames)) => unsafe {
                // Safety: We have got exclusive access to the whole address space we own, once a region is allocated,
                // we can safely map it, no one else can access it.
                let mut op = self.page_table.lock();
                drop(inner);
                let result = op.map_contiguous_to_frames(
                    Page::containing(allocated_start_addr),
                    frames,
                    map_flags,
                );
                result?;
            },
            (VMMAllocMode::Lazy, None) => {
                // do nothing
                drop(inner);
            }
            (VMMAllocMode::Normal, None) => {
                // FIXME: alloc_map zeroizes frames by default
                let mut op = self.page_table.lock();
                drop(inner);
                let result = op.alloc_map(
                    allocated_start_addr,
                    allocated_start_addr + size,
                    map_flags,
                    flags.contains(VMMMFlags::ZEROED),
                );
                result?;
            }
            (VMMAllocMode::Lazy, Some(_)) => unreachable!(),
        }

        Ok(allocated_start_addr)
    }
}

unsafe impl Send for VirtualMemoryManager {}

static VMM: SyncUnsafeCell<MaybeUninit<VirtualMemoryManager>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());

/// Calls `f` with the higher half's [`VirtualMemoryManager`].
#[inline(always)]
pub fn with_root<F, R>(f: F) -> R
where
    F: FnOnce(&VirtualMemoryManager) -> R,
{
    let vmm_guard = unsafe { &mut *VMM.get() };
    f(unsafe { vmm_guard.assume_init_ref() })
}

/// Safety: VMM must not be initialized yet, this function is not thread-safe.
pub unsafe fn init(vmm: VirtualMemoryManager) {
    let vmm_guard = unsafe { &mut *VMM.get() };
    let vmm = vmm_guard.write(vmm);
    logging::debug!(VirtualMemoryManager, "Initialized");
    #[cfg(test)]
    vmm.debug_regions();
    _ = vmm;
}

// /// Attempts to recover from a page fault with addr `addr`.
// /// If not succesfful returns whether the VMM found a region containing address or not.
// pub fn try_page_fault_recover(addr: VirtAddr) -> Result<(), Option<(VirtAddr, usize)>> {
//     let addr = addr.prev_page();
//     let lower_addr = addr.is_in_lower_half();

//     let f = |vmm: &VirtualMemoryManager| vmm.try_on_demand_map(addr);

//     if lower_addr {
//         with_user_vmm(f)
//     } else {
//         with_root(f)
//     }
// }
