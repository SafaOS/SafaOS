use crate::{
    errors::IntoErr,
    ffi::{
        NotZeroable,
        slice::{InvalidSliceError, Slice},
    },
};

#[derive(Debug, Clone, Copy)]
/// Represents an FFI-safe alternative to [&str]
///
/// Has the same requirements as &[`str`], however these requirements may not be met if passed from a foreign callsite, so additional checks may be necessary.
#[repr(transparent)]
pub struct Str(Slice<u8>);

/// An error that occurs when attempting to convert a [`Str`] to a &[`str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidStrError {
    InvalidSliceError(InvalidSliceError),
    Utf8Error,
}

impl From<InvalidSliceError> for InvalidStrError {
    fn from(value: InvalidSliceError) -> Self {
        Self::InvalidSliceError(value)
    }
}

impl IntoErr for InvalidStrError {
    fn into_err(self) -> crate::errors::ErrorStatus {
        match self {
            Self::InvalidSliceError(e) => e.into_err(),
            Self::Utf8Error => crate::errors::ErrorStatus::InvalidStr,
        }
    }
}

impl Str {
    /// Creates a new [`Str`] from a string slice.
    pub const fn from_str(s: &str) -> Self {
        Self(Slice::from_slice(s.as_bytes()))
    }

    pub const fn as_bytes(&self) -> &Slice<u8> {
        &self.0
    }

    pub const fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }

    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Attempts to convert [`Str`] into a mutable string slice &[str].
    #[inline]
    pub unsafe fn try_as_str_mut_custom<'a>(
        &self,
        custom_validate: impl FnOnce(*const ()) -> bool,
    ) -> Result<&'a mut str, InvalidStrError> {
        unsafe {
            let byte_slice = self
                .0
                .try_as_slice_mut_custom(custom_validate)
                .map_err(|e| InvalidStrError::InvalidSliceError(e))?;
            core::str::from_utf8_mut(byte_slice).map_err(|_| InvalidStrError::Utf8Error)
        }
    }
    /// Attempts to convert [`Str`] into a string slice &[`str`].
    #[inline]
    pub unsafe fn try_as_str_custom<'a>(
        &self,
        custom_validate: impl FnOnce(*const ()) -> bool,
    ) -> Result<&'a str, InvalidStrError> {
        unsafe {
            match self.try_as_str_mut_custom(custom_validate) {
                Ok(str) => Ok(str),
                Err(e) => Err(e),
            }
        }
    }

    /// Attempts to convert [`Str`] into a string slice &[`str`].
    #[inline]
    pub unsafe fn try_as_str<'a>(&self) -> Result<&'a str, InvalidStrError> {
        unsafe {
            match self.try_as_str_custom(|_| true) {
                Ok(str) => Ok(str),
                Err(err) => Err(err),
            }
        }
    }

    /// Attempts to convert [`Str`] into a mutable string slice &mut [`str`].
    #[inline]
    pub unsafe fn try_as_str_mut<'a>(&self) -> Result<&'a mut str, InvalidStrError> {
        unsafe {
            match self.try_as_str_mut_custom(|_| true) {
                Ok(str) => Ok(str),
                Err(err) => Err(err),
            }
        }
    }
}

impl Slice<Str> {
    /// Converts a mutable slice of string slices [`*mut str`] into an FFI safe [`Slice`] of [`Str`]s.
    ///
    /// # Safety
    ///
    /// The given slice will be unsafely reused, for now the data will be left unchanged in the current rust version because the layout of [Str] is the same as &[str],
    /// However since the layout of slices isn't guaranteed yet by rust, this function may change the given buffer in the future.
    ///
    /// this should be solved if this [RFC](https://github.com/rust-lang/rfcs/pull/3775) got accepted
    #[inline]
    pub const unsafe fn from_str_slices_mut(slices: *mut [*mut str]) -> Self {
        let old_slices = unsafe { &mut *slices };
        let raw_slices = unsafe { &mut *(slices as *mut [Str]) };

        let mut i = 0;
        while i < old_slices.len() {
            let slice = old_slices[i];
            raw_slices[i] = unsafe { Str::from_str(&*slice) };
            i += 1;
        }

        Slice::from_slice(raw_slices)
    }

    /// Attempts to convert an FFI [Slice] of [`Str`]s into a rust slice of str slices *mut [*mut [`str`]].
    /// given an FFI [Slice] of [`Str`]s
    ///
    /// # Safety
    ///
    /// The given FFI slice will be unsafely reused, for now the data will be left unchanged in the current rust version because the layout of [Str] is the same as &[str],
    /// However since the layout of slices isn't guaranteed yet by rust, this function may change the given buffer in the future, or by some obscure optimizations.
    ///
    /// this should be solved if this [RFC](https://github.com/rust-lang/rfcs/pull/3775) got accepted
    #[inline]
    pub unsafe fn try_into_str_slices_mut<'a>(
        self,
        custom_validate: impl Fn(*const ()) -> bool,
    ) -> Result<*mut [&'a str], InvalidStrError> {
        let root = unsafe { self.try_as_slice_mut_custom(&custom_validate)? };
        let root_ptr = root as *mut _;
        let results = unsafe { &mut *(root_ptr as *mut [&'a str]) };

        let mut i = 0;
        while i < results.len() {
            let slice = &root[i];
            results[i] = unsafe { slice.try_as_str_custom(&custom_validate)? };
            i += 1;
        }

        Ok(results)
    }
}

impl NotZeroable for Str {
    #[inline(always)]
    fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
}
