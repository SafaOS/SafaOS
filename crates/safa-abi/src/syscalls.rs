/// defines Syscall numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SyscallTable {
    SysPExit = 0,
    SysTYield = 1,
    /// Opens a file or directory with all permissions
    SysOpenAll = 2,
    /// Opens a file or directory with given mode (permissions and flags)
    SysOpen = 25,
    /// Deletes a path
    SysRemovePath = 28,
    SysDirIterOpen = 8,
    /// Destroys (closes) an open resource whether it is a file, directory, directory iterator, or any other resource
    SysDestroyResource = 5,
    /// Legacy system call to close a directory iterator, use [`SysDestroy`] instead
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

    /// Spawns a process (task)
    SysPSpawn = 19,
    /// Spawns a thread (context) inside the current process (task) with the given entry point
    SysTSpawn = 29,
    SysWait = 11,

    SysShutdown = 20,
    SysReboot = 21,
    /// returns the Uptime of the system in milliseconds
    SysUptime = 27,
}

// sadly we cannot use any proc macros here because this crate is used by the libstd port and more, they don't happen to like proc macros...
/// When a new syscall is added, add to this number, and use the old value as the syscall number
const _NEXT_SYSCALL_NUM: u16 = 30;

impl SyscallTable {
    // update when a new Syscall Num is added
    const MAX: u16 = Self::SysTSpawn as u16;
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
