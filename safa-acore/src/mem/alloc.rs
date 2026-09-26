//! The general puropse kernel allocator, uses [`super::slab_allocator`] internally.
use core::{
    alloc::{Allocator, GlobalAllocator, Layout},
    ptr::NonNull,
};

use thiserror::Error;

use crate::{
    logging,
    mem::slab::{INI_SLAB, SlabCacheRef, SlabError, slab_cache_create},
    misc::PAGE_SIZE,
    oninit,
};

#[cfg(test)]
mod tests;

const ALLOC_CACHE_COUNT: usize = 14;

const ALLOC_CACHE_SIZES: [usize; ALLOC_CACHE_COUNT] = [
    8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
];

oninit::define! {
    unsafe static ALLOC_CACHES: [SlabCacheRef; ALLOC_CACHE_COUNT] = with INI_SLAB || {
        core::array::from_fn(|i| {
            let size = ALLOC_CACHE_SIZES[i];
            let align = size.next_power_of_two().min(PAGE_SIZE);
            slab_cache_create(
                &"KernelAlloc",
                Layout::from_size_align(size, align)
                    .expect("failed to construct layout for kernel allocator"),
                None,
                None,
            )
            .expect("Failed to create a cache for the kernel allocator")
        })
    };
}

oninit::define_routine! {
    /// Initializes the kernel Global allocator.
    pub unsafe fn INI_ALLOC = with ALLOC_CACHES || {};
}

/// Returns all the slab caches the kernel allocator, assumes it is initialized (UB before init).
#[inline(always)]
fn caches() -> &'static [SlabCacheRef] {
    ALLOC_CACHES.borrow()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KAllocError {
    #[error("Out Of Memory")]
    OutOfMemory,
    #[error("Layout too big for the allocator to satisfy")]
    LayoutTooBig,
}

/// Allocates memory for a given [`layout`].
///
/// May allocate more memory.
pub fn kalloc(layout: Layout) -> Result<NonNull<[u8]>, KAllocError> {
    let requested_size = layout.size();
    let (idx, size) = ALLOC_CACHE_SIZES
        .iter()
        .enumerate()
        .find(|(_, p)| **p >= requested_size)
        .ok_or(KAllocError::LayoutTooBig)?;

    let align = size.next_power_of_two().min(PAGE_SIZE);
    if align < layout.align() {
        logging::error!(
            "kalloc",
            "requested layout: {layout:?} couldn't satisfy because of alignment"
        );

        return Err(KAllocError::LayoutTooBig);
    }

    let results = caches()[idx]
        .allocate()
        .map_err(|SlabError::OutOfMemory| KAllocError::OutOfMemory)?;

    debug_assert_eq!(results.len(), *size);
    Ok(results)
}

/// Frees a given [`ptr`] given it's `layout`.
/// If the layout isn't the same as the one that was used in [`kalloc`], expect UB.
pub unsafe fn kfree(ptr: NonNull<u8>, layout: Layout) {
    let requested_size = layout.size();
    let (idx, _) = ALLOC_CACHE_SIZES
        .iter()
        .enumerate()
        .find(|(_, p)| **p >= requested_size)
        .expect("Couldn't find matching size for layout");

    caches()[idx].free(ptr)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The general puropse kernel allocator, uses [`super::slab_allocator`] internally.
pub struct KAlloc;

unsafe impl Allocator for KAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, core::alloc::AllocError> {
        kalloc(layout).map_err(|_| core::alloc::AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { kfree(ptr, layout) }
    }
}

unsafe impl GlobalAllocator for KAlloc {}
#[global_allocator]
static GLOBAL_ALLOCATOR: KAlloc = KAlloc;
