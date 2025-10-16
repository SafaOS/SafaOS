use core::fmt::Debug;

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
    /// For example happens when a new error is added here, returned by a syscall and the Userspace App isn't aware of what that error means because it's ABI is out of date
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
    /// A given buffer is too short to hold full information, this for example is used with the keyboard input event device to indicate that we couldn't fit an event in a buffer
    TooShort = 0x25,
    /// Failed to connect to an address because it wasn't found
    AddressNotFound = 0x26,
    /// A given input buffer size is not acceptable by the attempted operation
    InvalidSize = 0x27,
    /// The syscall was interrupted by something, such as a kill command.
    ForceTerminated = 0x28,
    /// Attempt to use an address thats already used.
    AddressAlreadyInUse = 0x29,
    /// Attempt to use an interface thats not bound to an address.
    NotBound = 0x2A,
}

impl ErrorStatus {
    // update when a new error is added
    const MAX: u16 = Self::NotBound as u16;

    #[inline(always)]
    /// Gives a string description of the error
    pub fn as_str(&self) -> &'static str {
        use ErrorStatus::*;
        match *self {
            InvalidSize => "Invalid Size",
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
            ForceTerminated => "Operation Terminated",
            AddressAlreadyInUse => "Address Already In Use",
            NotBound => "Interface Not Bound",
        }
    }

    /// Try to convert a given `value` into an error code
    pub const fn try_from_u16(value: u16) -> Result<Self, ()> {
        if value > 0 && value <= Self::MAX {
            Ok(unsafe { core::mem::transmute(value) })
        } else {
            Err(())
        }
    }
    /// Converts a given `value` into an error code on failure, returns [`Self::Unknown`]
    pub const fn from_u16(value: u16) -> Self {
        /* const hack instead of unwrap_or */
        match Self::try_from_u16(value) {
            Ok(k) => k,
            Err(()) => Self::Unknown,
        }
    }
}

impl TryFrom<u16> for ErrorStatus {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::try_from_u16(value)
    }
}

/// Represents the results of a SafaOS syscall, either an [`ErrorStatus`] or an Ok [`usize`] value smaller than or equal to [`isize::MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SysResult(isize);

impl SysResult {
    /// Converts an err into a [`SysResult`].
    pub const fn err(err: ErrorStatus) -> Self {
        Self(-(err as isize))
    }

    /// Turns a [`SysResult`] into result.
    pub const fn into_result(self) -> Result<usize, ErrorStatus> {
        if self.0.is_negative() {
            Err(ErrorStatus::from_u16((-self.0) as u16))
        } else {
            Ok(self.0 as usize)
        }
    }

    /// Attempts to convert a result into [`SysResult`], returns an error if the value is Ok(x) and x is larger than [`isize::MAX`]
    pub const fn try_from_result(result: Result<usize, ErrorStatus>) -> Result<Self, ()> {
        match result {
            Err(err) => Ok(Self::err(err)),
            Ok(value) => Self::try_ok(value),
        }
    }

    /// Converts an Ok value [`value`] into [`Self`], it is expected to not be larger than [`isize::MAX`] or it panicks.
    pub const fn ok(value: usize) -> Self {
        let Ok(ok) = Self::try_ok(value) else {
            panic!("Attempt to construct an Ok SysResult value larger than isize::MAX")
        };
        ok
    }

    /// Tries to convert an Ok value [`value`] into [`SysResult`], return an error if the value is larger than [`isize::MAX`].
    pub const fn try_ok(value: usize) -> Result<Self, ()> {
        let value = value as isize;
        if value.is_negative() {
            return Err(());
        }

        Ok(Self(value))
    }

    /// Converts a [`SysResult`] into an isize, negative value is for an error, use [`Self::into_result`] instead.
    pub const fn as_isize(&self) -> isize {
        self.0
    }

    /// Converts an isize into [`SysResult`]
    /// # Safety
    /// Perefectly safe as this type doesn't guarantee the contained error value (negative value) is valid.
    pub const fn from_isize(isize: isize) -> Self {
        Self(isize)
    }
}

impl From<ErrorStatus> for SysResult {
    #[inline(always)]
    fn from(value: ErrorStatus) -> Self {
        Self::err(value)
    }
}

impl From<Result<usize, ErrorStatus>> for SysResult {
    /// Panicks if the results is an Ok value larger than isize::MAX
    #[inline(always)]
    fn from(value: Result<usize, ErrorStatus>) -> Self {
        Self::try_from_result(value).expect("Ok value is bigger than isize::MAX")
    }
}

impl From<SysResult> for Result<usize, ErrorStatus> {
    #[inline(always)]
    fn from(value: SysResult) -> Self {
        value.into_result()
    }
}

impl Into<isize> for SysResult {
    #[inline(always)]
    fn into(self) -> isize {
        self.0
    }
}

impl From<isize> for SysResult {
    fn from(value: isize) -> Self {
        Self::from_isize(value)
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

impl From<core::str::Utf8Error> for ErrorStatus {
    fn from(value: core::str::Utf8Error) -> Self {
        match value {
            core::str::Utf8Error { .. } => Self::InvalidStr,
        }
    }
}

#[cfg(feature = "std")]
mod std_only {
    use super::SysResult;
    use std::process::ExitCode;
    use std::process::Termination;
    impl Termination for SysResult {
        fn report(self) -> ExitCode {
            let u16: u16 = match self.into_result() {
                Ok(_) => 0,
                Err(smth) => smth as u16,
            };
            ExitCode::from(u16 as u8)
        }
    }
}
