/// defines Syscall numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SyscallTable {
    SysExit = 0,
    SysYield = 1,

    SysOpen = 2,
    SysDirIterOpen = 8,
    SysClose = 5,
    SysDirIterClose = 9,
    SysDirIterNext = 10,
    SysWrite = 3,
    SysRead = 4,
    SysCreate = 6,
    SysCreateDir = 7,
    SysSync = 16,
    SysTruncate = 17,
    SysCtl = 12,

    SysDup = 26,
    // TODO: remove in favor of FAttrs
    SysFSize = 22,
    SysFAttrs = 24,
    SysGetDirEntry = 23,

    SysCHDir = 14,
    SysGetCWD = 15,
    SysSbrk = 18,

    SysPSpawn = 19,
    SysWait = 11,
    SysMetaTake = 25,

    SysShutdown = 20,
    SysReboot = 21,
    /// returns the Uptime of the system in milliseconds
    SysUptime = 27,
}

impl SyscallTable {
    // update when a new Syscall Num is added
    const MAX: u16 = Self::SysDup as u16;
}

impl TryFrom<u16> for SyscallTable {
    type Error = ();
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value <= Self::MAX {
            Ok(unsafe { core::mem::transmute(value) })
        } else {
            Err(())
        }
    }
}
