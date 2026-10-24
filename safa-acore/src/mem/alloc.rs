//! The general puropse kernel allocator, uses [`super::slab_allocator`] internally.
use core::{alloc::Layout, ptr::NonNull};

use libkernel::{ffi::FFIResult, slab::SCacheFlags};

use crate::{
    logging,
    mem::{
        slab::{INI_SLAB, SlabCacheRef, SlabError, slab_cache_create},
        vmm::INI_VMM,
    },
    misc::PAGE_SIZE,
    oninit,
};

#[cfg(test)]
mod tests;

pub use libkernel::kalloc::*;

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
                SCacheFlags::empty(),
            )
            .expect("Failed to create a cache for the kernel allocator")
        })
    };
}

oninit::define_routine! {
    /// Initializes the kernel Global allocator.
    pub unsafe fn INI_ALLOC = with ALLOC_CACHES, INI_VMM || {};
}

/// Returns all the slab caches the kernel allocator, assumes it is initialized (UB before init).
#[inline(always)]
fn caches() -> &'static [SlabCacheRef] {
    ALLOC_CACHES.borrow()
}

/// Allocates memory for a given [`layout`].
///
/// May allocate more memory.
#[inline]
pub fn kalloc(layout: Layout) -> Result<NonNull<[u8]>, KAllocError> {
    let requested_size = layout.size();
    let (idx, size) = ALLOC_CACHE_SIZES
        .iter()
        .enumerate()
        .find(|(_, p)| **p >= requested_size)
        .ok_or(KAllocError::InvalidLayout)?;

    let align = size.next_power_of_two().min(PAGE_SIZE);
    if align < layout.align() {
        logging::error!(
            "kalloc",
            "requested layout: {layout:?} couldn't satisfy because of alignment"
        );

        return Err(KAllocError::InvalidLayout);
    }

    let results = caches()[idx]
        .allocate()
        .map_err(|SlabError::OutOfMemory| KAllocError::OutOfMemory)?;

    debug_assert_eq!(results.len(), *size);
    Ok(results)
}

/// Frees a given [`ptr`] given it's `layout`.
/// If the layout isn't the same as the one that was used in [`kalloc`], expect UB.
#[inline]
pub unsafe fn kfree(ptr: NonNull<u8>, layout: Layout) {
    let requested_size = layout.size();
    let (idx, _) = ALLOC_CACHE_SIZES
        .iter()
        .enumerate()
        .find(|(_, p)| **p >= requested_size)
        .expect("Couldn't find matching size for layout");

    caches()[idx].free(ptr)
}

#[unsafe(export_name = "kalloc")]
extern "C" fn kalloc_impl(align: usize, size: usize) -> FFIResult<NonNull<u8>, KAllocError> {
    kalloc(Layout::from_size_align(size, align).map_err(|_| KAllocError::InvalidLayout)?)
        .map(|n| n.cast::<u8>())
        .into()
}
#[unsafe(export_name = "kfree")]
unsafe extern "C" fn kfree_impl(align: usize, size: usize, ptr: NonNull<u8>) {
    unsafe {
        kfree(
            ptr,
            Layout::from_size_align(size, align).expect("Failed to construct a layout for kfree"),
        )
    };
}
