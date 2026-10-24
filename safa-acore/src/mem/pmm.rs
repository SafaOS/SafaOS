use libkernel::ffi::FFIResult;

use crate::{
    bootloader::INI_BOOTLOADER_MEMORY, logging::serial::_REGISTER_LOGGER,
    mem::region_list_allocator::RegionListAllocator, misc::Frame, oninit, sync::SpinLockIrq,
};

#[cfg(test)]
mod test;

oninit::define! {
    /// The region allocator used for allocating physical memory regions.
    unsafe static REGION_ALLOCATOR: SpinLockIrq<RegionListAllocator> = with INI_BOOTLOADER_MEMORY, _REGISTER_LOGGER ||
        SpinLockIrq::new(RegionListAllocator::create());
}

oninit::define_routine! {
    /// Initializes the PMM.
    pub unsafe fn INI_PMM = with REGION_ALLOCATOR || {};
}

pub use libkernel::pmm::*;
/// Allocates `count` contiugous frames with `align`-frames alignment.
#[unsafe(no_mangle)]
extern "C" fn SafaM_allocate_frames(align: usize, count: usize) -> FFIResult<Frame, PMMError> {
    REGION_ALLOCATOR
        .lock_no_irq(|mut alloc| alloc.allocate_frames(align, count))
        .ok_or(PMMError::OutOfMemory)
        .into()
}

/// Deallocates `count` contiugous frames starting at `base`.
///
/// Safety: each frame starting at `base` to `base`+count must no longer be used and allocated using this allocator.
#[unsafe(no_mangle)]
unsafe extern "C" fn SafaM_deallocate_frames(base: Frame, count: usize) -> FFIResult<(), PMMError> {
    REGION_ALLOCATOR.lock_no_irq(|mut alloc| alloc.deallocate_frames(base, count));
    FFIResult::Ok(())
}
