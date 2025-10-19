use crate::ffi::NotZeroable;

/// Represents a non-null raw pointer, similar to [`core::ptr::NonNull`], but it instead doesn't guarantee that the pointer is valid,
/// instead IT SHOULD BE valid, additional checks should be performed before using it.
///
/// The purpose is for use alongside [`crate::ffi::option::OptZero`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct FFINonNull<T: ?Sized>(pub(crate) *mut T);

impl<T: ?Sized> FFINonNull<T> {
    /// Creates a new instance of [`FFINonNull`] if the pointer is not null otherwise returns [`None`].
    pub const fn new(ptr: *mut T) -> Option<Self> {
        if ptr.is_null() { None } else { Some(Self(ptr)) }
    }

    /// Creates a new instance of [`FFINonNull`] without checking if the pointer is null.
    /// # Safety
    /// The caller must ensure that the pointer is not null.
    pub const unsafe fn new_unchecked(ptr: *mut T) -> Self {
        debug_assert!(
            !ptr.is_null(),
            "attempt to create an instance of FFINonNull with a null value"
        );
        Self(ptr)
    }
    /// Returns the raw pointer stored in this [`FFINonNull`] instance.
    pub const fn as_ptr(&self) -> *mut T {
        self.0
    }
}

impl<T: ?Sized> NotZeroable for FFINonNull<T> {
    fn is_zero(&self) -> bool {
        self.0.is_null()
    }
}
