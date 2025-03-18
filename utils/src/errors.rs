use int_enum::IntEnum;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntEnum)]
#[repr(u16)]
pub enum ErrorStatus {
    // use when no ErrorStatus is avalible for xyz and you cannot add a new one
    Generic = 1,
    OperationNotSupported = 2,
    // for example an elf class is not supported, there is a difference between NotSupported and
    // OperationNotSupported
    NotSupported = 3,
    // for example a magic value is invaild
    Corrupted = 4,
    InvaildSyscall = 5,
    InvaildResource = 6,
    InvaildPid = 7,
    InvaildOffset = 8,
    // instead of panicking syscalls will return this on null and unaligned pointers
    InvaildPtr = 9,
    // for operations that requires a vaild utf8 str...
    InvaildStr = 0xA,
    // for operations that requires a str that doesn't exceed a max length such as
    // file names (128 bytes)
    StrTooLong = 0xB,
    InvaildPath = 0xC,
    NoSuchAFileOrDirectory = 0xD,
    NotAFile = 0xE,
    NotADirectory = 0xF,
    AlreadyExists = 0x10,
    NotExecutable = 0x11,
    // would be useful when i add remove related operations to the vfs
    DirectoryNotEmpty = 0x12,
    // Generic premissions(protection) related error
    MissingPermissions = 0x13,
    // memory allocations and mapping error, most likely that memory is full
    MMapError = 0x14,
    Busy = 0x15,
    // errors sent by processes
    NotEnoughArguments = 0x16,
    OutOfMemory = 0x17,
}
impl ErrorStatus {
    #[inline(always)]
    /// Gives a string description of the error
    pub fn as_str(&self) -> &'static str {
        use ErrorStatus::*;
        match *self {
            Generic => "Generic Error",
            OperationNotSupported => "Operation Not Supported",
            NotSupported => "Object Not Supported",
            Corrupted => "Corrupted",
            InvaildSyscall => "Invaild Syscall",
            InvaildResource => "Invaild Resource",
            InvaildPid => "Invaild PID",
            InvaildOffset => "Invaild Offset",
            InvaildPtr => "Invaild Ptr (not aligned or null)",
            InvaildStr => "Invaild Str (not utf8)",
            StrTooLong => "Str too Long",
            InvaildPath => "Invaild Path",
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
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysResult {
    Sucess,
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
            Ok(()) => SysResult::Sucess,
            Err(err) => SysResult::Error(err),
        }
    }
}

impl TryFrom<u16> for SysResult {
    type Error = ();
    #[inline(always)]
    fn try_from(value: u16) -> Result<Self, ()> {
        match value {
            0 => Ok(SysResult::Sucess),
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
            SysResult::Sucess => Ok(()),
            SysResult::Error(err) => Err(err),
        }
    }
}

impl Into<u16> for SysResult {
    #[inline(always)]
    fn into(self) -> u16 {
        match self {
            SysResult::Sucess => 0,
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
