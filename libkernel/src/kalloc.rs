use core::{alloc::Layout, ptr::NonNull};

use alloc::alloc::{AllocError, Allocator, GlobalAllocator};
use thiserror::Error;

use crate::ffi::FFIResult;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KAllocError {
    #[error("Invalid Layout")]
    InvalidLayout,
    #[error("Out Of Memory")]
    OutOfMemory,
}

impl From<AllocError> for KAllocError {
    fn from(value: AllocError) -> Self {
        let AllocError = value;
        Self::OutOfMemory
    }
}

impl From<KAllocError> for AllocError {
    fn from(value: KAllocError) -> Self {
        _ = value;
        Self
    }
}

unsafe extern "C" {
    fn kalloc(align: usize, size: usize) -> FFIResult<NonNull<u8>, KAllocError>;
    unsafe fn kfree(align: usize, size: usize, ptr: NonNull<u8>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The general puropse kernel allocator, uses [`super::slab_allocator`] internally.
pub struct KAlloc;

unsafe impl Allocator for KAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, core::alloc::AllocError> {
        unsafe {
            kalloc(layout.align(), layout.size())
                .into_result()
                .map(|n| NonNull::slice_from_raw_parts(n, layout.size()))
                .map_err(|e| e.into())
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { kfree(layout.align(), layout.size(), ptr) }
    }
}

unsafe impl GlobalAllocator for KAlloc {}
#[global_allocator]
static GLOBAL_ALLOCATOR: KAlloc = KAlloc;
