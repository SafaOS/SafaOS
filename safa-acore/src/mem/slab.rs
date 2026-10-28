//! An implementation of a slab allocator based on Solaris.

use core::{alloc::Layout, cell::SyncUnsafeCell, ptr::NonNull};

use crate::{
    mem::{pmm::INI_PMM, vmm::INI_VMM},
    oninit,
};

pub use libakernel::slab::*;

/// The slab cache of slab caches.
#[unsafe(no_mangle)]
#[used]
static ROOT_SLAB_CACHE: SlabCache = SlabCache::new(
    &"/SlabCache",
    Layout::new::<SlabCache>(),
    None,
    None,
    SCacheFlags::empty(),
);

/// The slab cache of indirect slabs metadata.
///
/// TODO: Link this up with [`ROOT_SLAB_CACHE`]?
#[unsafe(no_mangle)]
#[used]
static INDIRECT_SLAB_CACHE: SyncUnsafeCell<SlabCache> = SyncUnsafeCell::new(SlabCache::new(
    &"/SlabIndirect",
    Layout::new::<IndirectSlab>(),
    Some(indirect_slab_init),
    None,
    SCacheFlags::NO_INDIRECT,
));

#[repr(transparent)]
struct TempMag(SyncUnsafeCell<Option<NonNull<SlabCache>>>);
unsafe impl Send for TempMag {}
unsafe impl Sync for TempMag {}

/// The magazine cache is a cache of N magazine caches, where N is the number of CPUs.
///
/// It isn't really correctly statically initialized.
#[unsafe(no_mangle)]
#[used]
static PERCPU_MAGAZINE_CACHE: TempMag = TempMag(SyncUnsafeCell::new(None));

#[unsafe(no_mangle)]
#[used]
static MAGAZINE_CACHE: SyncUnsafeCell<SlabCache> = SyncUnsafeCell::new(SlabCache::new(
    &"/MagazineCache",
    Layout::new::<Magazine>(),
    Some(magazine_init),
    None,
    SCacheFlags::NO_MAGAZINES.union(SCacheFlags::NO_INDIRECT),
));

oninit::define_routine! {
    /// The essentials required for a slab allocator to function.
    pub unsafe fn INI_SLAB = with INI_PMM || {};
    /// Initializes the Slab Allocator.
    ///
    /// Using it before that isn't exactly UB.
    unsafe fn INIT_SLAB_ALLOCATOR = with INI_SLAB, INI_VMM || {
        unsafe {
            init();
        }
    };
}
