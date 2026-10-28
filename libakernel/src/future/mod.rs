use core::{
    marker::PhantomData,
    mem::{ManuallyDrop, MaybeUninit},
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll, RawWakerVTable, Waker},
};

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

const STACK_NEEDED: usize = 0x18;
const _: () = {
    assert!(
        STACK_NEEDED == size_of::<Box<dyn Future<Output = ()> + Send + 'static, KeStackAlloc>>()
    )
};

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
        #[pin]
        stack: StackFuture<'a, T, { STACK_NEEDED }>,
    }
}

// Ensures layout didn't change
// FIXME: make our own FFI safe future, this isn't even actually FFI safe...
const _: () = assert!(size_of::<DynFut<()>>() == 40);

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
            stack: StackFuture::from(Box::into_pin(stack)),
        })
    }
}

impl<'a, T> Future for DynFut<'a, T> {
    type Output = T;
    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        self.project().stack.poll(cx)
    }
}

pin_project_lite::pin_project! {
    #[repr(C)]
    /// Equivalent to [`DynFut`] but can be sent to other threads. (may be costy to create and allocate).
    pub struct SendDynFut<'a, T> {
        #[pin]
        stack: StackFuture<'a, T, { STACK_NEEDED }>,
    }
}

impl<'a, T> SendDynFut<'a, T> {
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
        Box::try_new(fut).map(|boxed| Self {
            stack: StackFuture::from(Box::into_pin(boxed)),
        })
    }

    /// Returns self as [`Pin`]
    pub fn into_pin(self) -> Pin<Self> {
        // Safety: It is not safe to move stuff out of Self anyways and we store a Pin<Box> inside
        unsafe { Pin::new_unchecked(self) }
    }
}

impl<'a, T> Future for SendDynFut<'a, T> {
    type Output = T;
    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        self.project().stack.poll(cx)
    }
}

impl<'a, T> core::ops::Deref for SendDynFut<'a, T> {
    // TODO: is that actually sound?
    //
    // Deref and DerefMut impls are for [`Pin::new_unchecked`] to work.
    type Target = StackFuture<'a, T, { STACK_NEEDED }>;
    fn deref(&self) -> &Self::Target {
        &self.stack
    }
}

impl<'a, T> core::ops::DerefMut for SendDynFut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stack
    }
}

#[repr(C)]
/// Modified FFI friendly version of the stackfuture crate.
pub struct StackFuture<'a, T, const STACK_SIZE: usize> {
    /// An array of bytes that is used to store the wrapped future.
    data: [MaybeUninit<u8>; STACK_SIZE],
    // TODO: save 1 pointer size by saving `poll_fn` and `drop_fn`.
    /// Since the type of `StackFuture` does not know the underlying future that it is wrapping,
    /// we keep a manual vtable that serves pointers to Poll::poll and Drop::drop. These are
    /// generated and filled in by `StackFuture::from`.
    ///
    /// This field stores a pointer to the poll function wrapper.
    poll_fn: extern "C" fn(this: Pin<&mut Self>, cx: &mut FFIContext<'_>) -> FFIPoll<T>,
    /// Stores a pointer to the drop function wrapper
    ///
    /// See the documentation on `poll_fn` for more details.
    drop_fn: extern "C" fn(this: &mut Self),
    /// StackFuture can be used similarly to a `dyn Future`. We keep a PhantomData
    /// here so the type system knows this.
    _phantom: PhantomData<dyn Future<Output = T> + Send + 'a>,
}

impl<'a, T, const STACK_SIZE: usize> StackFuture<'a, T, STACK_SIZE> {
    pub fn from<F>(future: F) -> Self
    where
        F: Future<Output = T> + Send + 'a,
    {
        struct AssertFits<F, const STACK_SIZE: usize>(PhantomData<F>);

        impl<F, const STACK_SIZE: usize> AssertFits<F, STACK_SIZE> {
            const ASSERT: () = {
                assert!(
                    StackFuture::<F, STACK_SIZE>::has_space_for::<F>(),
                    "Future is too large"
                );

                assert!(
                    StackFuture::<F, STACK_SIZE>::has_alignment_for::<F>(),
                    "Future has incompatible alignment"
                );
            };
        }
        #[allow(clippy::let_unit_value)]
        let _ = AssertFits::<F, STACK_SIZE>::ASSERT;

        Self::try_from(future).unwrap()
    }
    pub fn try_from<F>(future: F) -> Result<Self, ()>
    where
        F: Future<Output = T> + Send + 'a,
    {
        if Self::has_space_for_val(&future) && Self::has_alignment_for_val(&future) {
            let mut result = StackFuture {
                data: [MaybeUninit::uninit(); STACK_SIZE],
                poll_fn: Self::poll_inner::<F>,
                drop_fn: Self::drop_inner::<F>,
                _phantom: PhantomData,
            };

            assert_eq!(result.data.as_ptr() as usize, &result as *const _ as usize);

            // SAFETY: result.as_mut_ptr returns a pointer into result.data, which is an
            // uninitialized array of bytes. result.as_mut_ptr ensures the returned pointer
            // is correctly aligned, and the if expression we are in ensures the buffer is
            // large enough.
            //
            // Because `future` is bound by `'a` and `StackFuture` is also bound by `'a`,
            // we can be sure anything that `future` closes over will also outlive `result`.
            unsafe { result.as_mut_ptr::<F>().write(future) };

            Ok(result)
        } else {
            Err(())
        }
    }

    /// A wrapper around the inner future's poll function, which we store in the poll_fn field
    /// of this struct.
    extern "C" fn poll_inner<F: Future>(
        self: Pin<&mut Self>,
        cx: &mut FFIContext<'_>,
    ) -> FFIPoll<F::Output> {
        match cx.with_std(|cx| self.as_pin_mut_ref::<F>().poll(cx)) {
            Poll::Ready(r) => FFIPoll::Ready(r),
            Poll::Pending => FFIPoll::Pending,
        }
    }

    /// A wrapper around the inner future's drop function, which we store in the drop_fn field
    /// of this struct.
    extern "C" fn drop_inner<F>(&mut self) {
        // SAFETY: *this.as_mut_ptr() was previously written as type F
        unsafe { core::ptr::drop_in_place(self.as_mut_ptr::<F>()) }
    }

    fn as_mut_ptr<F>(&mut self) -> *mut F {
        assert!(Self::has_space_for::<F>());
        // SAFETY: Self is laid out so that the space for the future comes at offset 0.
        // This is checked by an assertion in Self::from. Thus it's safe to cast a pointer
        // to Self into a pointer to the wrapped future.
        unsafe { core::mem::transmute(self) }
    }

    fn as_pin_mut_ref<F>(self: Pin<&mut Self>) -> Pin<&mut F> {
        // SAFETY: `StackFuture` is only created by `StackFuture::from`, which
        // writes an `F` to `self.as_mut_ptr(), so it's okay to cast the `*mut F`
        // to an `&mut F` with the same lifetime as `self`.
        //
        // For pinning, since self is already pinned, we know the wrapped future
        // is also pinned.
        //
        // This function is only doing pointer arithmetic and casts, so we aren't moving
        // any pinned data.
        unsafe { self.map_unchecked_mut(|this| &mut *this.as_mut_ptr()) }
    }

    /// Computes how much space is required to store a value of type `F`
    const fn required_space<F>() -> usize {
        core::mem::size_of::<F>()
    }

    /// Determines whether this `StackFuture` can hold a value of type `F`
    pub const fn has_space_for<F>() -> bool {
        Self::required_space::<F>() <= STACK_SIZE
    }

    /// Determines whether this `StackFuture` can hold the referenced value
    pub const fn has_space_for_val<F>(_: &F) -> bool {
        Self::has_space_for::<F>()
    }

    /// Determines whether this `StackFuture`'s alignment is compatible with the
    /// type `F`.
    pub const fn has_alignment_for<F>() -> bool {
        core::mem::align_of::<F>() <= core::mem::align_of::<Self>()
    }

    /// Determines whether this `StackFuture`'s alignment is compatible with the
    /// referenced value.
    pub const fn has_alignment_for_val<F>(_: &F) -> bool {
        Self::has_alignment_for::<F>()
    }
}

impl<'a, T, const STACK_SIZE: usize> Future for StackFuture<'a, T, { STACK_SIZE }> {
    type Output = T;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: This is doing pin projection. We unpin self so we can
        // access self.poll_fn, and then re-pin self to pass it into poll_in.
        // The part of the struct that needs to be pinned is data, since it
        // contains a potentially self-referential future object, but since we
        // do not touch that while self is unpinned and we do not move self
        // while unpinned we are okay.
        unsafe {
            let this = self.get_unchecked_mut();
            match (this.poll_fn)(Pin::new_unchecked(this), &mut FFIContext::from_std(cx)) {
                FFIPoll::Pending => Poll::Pending,
                FFIPoll::Ready(r) => Poll::Ready(r),
            }
        }
    }
}

impl<'a, T, const STACK_SIZE: usize> Drop for StackFuture<'a, T, { STACK_SIZE }> {
    fn drop(&mut self) {
        (self.drop_fn)(self);
    }
}

// ======== Really funky not so sound FFI async impl ==========
// FIXME: FFI AsYnC.
#[repr(u8)]
pub enum FFIPoll<T> {
    Ready(T),
    Pending,
}

#[repr(C)]
struct FFIWaker {
    data: *const (),
    /// A pointer to the vtable
    ///
    /// This is the same as the &'static RawWakerVtable, which stores rust fn pointers,
    /// which aren't really FFI safe...
    /// but this is enough to convince the compiler.
    vtable: *const (),
}

impl FFIWaker {
    fn borrow(&self) -> ManuallyDrop<Waker> {
        ManuallyDrop::new(unsafe { Waker::new(self.data, &*self.vtable.cast::<RawWakerVTable>()) })
    }
}
impl Drop for FFIWaker {
    fn drop(&mut self) {
        drop(ManuallyDrop::into_inner(self.borrow()));
    }
}
unsafe impl Send for FFIWaker {}
unsafe impl Sync for FFIWaker {}

#[repr(C)]
struct FFIContext<'a> {
    waker: ManuallyDrop<FFIWaker>,
    _m: PhantomData<&'a Waker>,
}

impl<'a> FFIContext<'a> {
    pub fn from_std(cx: &'a Context<'_>) -> Self {
        let w = cx.waker();
        Self {
            waker: ManuallyDrop::new(FFIWaker {
                data: w.data(),
                vtable: w.vtable() as *const RawWakerVTable as *const (),
            }),
            _m: PhantomData,
        }
    }

    pub fn with_std<R>(&mut self, f: impl FnOnce(&mut Context<'_>) -> R) -> R {
        let w = self.waker.borrow();
        f(&mut Context::from_waker(&w))
    }
}
