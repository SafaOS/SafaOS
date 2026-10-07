use core::{pin::Pin, ptr::NonNull};

use crate::rt::Waiter;

#[repr(u16)]
pub enum IoOp {
    ReadNext,
    WriteNext,
}

#[repr(C)]
pub enum IoBuffer {
    Single(NonNull<[u8]>),
    Multiple(NonNull<[NonNull<[u8]>]>),
}

impl IoBuffer {
    pub fn from_owned_buf(buf: alloc::boxed::Box<[u8]>) -> Self {
        Self::Single(alloc::boxed::Box::into_non_null(buf))
    }
    pub fn as_slices_mut<'a>(&'a mut self) -> impl Iterator<Item = &'a mut [u8]> {
        todo!();
        core::iter::empty()
    }
}
enum IOResponse {
    Success(usize),
    Err(usize),
    Cancalled,
}

struct IOFuture {
    // Set by driver
    sub_id: usize,
    op: IoOp,
    buffer: IoBuffer,

    waiter: Waiter,
}

trait IOHandle {
    fn cancel_io(&self, future: Pin<&mut IOFuture>) -> core::task::Poll<()>;
    fn poll_io(&self, future: Pin<&mut IOFuture>) -> core::task::Poll<IOResponse>;
}
