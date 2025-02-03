//! safe FFI types to make it easier to interact with userspace
use super::errors::{ErrorStatus, ErrorStatusResult};

/// a Nullable refrence to a value
/// if null it is a None if Some it is a valid reference
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Optional<T> {
    value: *mut T,
}

impl<'a, T> Optional<T> {
    pub const fn new(value: &'a mut T) -> Self {
        Self { value }
    }

    pub const fn none() -> Self {
        Self {
            value: core::ptr::null_mut(),
        }
    }

    pub fn is_none(&self) -> bool {
        self.value.is_null()
    }

    pub unsafe fn unwrap_unchecked(self) -> &'a mut T {
        &mut *self.value
    }

    pub fn into_option(self) -> Option<&'a mut T> {
        if self.is_none() {
            None
        } else {
            unsafe { Some(self.unwrap_unchecked()) }
        }
    }

    pub fn from_option(value: Option<&mut T>) -> Self {
        match value {
            Some(value) => Self::new(value),
            None => Self::none(),
        }
    }
}

impl<'a, T> From<Option<&'a mut T>> for Optional<T> {
    fn from(value: Option<&'a mut T>) -> Self {
        Self::from_option(value)
    }
}

impl<T> Default for Optional<T> {
    fn default() -> Self {
        Self::none()
    }
}

impl<'a, T> From<&'a mut T> for Optional<T> {
    fn from(value: &'a mut T) -> Self {
        Self::new(value)
    }
}

/// a slice of values
/// if into_slice is called on a null pointer it will return an empty slice
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Slice<T> {
    ptr: *const T,
    len: usize,
}

impl<'a, T> Slice<T> {
    /// ptr must be aligned
    /// panics if ptr is invaild
    pub fn new(ptr: *const T, len: usize) -> ErrorStatusResult<Self> {
        if !ptr.is_aligned() {
            ErrorStatusResult::err(ErrorStatus::InvaildPtr)
        } else {
            ErrorStatusResult::ok(Self { ptr, len })
        }
    }

    /// converts Slice to a slice
    /// returns an empty slice if the pointer is null
    #[inline(always)]
    pub fn into_slice(self) -> &'a [T] {
        if self.ptr.is_null() {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<'a, T> From<Slice<T>> for &'a [T] {
    fn from(slice: Slice<T>) -> &'a [T] {
        slice.into_slice()
    }
}

/// a mutable slice of values
/// if into_slice is called on a null pointer it will return an empty slice
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SliceMut<T> {
    ptr: *mut T,
    len: usize,
}

impl<'a, T> SliceMut<T> {
    /// ptr must be aligned
    /// panics if ptr is invaild
    pub fn new(ptr: *mut T, len: usize) -> ErrorStatusResult<Self> {
        if !ptr.is_aligned() {
            ErrorStatusResult::err(ErrorStatus::InvaildPtr)
        } else {
            ErrorStatusResult::ok(Self { ptr, len })
        }
    }

    /// converts Slice to a slice
    /// returns an empty slice if the pointer is null
    #[inline(always)]
    pub fn into_slice(self) -> &'a mut [T] {
        if self.ptr.is_null() {
            return &mut [];
        }

        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl<'a, T> From<SliceMut<T>> for &'a mut [T] {
    fn from(slice: SliceMut<T>) -> &'a mut [T] {
        slice.into_slice()
    }
}

impl SliceMut<u8> {
    /// converts the slice to a str which is accepted by the kernel
    /// may panic if the slice is not valid utf8 in the future
    pub fn into_str<'a>(self) -> &'a str {
        unsafe { core::str::from_utf8_unchecked(self.into_slice()) }
    }
}

impl Slice<u8> {
    /// converts the slice to a str which is accepted by the kernel
    /// may panic if the slice is not valid utf8 in the future
    pub fn into_str<'a>(self) -> &'a str {
        unsafe { core::str::from_utf8_unchecked(self.into_slice()) }
    }
}

impl<'a> From<Slice<u8>> for &'a str {
    fn from(slice: Slice<u8>) -> &'a str {
        slice.into_str()
    }
}

impl<'a> From<SliceMut<u8>> for &'a str {
    fn from(slice: SliceMut<u8>) -> &'a str {
        slice.into_str()
    }
}

impl SliceMut<Slice<u8>> {
    /// converts the slice to a slice of strs which is accepted by the kernel
    /// may panic if the slice is not valid utf8 in the future
    pub fn into_str_slice<'a>(self) -> &'a [&'a str] {
        let slice = self.into_slice();
        let double_slice = unsafe { &mut *(self.into_slice() as *const _ as *mut [&str]) };

        for (i, item) in slice.iter().enumerate() {
            double_slice[i] = item.into_str();
        }

        double_slice
    }
}
/// a non-Nullable muttable refrence to a value
/// use .get() to get the value
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct RequiredMut<T> {
    value: *mut T,
}

impl<'a, T> RequiredMut<T> {
    pub fn get(self) -> ErrorStatusResult<&'a mut T> {
        if self.value.is_null() || !self.value.is_aligned() {
            ErrorStatusResult::err(ErrorStatus::InvaildPtr)
        } else {
            ErrorStatusResult::ok(unsafe { &mut *self.value })
        }
    }
}

/// a non-Nullable refrence to a value
/// use .get() to get the value
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Required<T> {
    value: *const T,
}

impl<'a, T> Required<T> {
    pub fn get(self) -> ErrorStatusResult<&'a T> {
        if self.value.is_null() || !self.value.is_aligned() {
            ErrorStatusResult::err(ErrorStatus::InvaildPtr)
        } else {
            ErrorStatusResult::ok(unsafe { &*self.value })
        }
    }
}
