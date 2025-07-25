use crate::{
    errors::{ErrorStatus, IntoErr},
    ffi::NotZeroable,
};

/// Represents an error that can occur when converting a RawSlice<T> to a Rust slice &[T].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSliceError {
    PtrNotAligned,
    PtrIsNull,
    LenTooLarge,
    Other,
}

impl IntoErr for InvalidSliceError {
    fn into_err(self) -> ErrorStatus {
        match self {
            InvalidSliceError::LenTooLarge => ErrorStatus::StrTooLong,
            _ => ErrorStatus::InvalidPtr,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// A C compatible slice of type `T`
pub struct Slice<T> {
    pub(crate) ptr: *mut T,
    pub(crate) len: usize,
}

impl<T> Slice<T> {
    #[inline]
    pub const unsafe fn from_raw_parts(ptr: *mut T, len: usize) -> Self {
        assert!(
            !ptr.is_null() && len <= isize::MAX as usize,
            "RawSlice::from_raw_parts requires ptr to be aligned and non-null, and len must be less than or equal to isize::MAX"
        );
        Self { ptr, len }
    }

    #[inline]
    pub const fn from_slice_mut(slice: &mut [T]) -> Self {
        Self {
            ptr: slice.as_mut_ptr(),
            len: slice.len(),
        }
    }

    #[inline]
    pub const fn from_slice(slice: &[T]) -> Self {
        Self {
            ptr: slice.as_ptr().cast_mut(),
            len: slice.len(),
        }
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub const fn as_ptr(&self) -> *const T {
        self.ptr
    }

    /// Converts an FFI [`RawSlice<T>`] into a raw rust slice of type `T`
    #[inline]
    pub const unsafe fn as_slice_mut_unchecked<'a>(&self) -> &'a mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    #[inline]
    pub const unsafe fn as_slice_unchecked<'a>(&self) -> &'a [T] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Attempts to convert an FFI [`RawSlice<T>`] into a raw rust slice of type `T`, running the pointer into a custom validator function
    ///
    /// returns Err(InvalidSliceError) if the slice doesn't pass the rust slice requirements or returns Err(InvalidSliceError::Other) if the validator function returns false
    #[inline]
    pub unsafe fn try_as_slice_mut_custom<'a>(
        &self,
        validator: impl FnOnce(*const ()) -> bool,
    ) -> Result<&'a mut [T], InvalidSliceError> {
        if self.ptr.is_null() {
            return Err(InvalidSliceError::PtrIsNull);
        }

        if !self.ptr.is_aligned() {
            return Err(InvalidSliceError::PtrNotAligned);
        }

        if self.len > isize::MAX as usize {
            return Err(InvalidSliceError::LenTooLarge);
        }

        if !validator(self.ptr.cast()) {
            return Err(InvalidSliceError::Other);
        }

        Ok(unsafe { self.as_slice_mut_unchecked() })
    }

    /// Attempts to convert an FFI [`RawSlice<T>`] into a raw rust slice of type `T`, running the pointer into a custom validator function
    ///
    /// returns Err(InvalidSliceError) if the slice doesn't pass the rust slice requirements or returns Err(InvalidSliceError::Other) if the validator function returns false
    #[inline]
    pub unsafe fn try_as_slice_custom<'a>(
        &self,
        validator: impl Fn(*const ()) -> bool,
    ) -> Result<&'a [T], InvalidSliceError> {
        unsafe {
            match self.try_as_slice_mut_custom(validator) {
                Ok(slice) => Ok(slice),
                Err(err) => Err(err),
            }
        }
    }
}

impl<T> Slice<Slice<T>> {
    /// Converts an FFI [Slice] of slices to a Rust slice of slices of type `T` *mut [*mut [T]].
    ///
    /// # Safety
    ///
    /// The given slice will be unsafely reused, for now the data will be left unchanged in the current rust version because the layout of [Slice<T>] is the same as &`[T]`,
    /// However since the layout of slices isn't guaranteed yet by rust, this function may change the given buffer in the future, or by some obscure optimizations.
    ///
    /// this should be solved if this [RFC](https://github.com/rust-lang/rfcs/pull/3775) got accepted
    #[inline]
    pub const unsafe fn from_slices_ptr_mut(slices: *mut [*mut [T]]) -> Self {
        let old_slices = unsafe { &mut *slices };
        let raw_slices = unsafe { &mut *(slices as *mut [Slice<T>]) };

        let mut i = 0;
        while i < old_slices.len() {
            let slice = old_slices[i];
            raw_slices[i] = unsafe { Slice::from_slice(&*slice) };
            i += 1;
        }

        Slice::from_slice(raw_slices)
    }

    /// Attempts to convert an FFI [Slice] of [Slice]s into a rust slice of slices *mut `[*mut [T]]`.
    /// given an FFI [Slice] of [Slice]s
    ///
    /// # Safety
    ///
    /// The given FFI slice will be unsafely reused, for now the data will be left unchanged in the current rust version because the layout of [Slice<T>] is the same as &`[T]`,
    /// However since the layout of slices isn't guaranteed yet by rust, this function may change the given buffer in the future, or by some obscure optimizations.
    ///
    /// this should be solved if this [RFC](https://github.com/rust-lang/rfcs/pull/3775) got accepted
    #[inline]
    pub unsafe fn try_into_slices_ptr_mut(
        self,
        custom_validate: impl Fn(*const ()) -> bool,
    ) -> Result<*mut [*mut [T]], InvalidSliceError> {
        let root = unsafe { self.try_as_slice_mut_custom(&custom_validate)? };
        let root_ptr = root as *mut _;
        let results = unsafe { &mut *(root_ptr as *mut [*mut [T]]) };

        let mut i = 0;
        while i < results.len() {
            let slice = &root[i];
            results[i] = unsafe { slice.try_as_slice_mut_custom(&custom_validate)? };
            i += 1;
        }

        Ok(results)
    }
}

impl<T> NotZeroable for Slice<T> {
    #[inline(always)]
    fn is_zero(&self) -> bool {
        self.ptr.is_null()
    }
}
