use safa_abi::{
    errors::{ErrorStatus, IntoErr},
    ffi::{NotZeroable, option::OptZero, ptr::FFINonNull, slice::Slice, str::Str},
};

use crate::syscalls::ffi::{ptr_is_allowed, ptr_is_valid};

/// Describes a transmute operation from a foreign type to a Rust type
pub trait ForeignTryAccept<'a, ResultType: 'a>: Sized {
    /// Attempts to accept a foreign type [`Self`] as [`ResultType`], returning an error if it fails
    ///
    /// # Safety
    /// I am not sure whether or not this is safe, from the kernel side after this, it would cause no UB to use this,
    /// you may get page faults and so but this will only effect the userspace process,
    fn try_accept(self) -> Result<ResultType, ErrorStatus>;
}

impl<'a, T: 'a> ForeignTryAccept<'a, &'a mut T> for FFINonNull<T> {
    fn try_accept(self) -> Result<&'a mut T, ErrorStatus> {
        let ptr = self.as_ptr();
        if !ptr_is_valid(ptr) {
            return Err(ErrorStatus::InvalidPtr);
        }

        unsafe { Ok(&mut *ptr) }
    }
}

impl<'a, T: 'a> ForeignTryAccept<'a, &'a T> for FFINonNull<T> {
    fn try_accept(self) -> Result<&'a T, ErrorStatus> {
        let ptr = self.as_ptr();
        if !ptr_is_valid(ptr) {
            return Err(ErrorStatus::InvalidPtr);
        }

        unsafe { Ok(&*ptr) }
    }
}

impl<'a, T: 'a> ForeignTryAccept<'a, *mut [*mut [T]]> for Slice<Slice<T>> {
    fn try_accept(self) -> Result<*mut [*mut [T]], ErrorStatus> {
        unsafe {
            self.try_into_slices_ptr_mut(ptr_is_allowed)
                .map_err(|e| e.into_err())
        }
    }
}

impl<'a, T: 'a> ForeignTryAccept<'a, &'a mut [&'a mut [T]]> for Slice<Slice<T>> {
    fn try_accept(self) -> Result<&'a mut [&'a mut [T]], ErrorStatus> {
        unsafe {
            let raw: *mut [*mut [T]] = self.try_accept()?;
            Ok(&mut *(raw as *mut [&mut [T]]))
        }
    }
}

impl<'a, T: 'a> ForeignTryAccept<'a, &'a [&'a [T]]> for Slice<Slice<T>> {
    fn try_accept(self) -> Result<&'a [&'a [T]], ErrorStatus> {
        unsafe {
            let raw: *mut [*mut [T]] = self.try_accept()?;
            Ok(&*(raw as *mut [&[T]]))
        }
    }
}

impl<'a, T: 'a> ForeignTryAccept<'a, &'a mut [T]> for Slice<T> {
    fn try_accept(self) -> Result<&'a mut [T], ErrorStatus> {
        unsafe {
            self.try_as_slice_mut_custom(ptr_is_allowed)
                .map_err(|e| e.into_err())
        }
    }
}

impl<'a, T: 'a> ForeignTryAccept<'a, &'a [T]> for Slice<T> {
    fn try_accept(self) -> Result<&'a [T], ErrorStatus> {
        unsafe {
            self.try_as_slice_custom(ptr_is_allowed)
                .map_err(|e| e.into_err())
        }
    }
}

impl<'a> ForeignTryAccept<'a, *mut [&'a str]> for Slice<Str> {
    fn try_accept(self) -> Result<*mut [&'a str], ErrorStatus> {
        unsafe {
            self.try_into_str_slices_mut(ptr_is_allowed)
                .map_err(|e| e.into_err())
        }
    }
}

impl<'a> ForeignTryAccept<'a, &'a [&'a str]> for Slice<Str> {
    fn try_accept(self) -> Result<&'a [&'a str], ErrorStatus> {
        unsafe {
            let raw: *mut [&'a str] = self.try_accept()?;
            Ok(&*raw)
        }
    }
}

impl<'a> ForeignTryAccept<'a, &'a str> for Str {
    fn try_accept(self) -> Result<&'a str, ErrorStatus> {
        unsafe {
            self.try_as_str_custom(ptr_is_allowed)
                .map_err(|e| e.into_err())
        }
    }
}

impl<'a, PassedT: NotZeroable + ForeignTryAccept<'a, ResultT>, ResultT: 'a>
    ForeignTryAccept<'a, Option<ResultT>> for OptZero<PassedT>
{
    fn try_accept(self) -> Result<Option<ResultT>, ErrorStatus> {
        match self.into_option() {
            None => Ok(None),
            Some(some) => some.try_accept().map(Some),
        }
    }
}
