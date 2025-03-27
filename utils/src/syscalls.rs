use super::{errors::ErrorStatus, path::Path};

/// Safely converts FFI [`Self::Args`] into [`Self`] for being passed to a syscall
pub trait SyscallFFI: Sized {
    type Args;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus>;
}

/// converts `*const T` into `None` if the pointer is null if it is not aligned it will return an
/// [`ErrorStatus::InvaildPtr`]
impl<T> SyscallFFI for Option<&T> {
    type Args = *const T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() {
            Ok(None)
        } else if !args.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
        } else {
            Ok(unsafe { Some(&*args) })
        }
    }
}

/// converts `*mut T` into `None` if the pointer is null if it is not aligned it will return an
/// [`ErrorStatus::InvaildPtr`]
impl<T> SyscallFFI for Option<&mut T> {
    type Args = *mut T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() {
            Ok(None)
        } else if !args.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
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
            let str = core::str::from_utf8(slice).map_err(|_| ErrorStatus::InvaildStr)?;
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
        if args.is_null() || !args.is_aligned() {
            Err(ErrorStatus::InvaildPtr)
        } else {
            Ok(unsafe { &*args })
        }
    }
}

/// converts `&mut T` into `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &mut T {
    type Args = *mut T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() || !args.is_aligned() {
            Err(ErrorStatus::InvaildPtr)
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
        } else if !ptr.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
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
        } else if !ptr.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
        } else {
            Ok(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
        }
    }
}

impl SyscallFFI for &str {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let slice: &[u8] = SyscallFFI::make(args)?;
        core::str::from_utf8(slice).map_err(|_| ErrorStatus::InvaildPtr)
    }
}

impl SyscallFFI for Path<'_> {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let str = <&str>::make(args)?;
        Ok(Path::new(str)?)
    }
}

pub use safa_abi::syscalls::*;
