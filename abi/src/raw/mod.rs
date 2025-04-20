pub mod io;
pub mod processes;

use core::ptr::NonNull;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// A C compatible slice of type `T`
pub struct RawSlice<T> {
    ptr: *const T,
    len: usize,
}

impl<T> RawSlice<T> {
    #[inline(always)]
    pub unsafe fn from_raw_parts(ptr: *const T, len: usize) -> Self {
        Self { ptr, len }
    }
    #[inline(always)]
    pub unsafe fn from_slice(slice: &[T]) -> Self {
        Self {
            ptr: slice.as_ptr(),
            len: slice.len(),
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }

    /// Converts a [`RawSlice<T>`] into a slice of type `T`
    ///
    /// returns `None` if the slice ptr is null or the length is zero
    /// # Safety
    ///
    /// panics if the slice ptr is not aligned to the alignment of `T`
    #[inline(always)]
    pub unsafe fn into_slice<'a>(self) -> Option<&'a [T]> {
        if self.ptr.is_null() || self.len == 0 {
            None
        } else {
            Some(core::slice::from_raw_parts(self.ptr, self.len))
        }
    }
}

impl<T> RawSliceMut<T> {
    #[inline(always)]
    pub unsafe fn from_raw_parts(ptr: *mut T, len: usize) -> Self {
        Self { ptr, len }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }

    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut T {
        self.ptr
    }

    /// Converts a [`RawSliceMut<T>`] into a slice of type `T`
    ///
    /// returns `None` if the slice ptr is null or the length is zero
    /// # Safety
    ///
    /// panics if the slice ptr is not aligned to the alignment of `T`
    #[inline(always)]
    pub unsafe fn into_slice_mut<'a>(self) -> Option<&'a mut [T]> {
        if self.ptr.is_null() || self.len == 0 {
            None
        } else {
            Some(core::slice::from_raw_parts_mut(self.ptr, self.len))
        }
    }
}

impl<T> RawSliceMut<RawSlice<T>> {
    /// Converts a slice of slices of [`T`] into [`RawSliceMut<RawSlice<T>>`]
    /// # Safety
    /// `slices` becomes invalid after use
    /// as it is going to be reused as a memory location for creating `Self`
    /// making this unexpensive but dangerous
    /// O(N) expect if the Layout of RawSlice is equal to the Layout of rust slices, and it has been optimized it is O(1)
    #[inline(always)]
    pub unsafe fn from_slices(slices: *mut [&[T]]) -> Self {
        let old_slices = unsafe { &mut *slices };
        let raw_slices = unsafe { &mut *(slices as *mut [RawSlice<T>]) };

        for (i, slice) in old_slices.iter().enumerate() {
            raw_slices[i] = unsafe { RawSlice::from_slice(slice) };
        }
        unsafe { RawSliceMut::from_raw_parts(raw_slices.as_mut_ptr(), raw_slices.len()) }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// A C complitable mutable slice of type `T`
pub struct RawSliceMut<T> {
    ptr: *mut T,
    len: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NonNullSlice<T> {
    ptr: NonNull<T>,
    len: usize,
}

impl<T> NonNullSlice<T> {
    pub const unsafe fn from_raw_parts(ptr: NonNull<T>, len: usize) -> Self {
        Self { ptr, len }
    }

    pub const fn as_non_null(&self) -> NonNull<T> {
        self.ptr
    }

    pub const fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// Converts a [`NonNullSlice<T>`] into a slice of type `T`
    /// # Safety
    /// panics if the slice ptr is not aligned to the alignment of `T`
    #[inline(always)]
    pub unsafe fn into_slice_mut<'a>(self) -> &'a mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

/// A C complitable Option-like type
#[derive(Debug)]
#[repr(C)]
pub enum Optional<T> {
    None,
    Some(T),
}

impl<T> Default for Optional<T> {
    fn default() -> Self {
        Self::None
    }
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

impl<T: Clone> Clone for Optional<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        match self {
            Self::None => Self::None,
            Self::Some(x) => Self::Some(x.clone()),
        }
    }
}
impl<T: Copy> Copy for Optional<T> {}

impl<T: PartialEq> PartialEq for Optional<T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Some(x), Self::Some(y)) => x == y,
            _ => false,
        }
    }
}

impl<T: Eq> Eq for Optional<T> {}
