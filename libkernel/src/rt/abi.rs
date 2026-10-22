use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

/// Type erased &self pointer for use with generated trait vtable
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OpaqueSelf<'a> {
    ptr: NonNull<()>,
    _phantom: PhantomData<&'a ()>,
}

impl<'a> OpaqueSelf<'a> {
    #[inline(always)]
    pub const fn as_ptr(&self) -> NonNull<()> {
        self.ptr
    }
    #[inline(always)]
    pub const fn new(ptr: NonNull<()>) -> Self {
        Self {
            ptr,
            _phantom: PhantomData,
        }
    }
    #[inline(always)]
    pub unsafe fn into_ref<T>(self) -> &'a T {
        unsafe { self.ptr.cast::<T>().as_ref() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
/// Repr(c) str
pub struct FFIStr<'a> {
    slice: FFISlice<'a, u8>,
}

impl<'a> FFIStr<'a> {
    #[inline(always)]
    pub const fn from_str(str: &'a str) -> Self {
        Self {
            slice: FFISlice::from_slice(str.as_bytes()),
        }
    }

    #[inline(always)]
    pub const fn into_str(self) -> &'a str {
        unsafe { core::str::from_utf8_unchecked(self.slice.into_slice()) }
    }
}

impl<'a> Deref for FFIStr<'a> {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        unsafe { core::str::from_utf8_unchecked(self.slice.into_slice()) }
    }
}

impl<'a> core::fmt::Debug for FFIStr<'a> {
    #[inline(always)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.into_str().fmt(f)
    }
}

impl<'a> core::fmt::Display for FFIStr<'a> {
    #[inline(always)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.into_str().fmt(f)
    }
}

#[derive(PartialEq, Eq)]
#[repr(C)]
/// Repr(C) slice.
pub struct FFISlice<'a, T> {
    ptr: NonNull<T>,
    len: usize,
    _phantom: PhantomData<&'a [T]>,
}

impl<'a, T: core::fmt::Debug> core::fmt::Debug for FFISlice<'a, T> {
    #[inline(always)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.into_slice().fmt(f)
    }
}

impl<'a, T> Clone for FFISlice<'a, T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            len: self.len,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T> Copy for FFISlice<'a, T> {}

unsafe impl<'a, T: Send> Send for FFISlice<'a, T> {}
unsafe impl<'a, T: Sync> Sync for FFISlice<'a, T> {}

#[repr(C)]
/// Reper(C) muttable slice.
pub struct FFISliceMut<'a, T> {
    ptr: NonNull<T>,
    len: usize,
    _phantom: PhantomData<&'a mut [T]>,
}

impl<'a, T: core::fmt::Debug> core::fmt::Debug for FFISliceMut<'a, T> {
    #[inline(always)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (&**self).fmt(f)
    }
}

unsafe impl<'a, T: Send> Send for FFISliceMut<'a, T> {}
unsafe impl<'a, T: Sync> Sync for FFISliceMut<'a, T> {}

impl<'a, T> FFISlice<'a, T> {
    #[inline(always)]
    pub const fn from_slice(value: &'a [T]) -> Self {
        Self {
            ptr: NonNull::from_ref(value).cast(),
            len: value.len(),
            _phantom: PhantomData,
        }
    }

    #[inline(always)]
    pub const fn into_slice(self) -> &'a [T] {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_ref() }
    }
}

impl<'a, T> FFISliceMut<'a, T> {
    #[inline(always)]
    pub const fn from_slice_mut(value: &'a mut [T]) -> Self {
        Self {
            ptr: NonNull::from_mut(value).cast(),
            len: value.len(),
            _phantom: PhantomData,
        }
    }

    pub const fn into_slice_mut(self) -> &'a mut [T] {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_mut() }
    }
}

impl<'a, T> Deref for FFISlice<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_ref() }
    }
}

impl<'a, T> Deref for FFISliceMut<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_ref() }
    }
}

impl<'a, T> DerefMut for FFISliceMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_mut() }
    }
}

impl<'a, T> From<&'a [T]> for FFISlice<'a, T> {
    #[inline(always)]
    fn from(value: &'a [T]) -> Self {
        Self::from_slice(value)
    }
}

impl<'a> From<&'a str> for FFIStr<'a> {
    #[inline(always)]
    fn from(value: &'a str) -> Self {
        Self::from_str(value)
    }
}

impl<'a, T> From<&'a mut [T]> for FFISlice<'a, T> {
    #[inline(always)]
    fn from(value: &'a mut [T]) -> Self {
        Self::from_slice(value)
    }
}

impl<'a, T> From<&'a mut [T]> for FFISliceMut<'a, T> {
    #[inline(always)]
    fn from(value: &'a mut [T]) -> Self {
        Self::from_slice_mut(value)
    }
}
