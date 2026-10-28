//! Wrappers around the SafaOS's physical memory manager.
//!
//! You can use that to directly allocate physical memory, other ways to allocate and map memory include:
//!
//! - [`crate::vmm`]: better for allocating contigous page sized blocks of memory from uncontigous physical pages, should have all you need to map/unmap virtual memory and works with the PMM.
//! - [`crate::slab`]: best for frequent allocation of a lot of fixed sized long/short lived objects.
//! - [`crate::kalloc`]: a general puropse global allocator, that uses slab, good enough for small sized long lived objects if allocating isn't frequent.
use thiserror::Error;

use crate::{
    bootloader::hhdm,
    ffi::FFIResult,
    misc::{Frame, PhysAddr, VirtAddr},
};

unsafe extern "C" {
    fn SafaM_allocate_frames(align: usize, count: usize) -> FFIResult<Frame, PMMError>;
    fn SafaM_deallocate_frames(base: Frame, count: usize) -> FFIResult<(), PMMError>;

}

#[inline(always)]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(hhdm().raw() | phys.raw())
}

#[inline(always)]
pub fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    PhysAddr::new(virt.raw() - hhdm().raw())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[repr(u8)]
pub enum PMMError {
    #[error("PMM: Out Of Memory")]
    OutOfMemory,
}

/// Allocates `count` contiugous frames with `align`-frames alignment.
#[inline(always)]
pub fn allocate_frames(align: usize, count: usize) -> Result<Frame, PMMError> {
    unsafe { SafaM_allocate_frames(align, count) }.into_result()
}

/// Deallocates `count` contiugous frames starting at `base`.
///
/// Safety: each frame starting at `base` to `base`+count must no longer be used and allocated using this allocator.
#[inline(always)]
pub unsafe fn deallocate_frames(base: Frame, count: usize) -> Result<(), PMMError> {
    unsafe { SafaM_deallocate_frames(base, count) }.into_result()
}

#[inline(always)]
/// Allocates a single frame.
pub fn allocate_frame() -> Result<Frame, PMMError> {
    allocate_frames(1, 1)
}

#[inline(always)]
/// Deallocates a single frame `frame`.
///
/// Safety: `frame` must no longer be used and allocated using this allocator.
pub unsafe fn deallocate_frame(frame: Frame) -> Result<(), PMMError> {
    unsafe { deallocate_frames(frame, 1) }
}
