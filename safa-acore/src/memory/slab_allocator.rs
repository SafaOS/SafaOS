//! An implementation of a slab allocator based on Solaris.

use core::{alloc::Layout, cell::SyncUnsafeCell, ops::Deref, ptr::NonNull};

use bitflags::bitflags;

use crate::{
    bootloader, logging,
    memory::vmm::{self, VMMAllocError, VMMMFlags},
    misc::{PAGE_SIZE, VirtAddr},
    percpu,
    sync::{IntSpinLock, SpinLockGuard},
};

const MAGAZINE_CACHE_SIZE: usize = 8;

/// A magazine is like a stack of pre-allocated objects, you push or pop an object from it.
///
/// it is used per cpu for fast allocation of objects, without having to wait for the global lock.
struct Magazine {
    objects: heapless::Vec<NonNull<FreeObject>, MAGAZINE_CACHE_SIZE>,
    next: Option<NonNull<Magazine>>,
}

impl Magazine {
    pub const fn new() -> Self {
        Self {
            objects: heapless::Vec::new(),
            next: None,
        }
    }

    #[inline(always)]
    pub fn pop(&mut self) -> Option<NonNull<FreeObject>> {
        self.objects.pop()
    }

    #[inline(always)]
    /// Attempts to push an object up the magazine returning an Err(it) if it is full.
    pub fn push(&mut self, object: NonNull<FreeObject>) -> Result<(), NonNull<FreeObject>> {
        self.objects.push(object)
    }
}

/// MagazineCache is a cache of [`Magazine`]s, one per cpu for each [`SlabCache].
///
/// it consists of a loaded [`Magazine`] and a previous [`Magazine`], and switches between them to avoid contention, the idea is one magazine is full and the other is empty so an allocs/deallocs are separate.
struct MagazineCache {
    loaded: Option<NonNull<Magazine>>,
    previous: Option<NonNull<Magazine>>,
}

impl MagazineCache {
    pub const fn new() -> Self {
        Self {
            loaded: None,
            previous: None,
        }
    }

    /// Returns the loaded magazine.
    #[inline(always)]
    pub fn loaded(&mut self) -> Option<&mut Magazine> {
        unsafe { self.loaded.map(|mut m| m.as_mut()) }
    }

    /// Swaps magazines between loaded and previous one.
    #[inline(always)]
    pub fn swap(&mut self) {
        core::mem::swap(&mut self.loaded, &mut self.previous);
    }

    /// Loads a magazine dropping the previous one (returns it),
    #[inline(always)]
    pub fn load(&mut self, magazine: NonNull<Magazine>) -> Option<NonNull<Magazine>> {
        // If there is no previous to replace with then we should keep the current one.
        let previous = if self.loaded.is_some() {
            self.previous.take()
        } else {
            None
        };

        self.previous = self.loaded.take();
        self.loaded = Some(magazine);
        previous
    }
}

struct PerCpuMagazineCache {
    cache: IntSpinLock<MagazineCache>,
}

impl PerCpuMagazineCache {
    pub const fn new() -> Self {
        Self {
            cache: IntSpinLock::new(MagazineCache::new()),
        }
    }
}

/// MagazineDepot is a depot of [`Magazine`]s, used to allocate and deallocate them, it is global and not per-cpu.
struct MagazineDepot {
    head: IntSpinLock<Option<NonNull<Magazine>>>,
}

impl MagazineDepot {
    pub const fn new() -> Self {
        Self {
            head: IntSpinLock::new(None),
        }
    }

    /// Pushes a magazine to the depot list.
    pub fn push(&self, mut magazine: NonNull<Magazine>) {
        let mut head = self.head.lock();
        unsafe { magazine.as_mut().next = head.take() };
        *head = Some(magazine);
    }

    /// Pops a magazine from the depot list.
    pub fn pop(&self) -> Option<NonNull<Magazine>> {
        let mut head = self.head.lock();
        let magazine = head.take();

        if let Some(mut m) = magazine {
            *head = unsafe { m.as_mut().next.take() };
        }
        magazine
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct SCacheFlags: u32 {
        const INDIRECT = 1 << 1;
    }
}

/// When to start indirecting slabs (putting metadata in the slab itself).
const SLAB_INDIRECTION_SZ_THRESHOLD: usize = 512;

type SlabDeinitializer = fn(&SlabCache, &mut [u8]);
type SlabInitializer = fn(&SlabCache, &mut [u8]);

struct FreeObject {
    next: Option<NonNull<FreeObject>>,
}

struct Slab {
    magic: u32,
    free_count: u32,
    next: Option<NonNull<Self>>,
    prev: Option<NonNull<Self>>,
    head: Option<NonNull<FreeObject>>,
}

struct IndirectSlab {
    slab: Slab,
    memory_at: NonNull<u8>,
}

impl Slab {
    const HEADER_MAGIC: u32 = 0xb00b51AB;
    const DIRECT_HEADER_OFF: usize = PAGE_SIZE - size_of::<Self>();
    const _A: () = assert!(Self::DIRECT_HEADER_OFF.is_multiple_of(align_of::<Self>()));

    pub const fn new(obj_cnt: u32) -> Self {
        Self {
            magic: Self::HEADER_MAGIC,
            next: None,
            prev: None,
            head: None,
            free_count: obj_cnt,
        }
    }

    /// Given a direct slab, returns the block of memory it occupies.
    ///
    /// # Safety: has to be a direct slab.
    #[inline(always)]
    unsafe fn direct_block(&self) -> NonNull<u8> {
        let this: NonNull<Slab> = NonNull::from_ref(self);
        unsafe { this.byte_sub(Self::DIRECT_HEADER_OFF).cast::<u8>() }
    }

    /// Given a direct slab block, returns the slab itself.
    ///
    /// # Safety: has to be a direct slab block.
    #[inline(always)]
    unsafe fn from_direct_block(block: NonNull<u8>) -> NonNull<Self> {
        unsafe { block.byte_add(Self::DIRECT_HEADER_OFF).cast::<Slab>() }
    }

    /// Given a block of memory for a slab, returns an iterator over all the objects in it.
    #[inline(always)]
    fn all_objects(
        block_ptr: NonNull<u8>,
        obj_cnt: u32,
        obj_sz: u32,
    ) -> impl Iterator<Item = NonNull<FreeObject>> {
        let mut objs_ptr = block_ptr.cast::<FreeObject>();
        (0..obj_cnt).map(move |_| {
            let obj = objs_ptr;
            objs_ptr = unsafe { objs_ptr.byte_add(obj_sz as usize) };
            obj
        })
    }

    /// Initializes an indirect/direct slab given a pointer to its data and its metadata.
    pub unsafe fn init_free(
        block: NonNull<u8>,
        mut this: NonNull<Self>,
        obj_cnt: u32,
        obj_sz: u32,
    ) {
        let this_mut = unsafe { this.as_mut() };

        *this_mut = Self::new(obj_cnt);
        let objs = Self::all_objects(block, obj_cnt, obj_sz);

        for mut obj in objs {
            unsafe { obj.as_mut().next = this_mut.head };
            this_mut.head = Some(obj);
        }
    }

    /// Initializes a direct slab given a pointer to its metadata, which is directly stored within the slab itself.
    pub unsafe fn init_free_direct(
        mut this: NonNull<Self>,
        obj_cnt: u32,
        obj_sz: u32,
        slab_sz: u32,
    ) {
        let this_block = unsafe { this.as_ref().direct_block() };

        debug_assert_eq!(
            this_block.as_ptr() as usize + Self::DIRECT_HEADER_OFF,
            this.as_ptr() as usize
        );

        unsafe { Self::init_free(this_block, this, obj_cnt, obj_sz) };

        unsafe {
            // Ensure the head pointer is within the slab bounds.
            debug_assert!(
                this.as_mut().head.is_some_and(|h| (h.as_ptr() as usize
                    - (this.as_ptr() as usize - Self::DIRECT_HEADER_OFF))
                    < slab_sz as usize),
                "Head pointer out of bounds for a slab, during slab initialization"
            )
        }
    }

    /// Removes slab from linked list.
    ///
    /// Safety: `head` must be the head of the list `this` currently belongs to.
    #[inline]
    pub unsafe fn remove_from_list(this: NonNull<Self>, head: &mut Option<NonNull<Slab>>) {
        let (prev, next) = unsafe {
            let slab = this.as_ref();
            (slab.prev, slab.next)
        };

        match prev {
            Some(mut p) => unsafe { p.as_mut().next = next },
            // this assumes that head == this.
            None => *head = next,
        }
        if let Some(mut n) = next {
            unsafe { n.as_mut().prev = prev };
        }
    }

    /// Add the slab to the linked list with head at `head`.
    ///
    /// Safety: Must be called with a valid `NonNull<Self>` and `head` must have sole mutable access over the whole list.
    #[inline]
    pub unsafe fn add_to_list(mut this: NonNull<Self>, head: &mut Option<NonNull<Slab>>) {
        let slab = unsafe { this.as_mut() };

        if let Some(mut prev_list) = head.take() {
            slab.next = Some(prev_list);
            unsafe { prev_list.as_mut().prev = Some(this) };
        }
        *head = Some(this);
    }
}

/// Describes all the protected global lists within a [`SlabCache`].
struct SlabCacheL {
    next: Option<NonNull<SlabCache>>,
    slabs_full: Option<NonNull<Slab>>,
    slabs_partial: Option<NonNull<Slab>>,
    slabs_free: Option<NonNull<Slab>>,
}

impl SlabCacheL {
    pub const fn new() -> Self {
        Self {
            next: None,
            slabs_full: None,
            slabs_partial: None,
            slabs_free: None,
        }
    }
}

/// A slab cache is a cache of [`Slab`]s, which in turn contain [`FreeObject`]s.
///
/// it describes everything about slabs, contains their lists and how to build them, and has a per-CPU cache of [`Magazine`]s.
///
/// so particularly it is where a slab allocator starts.
pub struct SlabCache {
    lists: IntSpinLock<SlabCacheL>,

    depot_full: MagazineDepot,
    depot_empty: MagazineDepot,

    // UnsafeCell because it might be enabled on init after the slab was already allocated.
    cpu_cache: SyncUnsafeCell<Option<NonNull<[PerCpuMagazineCache]>>>,

    init: Option<SlabInitializer>,
    deinit: Option<SlabDeinitializer>,

    // meta
    slab_sz: u32,
    object_sz: u32,
    object_cnt: u32,
    object_align: u32,
    flags: SCacheFlags,

    name: &'static &'static str,
}

impl SlabCache {
    pub const fn new(
        name: &'static &'static str,
        obj_layout: Layout,
        initializer: Option<SlabInitializer>,
        deinitializer: Option<SlabDeinitializer>,
    ) -> Self {
        assert!(obj_layout.size() <= u32::MAX as usize);
        assert!(obj_layout.align() <= u32::MAX as usize);

        let object_align = obj_layout.align() as u32;
        let mut object_sz = (obj_layout.size() as u32).next_multiple_of(object_align);
        if object_sz < size_of::<FreeObject>() as u32 {
            object_sz = (size_of::<FreeObject>() as u32).next_multiple_of(object_align);
        }

        let indirect = object_sz >= SLAB_INDIRECTION_SZ_THRESHOLD as u32;
        let mut flags = SCacheFlags::empty();

        let slab_sz;
        let object_cnt;
        if !indirect {
            slab_sz = (object_sz + size_of::<Slab>() as u32)
                .next_multiple_of(object_align)
                .next_multiple_of(PAGE_SIZE as u32);
            object_cnt = (slab_sz - size_of::<Slab>() as u32) / object_sz;
        } else {
            slab_sz = object_sz
                .next_multiple_of(object_align)
                .next_multiple_of(PAGE_SIZE as u32);
            object_cnt = slab_sz / object_sz;

            flags = flags.union(SCacheFlags::INDIRECT);
        }

        Self {
            lists: IntSpinLock::new(SlabCacheL::new()),
            cpu_cache: SyncUnsafeCell::new(None),
            depot_empty: MagazineDepot::new(),
            depot_full: MagazineDepot::new(),
            object_sz,
            object_cnt,
            object_align,
            slab_sz,
            flags,
            init: initializer,
            deinit: deinitializer,
            name,
        }
    }

    fn try_init_magazines(&mut self) -> Result<(), VMMAllocError> {
        let data = percpu_magazine_cache().slab_allocate()?;

        let cpu_count = bootloader::cpu_count();
        assert_eq!(
            data.len(),
            size_of::<PerCpuMagazineCache>() * cpu_count,
            "magazine cache size mismatch, not exact as much memory as needed for {} CPUs",
            cpu_count
        );

        let data = NonNull::slice_from_raw_parts(data.cast::<PerCpuMagazineCache>(), cpu_count);
        self.cpu_cache = SyncUnsafeCell::new(Some(data));
        Ok(())
    }

    /// Returns a reference to the CPU cache, if one is available.
    ///
    /// Safety: only putting in a CPU cache when there are None is unsafe.
    #[inline(always)]
    fn cpu_cache(&self) -> Option<&[PerCpuMagazineCache]> {
        unsafe { (*self.cpu_cache.get()).map(|v| v.as_ref()) }
    }

    /// Returns whether this cache uses indirect slab allocation (metadata at the same memory as the slab).
    #[inline(always)]
    pub const fn indirect(&self) -> bool {
        self.flags.contains(SCacheFlags::INDIRECT)
    }

    /// Allocates memory for a new slab (the slab itself, which if in direct mode then it should contain the metadata).
    fn new_slab_memory(&self) -> Result<NonNull<u8>, VMMAllocError> {
        vmm::with_root(|vmm| {
            vmm.map_new(
                self.name,
                None,
                self.slab_sz as usize,
                VMMMFlags::WRITABLE | VMMMFlags::ZEROED,
                vmm::VMMAllocMode::Normal,
            )
            .map(|v| {
                let block_header = NonNull::new(v.into_ptr::<u8>())
                    .expect("Failed to make a pointer to a slab header because VMM returned null");
                block_header
            })
        })
    }

    /// Allocates a new indirect slab for the cache.
    fn new_indirect_slab(&self) -> Result<NonNull<IndirectSlab>, VMMAllocError> {
        let mut slab = indirect_cache().allocate()?.cast::<IndirectSlab>();

        let drop_guard = DeallocSlab { slab };
        struct DeallocSlab {
            slab: NonNull<IndirectSlab>,
        }
        impl Drop for DeallocSlab {
            fn drop(&mut self) {
                indirect_cache().free(self.slab.cast());
            }
        }

        let memory = self.new_slab_memory()?;

        unsafe {
            Slab::init_free(memory, slab.cast::<Slab>(), self.object_cnt, self.object_sz);

            slab.as_mut().memory_at = memory;
        };

        core::mem::forget(drop_guard);
        Ok(slab)
    }
    #[inline]
    /// Allocates a new direct slab for this cache.
    fn new_direct_slab(&self) -> Result<NonNull<Slab>, VMMAllocError> {
        self.new_slab_memory().map(|block_header| {
            let slab_header = unsafe {
                block_header
                    .byte_add(Slab::DIRECT_HEADER_OFF)
                    .cast::<Slab>()
            };
            unsafe {
                Slab::init_free_direct(slab_header, self.object_cnt, self.object_sz, self.slab_sz)
            };
            slab_header
        })
    }

    #[inline(always)]
    /// Allocates a new slab for this cache, either directly or indirectly depending on the cache's flags.
    fn new_slab(&self) -> Result<NonNull<Slab>, VMMAllocError> {
        if self.indirect() {
            self.new_indirect_slab().map(|c| c.cast())
        } else {
            self.new_direct_slab()
        }
    }

    #[inline]
    /// Given a pointer to an allocated object, returns the slab it belongs to.
    fn slab_from_object(
        &self,
        lists: &SlabCacheL,
        object_ptr: NonNull<FreeObject>,
    ) -> NonNull<Slab> {
        if self.indirect() {
            // Indirect mode: All slabs are actually [`IndirectSlab`], look it up in both the full and the partial lists
            for list in [lists.slabs_full, lists.slabs_partial] {
                let mut current = list;
                while let Some(slab) = current {
                    let indirect_slab = unsafe { slab.cast::<IndirectSlab>().as_ref() };
                    let memory_at = indirect_slab.memory_at.as_ptr() as usize;
                    let memory_max = memory_at + self.slab_sz as usize;

                    if memory_at <= object_ptr.as_ptr() as usize
                        && memory_max > object_ptr.as_ptr() as usize
                    {
                        return slab;
                    }

                    current = indirect_slab.slab.next;
                }
            }

            panic!(
                "Slab allocator object: {object_ptr:p} doesn't belong to current cache: {self:p} '{}'",
                self.name
            )
        } else {
            // Direct mode: the slab is at the same address as the object, the max slab size is PAGE_SIZE
            assert!(self.slab_sz <= PAGE_SIZE as u32);
            // NOP
            let raw_addr = VirtAddr::from_ptr(object_ptr.as_ptr());
            // Null check
            let block_addr = NonNull::new(raw_addr.prev_page().into_ptr())
                .expect("Slab block shouldn't be null");
            // Add
            unsafe { Slab::from_direct_block(block_addr) }
        }
    }

    /// Attempts to allocate from a magazine returning None if we should fallback to slow allocate.
    fn magazine_allocate(&self) -> Option<NonNull<FreeObject>> {
        let cpu_cache = self.cpu_cache()?;
        let mut cpu_cache = cpu_cache[percpu::cpu_index()].cache.lock();

        if let Some(obj) = cpu_cache.loaded().and_then(|m| m.pop()) {
            return Some(obj);
        }
        // Swaps and tries again
        cpu_cache.swap();

        if let Some(obj) = cpu_cache.loaded().and_then(|m| m.pop()) {
            return Some(obj);
        }

        if let Some(mag) = self.depot_full.pop() {
            // both mags are empty as verified by the previous swap operation.
            if let Some(prev) = cpu_cache.load(mag) {
                self.depot_empty.push(prev);
            }

            let obj = cpu_cache
                .loaded()
                .and_then(|m| m.pop())
                .expect("Magazine should be loaded and full");
            return Some(obj);
        }
        None
    }

    #[inline]
    /// Attempts to free `obj_ptr` to a local CPU magazine.
    ///
    /// on success returns Ok(()), on Failure returns Err(c) where c is the per cpu magazine cache to fill and try again or none if we should fallback to slow_allocate.
    fn magazine_free<'s>(
        &'s self,
        obj_ptr: NonNull<FreeObject>,
    ) -> Result<(), Option<SpinLockGuard<'s, MagazineCache>>> {
        let cpu_cache = self.cpu_cache().ok_or(None)?;
        let mut cpu_cache = cpu_cache[percpu::cpu_index()].cache.lock();

        if let Some(()) = cpu_cache.loaded().and_then(|m| m.push(obj_ptr).ok()) {
            return Ok(());
        }
        // Swaps and tries again
        cpu_cache.swap();

        if let Some(()) = cpu_cache.loaded().and_then(|m| m.push(obj_ptr).ok()) {
            return Ok(());
        }

        if let Some(mag) = self.depot_empty.pop() {
            // both mags are full as verified by the previous swap operation.
            if let Some(prev) = cpu_cache.load(mag) {
                self.depot_full.push(prev);
            }

            cpu_cache
                .loaded()
                .and_then(|m| m.push(obj_ptr).ok())
                .expect("Magazine should be loaded and full");
            return Ok(());
        }

        Err(Some(cpu_cache))
    }

    /// Initializes a free object given it's pointer, returns a pointer slice to it's data.
    fn init_object(&self, object: NonNull<FreeObject>) -> NonNull<[u8]> {
        let object_ptr = object.cast::<u8>();
        let object_len = self.object_sz as usize;
        let mut data_slice = NonNull::slice_from_raw_parts(object_ptr, object_len);

        if let Some(initializer) = self.init {
            initializer(self, unsafe { data_slice.as_mut() });
        }
        data_slice
    }

    /// Deinitializes a previously allocated object given it's data pointer, returns a pointer to a free deinitialized object.
    fn deinit_object(&self, data_ptr: NonNull<u8>) -> NonNull<FreeObject> {
        let mut data_slice = NonNull::slice_from_raw_parts(data_ptr, self.object_sz as usize);

        if let Some(deinit) = self.deinit {
            deinit(self, unsafe { data_slice.as_mut() });
        }

        data_ptr.cast()
    }

    /// Slow free deallocation operation that doesn't use per-cpus or anything special.
    ///
    /// Doesn't deinitialize the object beforehand (TODO: proper deinit and init? should it be done like this)
    fn slow_slab_free(&self, object_ptr: NonNull<FreeObject>) {
        debug_assert!(self.slab_sz.is_multiple_of(PAGE_SIZE as u32));

        let mut slabs_list = self.lists.lock();
        let slabs_list = &mut *slabs_list;

        let mut slab_ptr = self.slab_from_object(&slabs_list, object_ptr);

        let slab = unsafe { slab_ptr.as_mut() };
        let object = unsafe { object_ptr.cast::<FreeObject>().as_mut() };

        // Reading magic should be safe as long as the object pointer is valid
        assert_eq!(
            slab.magic,
            Slab::HEADER_MAGIC,
            "Slab was corrupted or object pointer is invalid"
        );

        object.next = slab.head.take();
        slab.head = Some(object_ptr);
        slab.free_count += 1;

        assert!(slab.free_count <= self.object_cnt);
        if slab.free_count == self.object_cnt || slab.free_count == 1 {
            let (remove_from, add_to) = if self.object_cnt == 1 {
                (&mut slabs_list.slabs_full, &mut slabs_list.slabs_free)
            } else if slab.free_count == self.object_cnt {
                // free == object_cnt meaning it was a partial and now fully free.
                (&mut slabs_list.slabs_partial, &mut slabs_list.slabs_free)
            } else {
                // free was 1 meaning it was in full and moving to partial
                (&mut slabs_list.slabs_full, &mut slabs_list.slabs_partial)
            };

            unsafe {
                Slab::remove_from_list(slab_ptr, remove_from);
                Slab::add_to_list(slab_ptr, add_to);
            }
        }
    }

    /// Slow allocate operation that doesn't use per-cpus or anything special.
    fn slab_allocate(&self) -> Result<NonNull<[u8]>, VMMAllocError> {
        let mut slabs_list_guard = self.lists.lock();
        let slabs_list = &mut *slabs_list_guard;

        let (mut slab_ptr, source) = if let Some(p) = slabs_list.slabs_partial {
            (p, Some(&mut slabs_list.slabs_partial))
        } else if let Some(f) = slabs_list.slabs_free {
            (f, Some(&mut slabs_list.slabs_free))
        } else {
            (self.new_slab()?, None)
        };

        let slab = unsafe { slab_ptr.as_mut() };
        assert!(slab.free_count != 0);

        let mut object = slab.head.take().expect("slab should have a free object");
        slab.free_count -= 1;
        slab.head = unsafe { object.as_mut().next.take() };

        if slab.free_count == 0 || slab.free_count == self.object_cnt - 1 {
            if let Some(source) = source {
                unsafe { Slab::remove_from_list(slab_ptr, source) };
            }
            let add_to = if slab.free_count == 0 {
                // free was 1 meaning it is now full, and was partial
                &mut slabs_list.slabs_full
            } else {
                // free was objects_cnt meaning it was free and now is partial (or full if free_count == 0 which is handled above)
                &mut slabs_list.slabs_partial
            };
            unsafe {
                Slab::add_to_list(slab_ptr, add_to);
            }
        }

        drop(slabs_list_guard);

        Ok(self.init_object(object))
    }

    /// The main allocation interface.
    ///
    /// May use [`Self::magazine_allocate`] or fallback to [`Self::slab_allocate`].
    pub fn allocate(&self) -> Result<NonNull<[u8]>, VMMAllocError> {
        if let Some(object) = self.magazine_allocate() {
            return Ok(self.init_object(object));
        }

        self.slab_allocate()
    }

    /// The main deallocation interface.
    ///
    /// May use [`Self::magazine_deallocate`] or fallback to [`Self::slow_slab_free`].
    pub fn free(&self, data_ptr: NonNull<u8>) {
        let object = self.deinit_object(data_ptr);
        match self.magazine_free(object) {
            Ok(()) => return,
            Err(None) => return self.slow_slab_free(object),
            Err(Some(mut cpu_cache)) => {
                let Ok(magazine) = allocate_magazine() else {
                    return self.slow_slab_free(object);
                };

                if let Some(_) = cpu_cache.load(magazine) {
                    unreachable!("There should be no magazines in the CPU cache")
                }

                cpu_cache
                    .loaded()
                    .expect("The magazine should be loaded")
                    .push(object)
                    .expect("The allocated magazine should be empty")
            }
        }
    }
}

unsafe impl Send for SlabCache {}
unsafe impl Sync for SlabCache {}

// Ensures that all indirect slabs are structly valid to avoid some UB.
fn indirect_slab_init(_cache: &SlabCache, data: &mut [u8]) {
    debug_assert_eq!(data.len(), size_of::<IndirectSlab>());

    let ptr = data.as_mut_ptr().cast::<IndirectSlab>();
    unsafe {
        *ptr = IndirectSlab {
            slab: Slab {
                magic: Slab::HEADER_MAGIC,
                free_count: 0,
                next: None,
                prev: None,
                head: None,
            },
            memory_at: NonNull::dangling(),
        }
    }
}

fn magazine_init(_cache: &SlabCache, data: &mut [u8]) {
    debug_assert_eq!(data.len(), size_of::<Magazine>());

    let ptr = data.as_mut_ptr().cast::<Magazine>();
    unsafe {
        *ptr = Magazine::new();
    }
}

fn percpu_magazine_cache_init(_cache: &SlabCache, data: &mut [u8]) {
    let cpu_count = bootloader::cpu_count();
    debug_assert_eq!(data.len(), size_of::<PerCpuMagazineCache>() * cpu_count);

    let slice = data.as_mut_ptr().cast::<PerCpuMagazineCache>();
    for i in 0..cpu_count {
        unsafe {
            *slice.add(i) = PerCpuMagazineCache::new();
        }
    }
}

/// The slab cache of slab caches.
static ROOT_SLAB_CACHE: SlabCache =
    SlabCache::new(&"/SlabCache", Layout::new::<SlabCache>(), None, None);

/// The slab cache of indirect slabs metadata.
///
/// TODO: Link this up with [`ROOT_SLAB_CACHE`]?
static INDIRECT_SLAB_CACHE: SyncUnsafeCell<SlabCache> = SyncUnsafeCell::new(SlabCache::new(
    &"/SlabIndirect",
    Layout::new::<IndirectSlab>(),
    Some(indirect_slab_init),
    None,
));

/// The magazine cache is a cache of N magazine caches, where N is the number of CPUs.
///
/// It isn't really correctly statically initialized.
static PERCPU_MAGAZINE_CACHE: SyncUnsafeCell<SlabCache> = SyncUnsafeCell::new(SlabCache::new(
    &"/MagazineCache",
    Layout::new::<()>(),
    None,
    None,
));

static MAGAZINE_CACHE: SyncUnsafeCell<SlabCache> = SyncUnsafeCell::new(SlabCache::new(
    &"/MagazineCache",
    Layout::new::<Magazine>(),
    Some(magazine_init),
    None,
));

fn for_each_linked_cache<F>(f: F)
where
    F: Fn(NonNull<SlabCache>),
{
    let root_cache = root_cache();
    let root_lists = root_cache.lists.lock();

    let mut cache = root_lists.next;
    while let Some(cache_ptr) = cache {
        f(cache_ptr);
        let lists = unsafe { cache_ptr.as_ref() }.lists.lock();

        cache = lists.next;
    }
}

#[inline(always)]
fn percpu_magazine_cache() -> &'static SlabCache {
    unsafe { &*PERCPU_MAGAZINE_CACHE.get() }
}

#[inline(always)]
fn indirect_cache() -> &'static SlabCache {
    unsafe { &*INDIRECT_SLAB_CACHE.get() }
}

#[inline(always)]
fn root_cache() -> &'static SlabCache {
    &ROOT_SLAB_CACHE
}

#[inline(always)]
fn magazine_cache() -> &'static SlabCache {
    unsafe { &*MAGAZINE_CACHE.get() }
}

#[inline(always)]
fn allocate_magazine() -> Result<NonNull<Magazine>, VMMAllocError> {
    magazine_cache().allocate().map(|p| p.cast::<Magazine>())
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
/// Represents a reference to a [`SlabCache`] allocated with [`slab_cache_create`].
pub struct SlabCacheRef(NonNull<SlabCache>);

impl Deref for SlabCacheRef {
    type Target = SlabCache;
    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

/// Creates a new slab cache and returns a reference to it.
///
/// `name` is the name of the cache.
///
/// `layout` is the memory layout of the objects within the cache.
///
/// `initializer` would be called before the object is ever returned to the caller on [`SlabCache::allocate`] and vise versa for deinitializer and [`SlabCache::free`].
pub fn slab_cache_create(
    name: &'static &'static str,
    layout: Layout,
    initializer: Option<SlabInitializer>,
    deinitializer: Option<SlabDeinitializer>,
) -> Result<SlabCacheRef, VMMAllocError> {
    let mut cache = root_cache().allocate()?.cast::<SlabCache>();
    unsafe {
        *cache.as_mut() = SlabCache::new(name, layout, initializer, deinitializer);
        if let Err(e) = cache.as_mut().try_init_magazines() {
            root_cache().free(cache.cast());
            return Err(e);
        }

        // Link up the cache in the list.
        let mut root_lists = root_cache().lists.lock();
        cache.as_mut().lists.get_mut().next = root_lists.next.take();
        root_lists.next = Some(cache);
    }

    let r = SlabCacheRef(cache);
    logging::debug!(
        SlabCache,
        "new cache created: {},flags={:?},slab_sz={:#x}b,obj_sz={:#x}b,obj_cnt={:#x},obj_align={:#x}b",
        r.name,
        r.flags,
        r.slab_sz,
        r.object_sz,
        r.object_cnt,
        r.object_align,
    );
    Ok(r)
}

/// Initializes the Slab Allocator.
///
/// Using it before that isn't exactly UB.
pub unsafe fn init() {
    unsafe {
        (*PERCPU_MAGAZINE_CACHE.get()) = SlabCache::new(
            &"/PerCpuMagazineCache",
            Layout::array::<PerCpuMagazineCache>(bootloader::cpu_count())
                .expect("womp womp layout too big blah blah blah"),
            Some(percpu_magazine_cache_init),
            None,
        );

        (*INDIRECT_SLAB_CACHE.get())
            .try_init_magazines()
            .expect("Failed to initialize magazines for indirect slab cache");

        for_each_linked_cache(|mut c| {
            c.as_mut()
                .try_init_magazines()
                .expect("Failed to init magazines")
        });
    }
}
