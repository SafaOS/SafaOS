#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorStatus {
    /// Use when no ErrorStatus is available for xyz and you cannot add a new one
    Generic = 1,
    OperationNotSupported = 2,
    /// For example an elf class is not supported, there is a difference between NotSupported and
    /// OperationNotSupported
    NotSupported = 3,
    /// For example a magic value is invalid
    Corrupted = 4,
    InvalidSyscall = 5,
    /// There is no Resource associated with a given ID
    UnknownResource = 6,
    InvalidPid = 7,
    InvalidOffset = 8,
    /// instead of panicking syscalls will return this on null and unaligned pointers
    /// some operations may accept null pointers
    InvalidPtr = 9,
    /// for operations that requires a valid utf8 str...
    InvalidStr = 0xA,
    /// for operations that requires a str that doesn't exceed a max length such as
    /// file names (128 bytes)
    StrTooLong = 0xB,
    InvalidPath = 0xC,
    NoSuchAFileOrDirectory = 0xD,
    NotAFile = 0xE,
    NotADirectory = 0xF,
    AlreadyExists = 0x10,
    NotExecutable = 0x11,
    DirectoryNotEmpty = 0x12,
    /// Generic permissions(protection) related error
    MissingPermissions = 0x13,
    /// Memory Mapping error for now this means that the region has been already mapped before
    MMapError = 0x14,
    Busy = 0x15,
    // Errors sent by processes
    NotEnoughArguments = 0x16,
    OutOfMemory = 0x17,
    /// Invalid Thread ID
    InvalidTid = 0x18,
    /// Operation Timeouted
    Timeout = 0x19,
    /// A given Command is unknown or invalid
    InvalidCommand = 0x1A,
    /// A given Argument is invalid
    InvalidArgument = 0x1B,
    Unknown = 0x1C,
    /// A panick or a fatal exception occurred, used for example when the rust runtime panics and it wants to exit the process with a value
    Panic = 0x1D,
    /// A given resource wasn't a Device while one was expected
    NotADevice = 0x1E,
    /// An Operation on a resource would block while it was configured as not blockable, for example through sockets
    WouldBlock = 0x1F,
    /// A bi-directional Connection closed from a side and not the other
    ConnectionClosed = 0x20,
    /// Attempt to form a Connection failed
    ConnectionRefused = 0x21,
    /// There is a resource associated with the given ID but it isn't supported by that operation
    UnsupportedResource = 0x22,
    /// The given Resource is not duplictable
    ResourceCloneFailed = 0x23,
    /// A given X is incompatible with a Y in an operation that requires them to be compatible
    ///
    /// Used for example with Sockets when you try to connect with a bad Descriptor
    TypeMismatch = 0x24,
    TooShort = 0x25,
    /// Failed to connect to an address because it wasn't found
    AddressNotFound = 0x26,
}

impl ErrorStatus {
    // update when a new error is added
    const MAX: u16 = Self::AddressNotFound as u16;

    #[inline(always)]
    /// Gives a string description of the error
    pub fn as_str(&self) -> &'static str {
        use ErrorStatus::*;
        match *self {
            AddressNotFound => "Address Not Found",
            TooShort => "Too Short",
            Generic => "Generic Error",
            OperationNotSupported => "Operation Not Supported",
            NotSupported => "Object Not Supported",
            Corrupted => "Corrupted",
            InvalidSyscall => "Invalid Syscall",
            UnknownResource => "Unknown Resource ID",
            UnsupportedResource => "Resource not supported by that Operation",
            ResourceCloneFailed => "Failed to clone Resource",
            TypeMismatch => "Type Mismatch",
            InvalidPid => "Invalid PID",
            InvalidTid => "Invalid TID",
            InvalidOffset => "Invalid Offset",
            InvalidPtr => "Invalid Ptr (not aligned or null)",
            InvalidStr => "Invalid Str (not utf8)",
            StrTooLong => "Str too Long",
            InvalidPath => "Invalid Path",
            NoSuchAFileOrDirectory => "No Such a File or Directory",
            NotAFile => "Not a File",
            NotADirectory => "Not a Directory",
            AlreadyExists => "Already Exists",
            NotExecutable => "Not Executable",
            DirectoryNotEmpty => "Directory not Empty",
            MissingPermissions => "Missing Permissions",
            MMapError => "Memory Map Error (most likely out of memory)",
            Busy => "Resource Busy",
            NotEnoughArguments => "Not Enough Arguments",
            OutOfMemory => "Out of Memory",
            InvalidArgument => "Invalid Argument",
            InvalidCommand => "Invalid Command",
            Unknown => "Operation Unknown",
            Panic => "Unrecoverable Panick",
            Timeout => "Operation Timeouted",
            NotADevice => "Not A Device",
            ConnectionClosed => "Connection Closed",
            ConnectionRefused => "Connection Refused",
            WouldBlock => "Operation Would Block",
        }
    }
}

impl TryFrom<u16> for ErrorStatus {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value > 0 && value <= Self::MAX {
            Ok(unsafe { core::mem::transmute(value) })
        } else {
            Err(())
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysResult {
    Success,
    Error(ErrorStatus),
}

impl From<ErrorStatus> for SysResult {
    #[inline(always)]
    fn from(value: ErrorStatus) -> Self {
        SysResult::Error(value)
    }
}

impl From<Result<(), ErrorStatus>> for SysResult {
    #[inline(always)]
    fn from(value: Result<(), ErrorStatus>) -> Self {
        match value {
            Ok(()) => SysResult::Success,
            Err(err) => SysResult::Error(err),
        }
    }
}

impl TryFrom<u16> for SysResult {
    type Error = ();
    #[inline(always)]
    fn try_from(value: u16) -> Result<Self, ()> {
        match value {
            0 => Ok(SysResult::Success),
            other => {
                let err = ErrorStatus::try_from(other).map_err(|_| ())?;
                Ok(SysResult::Error(err))
            }
        }
    }
}

impl From<SysResult> for Result<(), ErrorStatus> {
    #[inline(always)]
    fn from(value: SysResult) -> Self {
        match value {
            SysResult::Success => Ok(()),
            SysResult::Error(err) => Err(err),
        }
    }
}

impl Into<u16> for SysResult {
    #[inline(always)]
    fn into(self) -> u16 {
        match self {
            SysResult::Success => 0,
            SysResult::Error(err) => err as u16,
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

#[cfg(feature = "std")]
mod std_only {
    use super::SysResult;
    use std::process::ExitCode;
    use std::process::Termination;
    impl Termination for SysResult {
        fn report(self) -> ExitCode {
            let u16: u16 = self.into();
            ExitCode::from(u16 as u8)
        }
    }
}
