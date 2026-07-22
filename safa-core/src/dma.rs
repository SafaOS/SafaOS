use core::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use crate::{
    PhysAddr, VirtAddr,
    memory::{
        frame_allocator::{self, Frame, FrameIter},
        paging::PAGE_SIZE,
        vmm::{self, Location, VMMMFlags},
    },
};

#[derive(Debug)]
/// Describes a buffer allocated for DMA with a fixed capacity.
pub struct DMABuffer<T> {
    start: Frame,
    data: NonNull<[MaybeUninit<T>]>,
    len: usize,
}

unsafe impl<T> Send for DMABuffer<T> {}
unsafe impl<T> Sync for DMABuffer<T> {}

impl<T> DMABuffer<T> {
    /// Attempts to allocate a buffer with given count and fill it with `element`, returns an error if memory allocation fails.
    pub fn new_filled(count: usize, element: T) -> Result<Self, ()>
    where
        T: Clone,
    {
        Self::new(count).map(|mut r| {
            unsafe {
                r.len = count;
                r.data.as_mut()[..count].fill_with(|| MaybeUninit::new(element.clone()));
            }
            r
        })
    }

    /// Attempts to allocate a DMA buffer of capacity `cap`, returning an Error if memory allocation fails.
    pub fn new(cap: usize) -> Result<Self, ()> {
        const {
            assert!(
                size_of::<T>() > 0,
                "Cannot allocate a DMA Buffer for a zero-sized type"
            );
        }
        let cap_bytes = size_of::<T>() * cap;
        let pages = cap_bytes.div_ceil(PAGE_SIZE);
        let real_cap = (pages * PAGE_SIZE) / size_of::<T>();

        let (start, end) = frame_allocator::allocate_contiguous(1, pages).ok_or(())?;

        vmm::with_root(|vmm| {
            let addr = vmm
                .map_direct_phys(
                    &"DMA",
                    Some(Location::Hint(VirtAddr::null())),
                    start.phys_addr(),
                    pages,
                    VMMMFlags::WRITEABLE | VMMMFlags::UNCACHABLE,
                )
                .map_err(|_| {
                    for frame in Frame::iter_frames(
                        start,
                        Frame::containing_address(end.phys_addr() + PAGE_SIZE),
                    ) {
                        frame_allocator::deallocate_frame(frame);
                    }
                    ()
                })?;

            assert_ne!(addr, VirtAddr::null());

            Ok(Self {
                start,
                data: NonNull::slice_from_raw_parts(
                    NonNull::new(addr.into_ptr()).unwrap(),
                    real_cap,
                ),
                len: 0,
            })
        })
    }

    pub fn phys(&self) -> PhysAddr {
        self.start.phys_addr()
    }

    pub fn raw_frames(&self) -> FrameIter {
        Frame::iter_frames(
            self.start,
            Frame::containing_address(
                self.start.phys_addr()
                    + (PAGE_SIZE * (self.data.len() * size_of::<T>()).div_ceil(PAGE_SIZE)),
            ),
        )
    }

    pub fn as_slice(&self) -> &[T] {
        unsafe { self.data.as_ref()[..self.len()].assume_init_ref() }
    }

    pub fn as_slice_mut(&mut self) -> &mut [T] {
        unsafe { self.data.as_mut()[..self.len()].assume_init_mut() }
    }

    #[allow(unused)]
    pub const fn capacity(&self) -> usize {
        self.data.len()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    #[allow(unused)]
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }

        self.len -= 1;
        Some(unsafe {
            core::mem::replace(&mut self.data.as_mut()[self.len], MaybeUninit::uninit())
                .assume_init()
        })
    }

    pub fn push(&mut self, item: T) -> Result<(), T> {
        if let Some(slot) = unsafe { self.data.as_mut().get_mut(self.len) } {
            *slot = MaybeUninit::new(item);
            self.len += 1;
            Ok(())
        } else {
            Err(item)
        }
    }
}

impl<T> Drop for DMABuffer<T> {
    fn drop(&mut self) {
        for item in unsafe { &mut self.data.as_mut()[..self.len()] } {
            unsafe { item.assume_init_drop() };
        }

        vmm::with_root(|vmm| {
            assert!(
                vmm.unmap(VirtAddr::from_ptr(self.data.cast::<u8>().as_ptr())),
                "DMABuffer wasn't pointing to allocated memory"
            )
        });

        for frame in self.raw_frames() {
            frame_allocator::deallocate_frame(frame);
        }
    }
}

impl<T> Deref for DMABuffer<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T> DerefMut for DMABuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_slice_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ops::{Deref, DerefMut};
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    /// Helper type that records how many times it has been dropped, so we
    /// can verify DMABuffer's manual MaybeUninit bookkeeping (push/pop/Drop)
    /// neither leaks nor double-drops elements.
    struct DropCounter<'a> {
        counter: &'a AtomicUsize,
    }

    impl<'a> Drop for DropCounter<'a> {
        fn drop(&mut self) {
            self.counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl<'a> Clone for DropCounter<'a> {
        fn clone(&self) -> Self {
            DropCounter {
                counter: self.counter,
            }
        }
    }

    #[test_case]
    fn new_allocates_empty_buffer() {
        let buf: DMABuffer<u64> = DMABuffer::new(10).expect("allocation failed");
        assert_eq!(buf.len(), 0);
        assert!(buf.capacity() >= 10);
    }

    #[test_case]
    fn new_capacity_is_rounded_up_to_a_full_page() {
        // Requesting fewer elements than fit in one page should still
        // yield a buffer whose capacity spans exactly one page.
        let buf: DMABuffer<u8> = DMABuffer::new(1).expect("allocation failed");
        assert_eq!(buf.capacity(), PAGE_SIZE);
    }

    #[test_case]
    fn new_with_zero_capacity_is_allowed() {
        let buf: DMABuffer<u32> = DMABuffer::new(0).expect("allocation failed");
        assert_eq!(buf.len(), 0);
        // pages = ceil(0 / PAGE_SIZE) = 0, so capacity should be 0 too.
        assert_eq!(buf.capacity(), 0);
    }

    #[test_case]
    fn new_with_fills_buffer_and_sets_len() {
        let buf = DMABuffer::new_filled(5, 42u32).expect("allocation failed");
        assert_eq!(buf.len(), 5);
        assert_eq!(&*buf, &[42, 42, 42, 42, 42]);
    }

    #[test_case]
    fn new_with_clones_are_independent() {
        #[derive(Clone, PartialEq, Debug)]
        struct Wrapper(u32);

        let mut buf = DMABuffer::new_filled(3, Wrapper(7)).expect("allocation failed");
        buf[0] = Wrapper(99);
        assert_eq!(buf[1], Wrapper(7));
        assert_eq!(buf[2], Wrapper(7));
    }

    #[test_case]
    fn push_and_pop_are_lifo() {
        let mut buf: DMABuffer<u32> = DMABuffer::new(4).unwrap();
        assert!(buf.push(1).is_ok());
        assert!(buf.push(2).is_ok());
        assert!(buf.push(3).is_ok());
        assert_eq!(buf.len(), 3);

        assert_eq!(buf.pop(), Some(3));
        assert_eq!(buf.pop(), Some(2));
        assert_eq!(buf.pop(), Some(1));
        assert_eq!(buf.pop(), None);
        assert_eq!(buf.len(), 0);
    }

    #[test_case]
    fn pop_on_empty_buffer_returns_none() {
        let mut buf: DMABuffer<u32> = DMABuffer::new(4).unwrap();
        assert_eq!(buf.pop(), None);
    }

    #[test_case]
    fn push_fails_once_full_and_returns_the_item_back() {
        let mut buf: DMABuffer<u32> = DMABuffer::new(1).unwrap();
        // capacity is rounded up to a full page's worth of elements, so
        // drain it before expecting failure.
        while buf.push(0).is_ok() {}

        match buf.push(999) {
            Err(item) => assert_eq!(item, 999),
            Ok(()) => panic!("push should fail once the buffer is at capacity"),
        }
    }

    #[test_case]
    fn drop_runs_destructors_for_every_live_element() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);
        {
            let mut buf: DMABuffer<DropCounter> = DMABuffer::new(4).unwrap();
            buf.push(DropCounter { counter: &COUNTER }).unwrap();
            buf.push(DropCounter { counter: &COUNTER }).unwrap();
            buf.push(DropCounter { counter: &COUNTER }).unwrap();
            assert_eq!(COUNTER.load(Ordering::SeqCst), 0);
        }
        assert_eq!(COUNTER.load(Ordering::SeqCst), 3);
    }

    #[test_case]
    fn drop_ignores_uninitialized_tail_capacity() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);
        {
            let mut buf: DMABuffer<DropCounter> = DMABuffer::new(8).unwrap();
            // Only push one element; capacity is much larger. If Drop
            // iterated over `capacity()` instead of `len()`, this would
            // read/drop uninitialized memory.
            buf.push(DropCounter { counter: &COUNTER }).unwrap();
        }
        assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
    }

    #[test_case]
    fn pop_transfers_ownership_without_double_drop() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);
        {
            let mut buf: DMABuffer<DropCounter> = DMABuffer::new(2).unwrap();
            buf.push(DropCounter { counter: &COUNTER }).unwrap();

            let item = buf.pop().unwrap();
            assert_eq!(COUNTER.load(Ordering::SeqCst), 0, "pop must not drop");
            drop(item);
            assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
        }
        // buf.len() was 0 by the time buf itself dropped, so no further
        // drops should occur (would show up as COUNTER == 2, a bug).
        assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
    }

    #[test_case]
    fn deref_valid() {
        let mut buf: DMABuffer<u32> = DMABuffer::new(10).unwrap();
        buf.push(1).unwrap();
        buf.push(2).unwrap();
        assert_eq!(&*buf, &[1, 2]);
        assert_eq!(buf.deref().len(), buf.len());

        let mut buf: DMABuffer<u32> = DMABuffer::new(4).unwrap();
        buf.push(1).unwrap();
        buf.push(2).unwrap();
        buf.deref_mut()[0] = 100;
        assert_eq!(&*buf, &[100, 2]);
    }

    #[test_case]
    fn as_slice_and_as_slice_mut_agree_with_deref() {
        let mut buf: DMABuffer<u32> = DMABuffer::new(4).unwrap();
        buf.push(10).unwrap();
        buf.push(20).unwrap();
        assert_eq!(buf.as_slice(), &*buf);
        buf.as_slice_mut()[0] = 999;
        assert_eq!(buf[0], 999);
    }

    #[test_case]
    fn phys_alloc_valid() {
        let buf: DMABuffer<u32> = DMABuffer::new(4).unwrap();
        assert_eq!(buf.phys().into_raw() % PAGE_SIZE, 0);

        // Ask for exactly 3 pages' worth of u8 elements.
        let buf: DMABuffer<u8> = DMABuffer::new(PAGE_SIZE * 3).unwrap();
        assert_eq!(buf.raw_frames().count(), 3);
    }

    #[test_case]
    fn multiple_buffers_do_not_alias() {
        let mut a: DMABuffer<u32> = DMABuffer::new(4).unwrap();
        let mut b: DMABuffer<u32> = DMABuffer::new(4).unwrap();

        a.push(1).unwrap();
        b.push(2).unwrap();

        assert_eq!(a[0], 1);
        assert_eq!(b[0], 2);
        assert_ne!(a.phys(), b.phys());
    }
}
