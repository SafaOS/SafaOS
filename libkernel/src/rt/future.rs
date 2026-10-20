use core::{pin::Pin, ptr::NonNull};

use alloc::{
    alloc::{AllocError, Allocator},
    boxed::Box,
};

unsafe extern "C" {
    fn SafaRT_curr_stack_ptr() -> NonNull<()>;
    fn SafaRT_stack_alloc(
        ptr_to_stack: NonNull<()>,
        size: usize,
        align: usize,
    ) -> Result<NonNull<u8>, ()>;
    fn SafaRT_stack_free(
        ptr_to_stack: NonNull<()>,
        data_ptr: NonNull<u8>,
        size: usize,
        align: usize,
    );
}

/// A kernel stack allocator, which allocates stuff on the current task's kernel stack.
///
/// Constructed once for every task.
///
/// Once constructed it cannot be sent to other threads, this won't cause UB because it would be caught by a panic, therefore this has a fake `Send` impl.
#[derive(Debug, Clone, Copy)]
pub struct KeStackAlloc {
    stack_ptr: NonNull<()>,
}

impl KeStackAlloc {
    #[inline(always)]
    /// Returns the Allocator instance for the current task.
    pub fn new() -> Self {
        Self {
            stack_ptr: unsafe { SafaRT_curr_stack_ptr() },
        }
    }
}

unsafe impl Allocator for KeStackAlloc {
    #[inline(always)]
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        unsafe {
            SafaRT_stack_alloc(self.stack_ptr, layout.size(), layout.align())
                .map(|p| NonNull::slice_from_raw_parts(p, layout.size()))
                .map_err(|()| AllocError)
        }
    }

    #[inline(always)]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        unsafe {
            SafaRT_stack_free(self.stack_ptr, ptr, layout.size(), layout.align());
        }
    }
}

/// Fake Send impl, if any value was sent to another thread, it is going to be caught by the allocate/deallocate checks (will panic)
unsafe impl Send for KeStackAlloc {}

pin_project_lite::pin_project! {
    #[repr(C)]
    /// A dynamic future call.
    ///
    /// Currently implemented as a dynamic per-task arena allocation, and thus this represents an entry in the future stack of the task.
    ///
    /// This value cannot be sent across threads or it will panic.
    ///
    /// Prefer popping the value off the stack in LILO order this will help with deallocation.
    pub struct DynFut<'a, T> {
        // FIXME: We want to make that Box not Pin so we could move the future out to a heap allocation or such
        // And this isn't actually repr(C) too
        #[pin]
        boxed: Pin<Box<dyn Future<Output = T> + Send + 'a, KeStackAlloc>>,
    }
}

impl<'a, T> DynFut<'a, T> {
    /// Constructs a new dynamic future.
    ///
    /// Panicks if this fails as opposite to [`Self::try_new`].
    pub fn new<F>(fut: F) -> Self
    where
        F: Future<Output = T> + Send + 'a,
    {
        Self::try_new(fut).expect("Failed to allocate for a future (stack overflow)")
    }

    #[inline]
    pub fn try_new<F>(fut: F) -> Result<Self, AllocError>
    where
        F: Future<Output = T> + Send + 'a,
    {
        Box::try_new_in(fut, KeStackAlloc::new()).map(|stack| Self {
            boxed: Box::into_pin(stack),
        })
    }
}

impl<'a, T> Future for DynFut<'a, T> {
    type Output = T;
    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        self.project().boxed.poll(cx)
    }
}
