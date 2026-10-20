use core::{
    alloc::Layout,
    cell::{Cell, UnsafeCell},
    mem::MaybeUninit,
    ptr::NonNull,
};

use alloc::alloc::{AllocError, Allocator, Global};

pub struct RawLocalArena<const N: usize, A: Allocator = Global> {
    // Data may be accessed by other threads.
    data: UnsafeCell<[MaybeUninit<u8>; N]>,
    alloc: A,
    // Only a single thread may modify these.
    next_arena_off: Cell<usize>,
    /// The amount of allocations currently live on the allocation.
    allocations: Cell<usize>,
    /// The amount of references to the arena itself that isn't allocations, the arena will be kept allocated as long as this isn't zero.
    arena_refs: Cell<usize>,
}

impl<const N: usize, A: Allocator> RawLocalArena<N, A> {
    fn try_allocate_in(alloc: A) -> Result<NonNull<Self>, AllocError> {
        let ptr = alloc.allocate(Layout::new::<Self>())?.cast::<Self>();
        unsafe {
            core::ptr::write(
                ptr.as_ptr(),
                Self {
                    data: UnsafeCell::new([MaybeUninit::uninit(); N]),
                    alloc,
                    next_arena_off: Cell::new(0),
                    allocations: Cell::new(0),
                    arena_refs: Cell::new(1),
                },
            );
        }

        Ok(ptr)
    }

    /// Allocates a given layout
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // Put out a minimal alignment and pads by alignment for more predictability
        let layout = layout
            .align_to(8)
            .map(|l| l.pad_to_align())
            .map_err(|_| AllocError)?;

        let arena_ptr = self.data.get().cast::<u8>();

        let possible_candaite = unsafe { arena_ptr.add(self.next_arena_off.get()) };
        let off = possible_candaite.align_offset(layout.align());

        let alloc_ptr = unsafe { possible_candaite.byte_add(off) };
        let alloc_end_ptr = unsafe { alloc_ptr.byte_add(layout.size()) };

        let alloc_off_from_arena = unsafe { alloc_end_ptr.offset_from_unsigned(arena_ptr) };
        if alloc_off_from_arena > N {
            return Err(AllocError);
        }

        self.next_arena_off.set(alloc_off_from_arena);
        self.allocations.set(self.allocations.get() + 1);

        Ok(unsafe {
            NonNull::slice_from_raw_parts(NonNull::new_unchecked(alloc_ptr), layout.size())
        })
    }

    unsafe fn deallocate_if_no_refs(this_ptr: NonNull<Self>) -> bool {
        let this = unsafe { this_ptr.as_ref() };

        if this.allocations.get() + this.arena_refs.get() == 0 {
            let alloc = unsafe { core::ptr::read(&this.alloc) };
            unsafe { alloc.deallocate(this_ptr.cast(), Layout::new::<Self>()) };
            true
        } else {
            false
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let layout = layout
            .align_to(8)
            .map(|l| l.pad_to_align())
            .expect("Invalid layout given to deallocate");

        let arena_ptr = self.data.get().cast::<u8>();
        let ptr = ptr.as_ptr();
        let end_ptr = unsafe { ptr.byte_add(layout.size()) };

        let arena_offset = unsafe { ptr.offset_from(arena_ptr) };
        let end_arena_offset = unsafe { end_ptr.offset_from(arena_ptr) };
        assert!(
            arena_offset >= 0 && arena_offset < N as isize,
            "deallocate() called with a pointer that is outside of arena"
        );
        assert!(
            end_arena_offset >= 0 && end_arena_offset <= N as isize,
            "deallocate() called with a pointer that is outside of arena"
        );

        if self.next_arena_off.get() == end_arena_offset as usize {
            self.next_arena_off.set(arena_offset as usize);
        }
        self.allocations.set(self.allocations.get() - 1);
    }
}

/// A local arena is a bump-allocator of sorts that is supposed to be thread local
///
/// Is only deallocated once all references to it are gone.
pub struct LocalArena<const N: usize, A: Allocator = Global> {
    arena: NonNull<RawLocalArena<N, A>>,
}

impl<const N: usize, A: Allocator> LocalArena<N, A> {
    pub const SIZE_OF_ARENA: usize = size_of::<RawLocalArena<N, A>>();

    /// Attempts to allocate a new arena in a given allocator
    #[inline(always)]
    pub fn try_new_in(alloc: A) -> Result<Self, AllocError> {
        let arena = RawLocalArena::try_allocate_in(alloc)?;
        Ok(Self { arena })
    }

    /// Allocates memory for a given layout.
    ///
    /// Returning a slice of the allocated memory.
    ///
    /// Currently the only way to deallocate is through [`deallocate`] you can get a pointer through [`Self::arena_ptr`].
    #[inline(always)]
    pub fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        unsafe { self.arena.as_ref().allocate(layout) }
    }

    /// Returns a pointer to the arena
    #[inline(always)]
    pub const fn arena_ptr(&self) -> NonNull<RawLocalArena<N, A>> {
        self.arena
    }
}

#[inline(always)]
/// Deallocates a pointer from a pointer to a given arena
pub unsafe fn deallocate<const N: usize, A: Allocator>(
    arena_ptr: NonNull<RawLocalArena<N, A>>,
    ptr: NonNull<u8>,
    layout: Layout,
) {
    unsafe {
        arena_ptr.as_ref().deallocate(ptr, layout);
        RawLocalArena::deallocate_if_no_refs(arena_ptr);
    }
}

impl<const N: usize, A: Allocator> Drop for LocalArena<N, A> {
    fn drop(&mut self) {
        let arena_ptr = self.arena;
        let arena = unsafe { arena_ptr.as_ref() };
        arena.arena_refs.set(arena.arena_refs.get() - 1);

        unsafe { RawLocalArena::deallocate_if_no_refs(arena_ptr) };
    }
}
