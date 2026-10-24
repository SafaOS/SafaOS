use alloc::alloc::AllocError;

use crate::{
    ffi::{FFIResult, FFIStr},
    misc::VirtAddr,
    paging::{MapToError, PageEntryFlags},
    pmm::PMMError,
};

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(C)]
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
#[repr(u8)]
pub enum Location {
    /// Allows the VMM to pick any location it wants as long as it fits.
    Fit,
    /// Address is just a hint, picked location would be after it or at it.
    Hint(VirtAddr),
    /// Address is fixed, the location would be picked at it.
    Fixed(VirtAddr),
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
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
#[repr(u8)]
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

unsafe extern "C" {
    fn SafaM_vm_map_new(
        name: FFIStr<'static>,
        starting_addr: Location,
        pages: usize,
        flags: VMMMFlags,
        mode: VMMAllocMode,
    ) -> FFIResult<VirtAddr, VMMAllocError>;
    fn SafaM_vm_unmap(addr: VirtAddr, n: usize) -> FFIResult<(), ()>;
}

#[inline(always)]
/// VMM maps a new kernelspace region with the name `name` at the address picked according to `addr`, with `n` pages. and `flags` flags.
pub fn map_new(
    name: &'static str,
    addr: Location,
    pages: usize,
    flags: VMMMFlags,
    mode: VMMAllocMode,
) -> Result<VirtAddr, VMMAllocError> {
    unsafe { SafaM_vm_map_new(FFIStr::from_str(name), addr, pages, flags, mode) }.into_result()
}

#[inline(always)]
/// Attempts to unmap `n` pages starting at `addr`.
pub unsafe fn unmap(addr: VirtAddr, n: usize) -> Result<(), ()> {
    unsafe { SafaM_vm_unmap(addr, n) }.into_result()
}
