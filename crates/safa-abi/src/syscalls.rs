/// defines Syscall numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SyscallTable {
    SysPExit = 0,
    /// Yields execution to the next thread in the current CPU
    SysTYield = 1,
    /// Opens a file or directory with all permissions
    SysFSOpenAll = 2,
    /// Opens a file or directory with given mode (permissions and flags)
    SysFSOpen = 25,
    /// Deletes a path
    SysFSRemovePath = 28,
    /// Given a Directory resource, opens a Directory Iterator
    SysFDirIterOpen = 8,
    /// Destroys (closes) an open resource whether it is a file, directory, directory iterator, or any other resource
    SysRDestroy = 5,
    /// Legacy system call to close a directory iterator, use [`SysRDestroy`] instead
    SysDirIterClose = 9,
    /// Given a Directory Iterator Resource, returns the next DirEntry in the directory
    SysDirIterNext = 10,
    /// Performs a write operation on a given resource
    ///
    /// If the resource is a file, writes the given buffer to the file, the writes are pending until [SysIOSync] is performed.
    ///
    /// If the resource is a device, the behavior is device specific.
    ///
    /// Otherwise, errors with [`NotAFile`]
    SysIOWrite = 3,
    /// Performs a read operation on a given resource
    ///
    /// If the resource is a file, reads the given buffer from the file.
    ///
    /// If the resource is a device, the behavior is device specific.
    ///
    /// Otherwise, errors with [`NotAFile`]
    SysIORead = 4,
    /// Creates a new file
    SysFSCreate = 6,
    /// Creates a new directory
    SysFSCreateDir = 7,
    /// Performs a Sync operation on a given resource
    ///
    /// If the resource is a device, the behavior is device specific.
    ///
    /// If the resource is a file, writes all pending data to the file.
    ///
    /// Otherwise, does nothing or errors with [`NotAFile`]
    SysIOSync = 16,
    /// Truncates a file to a given size
    SysIOTruncate = 17,
    /// Sends a Command to a given resource that is a device
    ///
    /// The behavior is device specific.
    ///
    /// Takes 2 arguments: the command (can be as big as size of u16) and the argument (can be as big as size of u64)
    SysIOCommand = 12,
    /// Duplicates a given resource, returns a new resource ID pointing to the same resource internally
    ///
    /// Succeeds whether the resource is a file, directory, directory iterator or a device
    SysRDup = 26,
    // TODO: remove in favor of FAttrs
    SysFSize = 22,
    SysFAttrs = 24,
    SysFGetDirEntry = 23,
    /// Changes the current working directory to the given path
    SysPCHDir = 14,
    /// Gets the current working directory, returns [`crate::errors::ErrorStatus::Generic`] if the given buffer
    /// is too small to hold the path, always returns the current working directory length whether or not the buffer is small
    SysPGetCWD = 15,
    /// Extends the current process's address space by the given amount, amount can be negative to shrink the address space
    ///
    /// Basically maps (or unmaps) the given amount of memory
    /// Returns the new data break (address space end)
    SysPSbrk = 18,
    /// Spawns a new process
    SysPSpawn = 19,
    /// Spawns a thread inside the current process with the given entry point
    SysTSpawn = 29,
    /// Exits the current thread, takes an exit code so that it can act as [SysPExit] if it's the last thread in the process (otherwise it is unused)
    SysTExit = 30,
    /// Sleeps the current thread for the given amount of milliseconds, max is [`u64::MAX`]
    SysTSleep = 31,
    /// Waits for a child process with a given PID to exit, cleans it up and returns the exit code
    SysPWait = 11,
    /// Waits for a child thread with a given TID to exit
    SysTWait = 32,
    /// like [`SysPWait`] without the waiting part, cleans up the given process and returns the exit code
    ///
    /// returns [`crate::errors::ErrorStatus::InvalidPid`] if the process doesn't exist
    ///
    /// returns [`crate::errors::ErrorStatus::Generic`] if the process exists but hasn't exited yet
    SysPTryCleanUp = 33,
    /// Performs a WAIT(addr, val) on the current thread, also takes a timeout
    SysTFutWait = 34,
    /// Performs a WAKE(addr, n) on the current thread, wakes n threads waiting on the given address
    SysTFutWake = 35,

    SysShutdown = 20,
    SysReboot = 21,
    /// returns the Uptime of the system in milliseconds
    SysUptime = 27,
}

// sadly we cannot use any proc macros here because this crate is used by the libstd port and more, they don't happen to like proc macros...
/// When a new syscall is added, add to this number, and use the old value as the syscall number
const NEXT_SYSCALL_NUM: u16 = 36;

impl TryFrom<u16> for SyscallTable {
    type Error = ();
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value < NEXT_SYSCALL_NUM {
            Ok(unsafe { core::mem::transmute(value) })
        } else {
            Err(())
        }
    }
}
