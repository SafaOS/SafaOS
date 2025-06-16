use crate::{
    memory::{
        frame_allocator::{self, Frame},
        paging::PAGE_SIZE,
    },
    PhysAddr,
};

// FIXME: make a DMA allocator that doesn't waste memory like this
/// uses the given frame as a place to store an allocated list of T with length `len`
/// length must be less then 4096 / size_of::<T>()
/// allocated area is zero initialized
pub fn allocate_buffers_frame<'a, T: Clone>(
    frame: Frame,
    offset: usize,
    len: usize,
) -> (&'a mut [T], PhysAddr) {
    assert!(len / size_of::<T>() <= PAGE_SIZE - offset);
    let virt_addr = frame.virt_addr() + offset;
    let phys_addr = frame.phys_addr() + offset;
    let slice_ptr = virt_addr.into_ptr::<T>();
    let slice = unsafe { core::slice::from_raw_parts_mut(slice_ptr, len) };
    slice.fill(unsafe { core::mem::zeroed() });
    (slice, phys_addr)
}

// FIXME: make a DMA allocator that doesn't waste memory like this
/// allocates a frame then calls [`allocate_buffers_frame`] on it
/// returns None if frame allocation failed
pub fn allocate_buffers<'a, T: Clone>(len: usize) -> Option<(&'a mut [T], PhysAddr)> {
    frame_allocator::allocate_frame().map(|frame| allocate_buffers_frame(frame, 0, len))
}
