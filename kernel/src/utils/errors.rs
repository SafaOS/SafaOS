use core::{
    convert::Infallible,
    ops::{FromResidual, Try},
};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorStatus {
    None,
    // use when no ErrorStatus is avalible for xyz and you cannot add a new one
    Generic,
    OperationNotSupported,
    // for example an elf class is not supported, there is a difference between NotSupported and
    // OperationNotSupported
    NotSupported,
    // for example a magic value is invaild
    Corrupted,
    InvaildSyscall,
    InvaildResource,
    InvaildPid,
    InvaildOffset,
    // instead of panicking syscalls will return this on null and unaligned pointers
    InvaildPtr,
    // for operations that requires a vaild utf8 str...
    InvaildStr,
    InvaildPath,
    NoSuchAFileOrDirectory,
    NotAFile,
    NotADirectory,
    AlreadyExists,
    NotExecutable,
    // would be useful when i add remove related operations to the vfs
    DirectoryNotEmpty,
    // Generic premissions(protection) related error
    MissingPermissions,
    // memory allocations and mapping error, most likely that memory is full
    MMapError,
    Busy,
    // errors sent by processes
    NotEnoughArguments,
}

impl FromResidual for ErrorStatus {
    fn from_residual(residual: Self) -> Self {
        residual
    }
}

impl FromResidual<Result<Infallible, ErrorStatus>> for ErrorStatus {
    fn from_residual(residual: Result<Infallible, ErrorStatus>) -> Self {
        match residual {
            Ok(_) => Self::None,
            Err(err) => err,
        }
    }
}

impl Try for ErrorStatus {
    type Output = ();
    type Residual = Self;

    fn from_output(_: Self::Output) -> Self {
        Self::None
    }

    fn branch(self) -> core::ops::ControlFlow<Self::Residual, Self::Output> {
        if self == ErrorStatus::None {
            core::ops::ControlFlow::Continue(())
        } else {
            core::ops::ControlFlow::Break(self)
        }
    }
}

pub trait IntoErr {
    fn into_err(self) -> ErrorStatus;
}

impl<T: IntoErr> From<T> for ErrorStatus {
    fn from(value: T) -> Self {
        value.into_err()
    }
}
/// a Result that can be converted to an ErrorStatus
/// using `?` operator on this will return an ErrorStatus if the Result is an Err
/// this type is a bit of a hack that isn't used much, it helps clean up things with `super::ffi`
#[derive(Debug, Clone, Copy)]
pub struct ErrorStatusResult<T>(Result<T, ErrorStatus>);
impl<T> ErrorStatusResult<T> {
    pub const fn ok(s: T) -> Self {
        Self(Ok(s))
    }

    pub const fn err(s: ErrorStatus) -> Self {
        Self(Err(s))
    }
}

impl<T> FromResidual for ErrorStatusResult<T> {
    fn from_residual(residual: <Self as Try>::Residual) -> Self {
        Self(Err(residual))
    }
}

impl<T> Try for ErrorStatusResult<T> {
    type Output = T;
    type Residual = ErrorStatus;

    fn from_output(output: Self::Output) -> Self {
        Self(Ok(output))
    }

    fn branch(self) -> core::ops::ControlFlow<Self::Residual, Self::Output> {
        match self.0 {
            Ok(output) => core::ops::ControlFlow::Continue(output),
            Err(err) => core::ops::ControlFlow::Break(err),
        }
    }
}
