use core::{ops::Deref, ptr::NonNull};

use crate::{
    VirtAddr,
    drivers::vfs::SeekOffset,
    fs::{DirIter, File},
    process::resources::{self, Resource, ResourceNodeRef, Ri},
};

use crate::utils::path::Path;
use safa_abi::errors::ErrorStatus;

/// Safely converts FFI [`Self::Args`] into [`Self`] for being passed to a syscall
pub trait SyscallFFI: Sized {
    type Args;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus>;
}

/// converts `*const T` into `None` if the pointer is null if it is not aligned it will return an
/// [`ErrorStatus::InvalidPtr`]
impl<T> SyscallFFI for Option<&T> {
    type Args = *const T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() {
            Ok(None)
        } else if !ptr_is_valid(args) {
            return Err(ErrorStatus::InvalidPtr);
        } else {
            Ok(unsafe { Some(&*args) })
        }
    }
}

/// converts `*mut T` into `None` if the pointer is null if it is not aligned it will return an
/// [`ErrorStatus::InvalidPtr`]
impl<T> SyscallFFI for Option<&mut T> {
    type Args = *mut T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() {
            Ok(None)
        } else if !ptr_is_valid(args) {
            return Err(ErrorStatus::InvalidPtr);
        } else {
            Ok(unsafe { Some(&mut *args) })
        }
    }
}

impl<T> SyscallFFI for Option<&[T]> {
    type Args = (*const T, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        let slice = <&[T]>::make((ptr, len))?;
        if slice.is_empty() {
            Ok(None)
        } else {
            Ok(Some(slice))
        }
    }
}

impl SyscallFFI for Option<&str> {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        let opt = <Option<&[u8]>>::make((ptr, len))?;

        if let Some(slice) = opt {
            let str = core::str::from_utf8(slice).map_err(|_| ErrorStatus::InvalidStr)?;
            Ok(Some(str))
        } else {
            Ok(None)
        }
    }
}

/// converts `&T` into `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &T {
    type Args = *const T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if !ptr_is_valid(args) {
            Err(ErrorStatus::InvalidPtr)
        } else {
            Ok(unsafe { &*args })
        }
    }
}

/// converts `&mut T` into `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &mut T {
    type Args = *mut T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if !ptr_is_valid(args) {
            Err(ErrorStatus::InvalidPtr)
        } else {
            Ok(unsafe { &mut *args })
        }
    }
}

/// for an `&[T]` it will return `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &[T] {
    type Args = (*const T, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        if ptr.is_null() {
            Ok(&[])
        } else if !ptr_is_valid(ptr) {
            return Err(ErrorStatus::InvalidPtr);
        } else {
            Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
        }
    }
}

impl<T> SyscallFFI for &mut [T] {
    type Args = (*mut T, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        if ptr.is_null() {
            Ok(&mut [])
        } else if !ptr_is_valid(ptr) {
            return Err(ErrorStatus::InvalidPtr);
        } else {
            Ok(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
        }
    }
}

impl SyscallFFI for &str {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let slice: &[u8] = SyscallFFI::make(args)?;
        core::str::from_utf8(slice).map_err(|_| ErrorStatus::InvalidPtr)
    }
}

impl SyscallFFI for Path<'_> {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let str = <&str>::make(args)?;
        Ok(Path::new(str)?)
    }
}

macro_rules! impl_ffi_int {
    ($ty:ty) => {
        impl SyscallFFI for $ty {
            type Args = $ty;
            fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
                Ok(args)
            }
        }
    };
}

impl_ffi_int!(usize);
impl_ffi_int!(isize);
impl_ffi_int!(u8);
impl_ffi_int!(i8);
impl_ffi_int!(u16);
impl_ffi_int!(i16);
impl_ffi_int!(u32);
impl_ffi_int!(i32);
impl_ffi_int!(u64);
impl_ffi_int!(i64);

impl SyscallFFI for File {
    type Args = Ri;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        File::from_fd(args)
            .map(|s| s.map_err(|()| ErrorStatus::UnsupportedResource))
            .ok_or(ErrorStatus::UnknownResource)
            .flatten()
    }
}

impl SyscallFFI for DirIter {
    type Args = Ri;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        DirIter::from_ri(args)
            .map(|s| s.map_err(|()| ErrorStatus::UnsupportedResource))
            .ok_or(ErrorStatus::UnknownResource)
            .flatten()
    }
}

impl SyscallFFI for VirtAddr {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(VirtAddr::from(args))
    }
}

/// Returns whether or not the kernel can accept this pointer
#[inline(always)]
pub fn ptr_is_allowed<T: ?Sized>(ptr: *const T) -> bool {
    let addr = VirtAddr::from_ptr(ptr);
    addr <= crate::process::PROCESS_AREA_END_ADDR
}

/// Returns whether or not the pointer is valid and the kernel can accept it
#[inline]
pub fn ptr_is_valid<T>(ptr: *const T) -> bool {
    !ptr.is_null() && ptr.is_aligned() && ptr_is_allowed(ptr)
}

impl SyscallFFI for SeekOffset {
    type Args = isize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(SeekOffset::from(args))
    }
}

/// Represents an argument passed to a syscall thats a Resource ID that points to a resource of an Unknown type,
/// Use [ExpectedResource] if you know the type.
pub struct ResourceDesc(ResourceNodeRef);

impl SyscallFFI for ResourceDesc {
    type Args = Ri;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        resources::get_expected(args as Ri).map(|ok| Self(ok))
    }
}

impl Deref for ResourceDesc {
    type Target = dyn Resource;
    fn deref(&self) -> &Self::Target {
        self.0.data()
    }
}

/// Represents a syscall resource argument of an expected resource type: T
pub struct ExpectedResource<T: Resource> {
    _inner: ResourceDesc,
    // Points to data in inner
    reference: NonNull<T>,
}

impl<T: Resource> SyscallFFI for ExpectedResource<T> {
    type Args = Ri;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let inner = ResourceDesc::make(args)?;
        // Safety: data points to data allocated on the heap thats alive as long as inner is alive
        let reference = inner.as_ref_expected::<T>()?;
        let reference = NonNull::from_ref(reference);
        Ok(Self {
            _inner: inner,
            reference,
        })
    }
}

impl<T: Resource> Deref for ExpectedResource<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { self.reference.as_ref() }
    }
}
