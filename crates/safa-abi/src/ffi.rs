//! FFI bindings for SafaOS's ABI
//!
//! for example exports [`RawSlice<T>`] which is an FFI safe alternative to `&[T]`

use core::{marker::PhantomData, ptr::NonNull};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// A C compatible slice of type `T`
pub struct RawSlice<'a, T> {
    ptr: *const T,
    len: usize,
    _maker: PhantomData<&'a [T]>,
}

impl<'a, T> RawSlice<'a, T> {
    #[inline(always)]
    pub const unsafe fn from_raw_parts(ptr: *const T, len: usize) -> Self {
        let ptr = if len == 0 {
            const { NonNull::dangling().as_ptr() }
        } else {
            ptr
        };
        Self {
            ptr,
            len,
            _maker: PhantomData,
        }
    }

    #[inline(always)]
    pub const unsafe fn from_slice(slice: &'a [T]) -> Self {
        Self {
            ptr: slice.as_ptr(),
            len: slice.len(),
            _maker: PhantomData,
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

    /// Converts a [`RawSlice<T>`] into a slice of type `T`
    ///
    /// returns `None` if the slice ptr is null or isn't aligned to the alignment of `T`
    #[inline]
    pub unsafe fn into_slice(self) -> Option<&'a [T]> {
        if self.ptr.is_null() || !self.ptr.is_aligned() {
            return None;
        }

        Some(if self.len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// A C complitable mutable slice of type `T`
pub struct RawSliceMut<'a, T> {
    ptr: *mut T,
    len: usize,
    _marker: PhantomData<&'a mut [T]>,
}

impl<'a, T> RawSliceMut<'a, T> {
    #[inline(always)]
    pub const unsafe fn from_raw_parts(ptr: *mut T, len: usize) -> Self {
        let ptr = if len == 0 {
            const { NonNull::dangling().as_ptr() }
        } else {
            ptr
        };

        Self {
            ptr,
            len,
            _marker: PhantomData,
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

    #[inline(always)]
    pub const fn as_mut_ptr(&self) -> *mut T {
        self.ptr
    }

    /// Converts a [`RawSliceMut<T>`] into a slice of type `T`
    ///
    /// returns `None` if the slice ptr is null or is not aligned to the alignment of `T`
    /// returns an empty slice if the length is zero
    #[inline(always)]
    pub unsafe fn into_slice_mut(self) -> Option<&'a mut [T]> {
        if self.ptr.is_null() || !self.ptr.is_aligned() {
            return None;
        }

        Some(if self.len == 0 {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
        })
    }
}

impl<'a, T> RawSliceMut<'a, RawSlice<'a, T>> {
    /// Converts a slice of slices of [`T`] into [`RawSliceMut<RawSlice<T>>`]
    /// # Safety
    /// `slices` becomes invalid after use
    /// as it is going to be reused as a memory location for creating `Self`
    /// making this unexpensive but dangerous
    ///
    /// O(N) expect if the Layout of RawSlice is equal to the Layout of rust slices which should be the case, and it has been optimized it is O(1)
    #[inline]
    pub const unsafe fn from_slices(slices: *mut [&'a [T]]) -> Self {
        let old_slices = unsafe { &mut *slices };
        let raw_slices = unsafe { &mut *(slices as *mut [RawSlice<T>]) };

        let mut i = 0;
        while i < old_slices.len() {
            let slice = old_slices[i];
            raw_slices[i] = unsafe { RawSlice::from_slice(slice) };
            i += 1;
        }

        unsafe { RawSliceMut::from_raw_parts(raw_slices.as_mut_ptr(), raw_slices.len()) }
    }
}

/// Raw slice of bytes that can be used as a rust &str slice
///
/// has to be valid utf8
pub type RawStrSlice<'a> = RawSlice<'a, u8>;

/// A C complitable [Option]-like type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(C)]
pub enum Optional<T> {
    #[default]
    None,
    Some(T),
}

impl<T> From<Option<T>> for Optional<T> {
    #[inline(always)]
    fn from(value: Option<T>) -> Self {
        match value {
            None => Self::None,
            Some(x) => Self::Some(x),
        }
    }
}

impl<T> From<Optional<T>> for Option<T> {
    #[inline(always)]
    fn from(value: Optional<T>) -> Self {
        match value {
            Optional::None => None,
            Optional::Some(x) => Some(x),
        }
    }
}
