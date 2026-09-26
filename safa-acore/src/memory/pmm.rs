use thiserror::Error;

use crate::{
    bootloader::INI_BOOTLOADER_MEMORY, logging::serial::_REGISTER_LOGGER,
    memory::region_list_allocator::RegionListAllocator, misc::Frame, oninit, sync::SpinLockIrq,
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

/// Allocates `count` contiugous frames with `align`-frames alignment.
pub fn allocate_frames(align: usize, count: usize) -> Result<Frame, PMMError> {
    REGION_ALLOCATOR
        .lock_no_irq(|mut alloc| alloc.allocate_frames(align, count))
        .ok_or(PMMError::OutOfMemory)
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum PMMError {
    #[error("PMM out of memory")]
    OutOfMemory,
}

#[inline(always)]
/// Allocates a single frame.
pub fn allocate_frame() -> Result<Frame, PMMError> {
    allocate_frames(1, 1)
}

/// Deallocates `count` contiugous frames starting at `base`.
///
/// Safety: each frame starting at `base` to `base`+count must no longer be used and allocated using this allocator.
pub unsafe fn deallocate_frames(base: Frame, count: usize) -> Result<(), PMMError> {
    REGION_ALLOCATOR.lock_no_irq(|mut alloc| alloc.deallocate_frames(base, count));
    Ok(())
}

#[inline(always)]
/// Deallocates a single frame `frame`.
///
/// Safety: `frame` must no longer be used and allocated using this allocator.
pub unsafe fn deallocate_frame(frame: Frame) -> Result<(), PMMError> {
    unsafe { deallocate_frames(frame, 1) }
}
