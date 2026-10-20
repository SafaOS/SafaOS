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

#[repr(C)]
/// Repr(C) slice.
pub struct CSlice<'a, T> {
    ptr: NonNull<T>,
    len: usize,
    _phantom: PhantomData<&'a [T]>,
}

impl<'a, T> Clone for CSlice<'a, T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            len: self.len,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T> Copy for CSlice<'a, T> {}

unsafe impl<'a, T: Send> Send for CSlice<'a, T> {}
unsafe impl<'a, T: Sync> Sync for CSlice<'a, T> {}

#[repr(C)]
/// Reper(C) muttable slice.
pub struct CSliceMut<'a, T> {
    ptr: NonNull<T>,
    len: usize,
    _phantom: PhantomData<&'a mut [T]>,
}

unsafe impl<'a, T: Send> Send for CSliceMut<'a, T> {}
unsafe impl<'a, T: Sync> Sync for CSliceMut<'a, T> {}

impl<'a, T> CSlice<'a, T> {
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

impl<'a, T> CSliceMut<'a, T> {
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

impl<'a, T> Deref for CSlice<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_ref() }
    }
}

impl<'a, T> Deref for CSliceMut<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_ref() }
    }
}

impl<'a, T> DerefMut for CSliceMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { NonNull::slice_from_raw_parts(self.ptr, self.len).as_mut() }
    }
}

impl<'a, T> From<&'a [T]> for CSlice<'a, T> {
    #[inline(always)]
    fn from(value: &'a [T]) -> Self {
        Self::from_slice(value)
    }
}

impl<'a, T> From<&'a mut [T]> for CSlice<'a, T> {
    #[inline(always)]
    fn from(value: &'a mut [T]) -> Self {
        Self::from_slice(value)
    }
}

impl<'a, T> From<&'a mut [T]> for CSliceMut<'a, T> {
    #[inline(always)]
    fn from(value: &'a mut [T]) -> Self {
        Self::from_slice_mut(value)
    }
}
