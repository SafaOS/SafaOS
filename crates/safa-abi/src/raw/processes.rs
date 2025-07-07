use core::ops::BitOr;

use super::{Optional, RawSlice, RawSliceMut};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
/// ABI structures are structures that are passed to tasks by the parent task
/// for now only stdio file descriptors are passed
/// you get a pointer to them in the `r8` register at _start (the 5th argument)
pub struct AbiStructures {
    pub stdio: TaskStdio,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct TaskStdio {
    pub stdout: Optional<usize>,
    pub stdin: Optional<usize>,
    pub stderr: Optional<usize>,
}

impl TaskStdio {
    pub fn new(stdout: Option<usize>, stdin: Option<usize>, stderr: Option<usize>) -> Self {
        Self {
            stdout: stdout.into(),
            stdin: stdin.into(),
            stderr: stderr.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
/// Flags for the [crate::syscalls::SyscallTable::SysPSpawn] syscall
pub struct SpawnFlags(u8);
impl SpawnFlags {
    pub const CLONE_RESOURCES: Self = Self(1 << 0);
    pub const CLONE_CWD: Self = Self(1 << 1);
    pub const EMPTY: Self = Self(0);
}

impl BitOr for SpawnFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPriority {
    Low,
    Medium,
    High,
}

impl ContextPriority {
    /// Returns the number of timeslices a thread with this priority should be given.
    pub const fn timeslices(&self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Medium => 3,
            Self::High => 5,
        }
    }
}

/// configuration for the spawn syscall
#[repr(C)]
pub struct PSpawnConfig {
    /// config version for compatibility
    /// added in kernel version 0.2.1 and therefore breaking compatibility with any program compiled for version below 0.2.1
    /// revision 1: added env
    /// revision 2: added priority (v0.4.0)
    pub revision: u8,
    pub name: RawSlice<u8>,
    pub argv: RawSliceMut<RawSlice<u8>>,
    pub flags: SpawnFlags,
    pub stdio: *const TaskStdio,
    pub env: RawSliceMut<RawSlice<u8>>,
    pub priority: Optional<ContextPriority>,
}

impl PSpawnConfig {
    pub fn new(
        name: &str,
        argv: *mut [&[u8]],
        env: *mut [&[u8]],
        flags: SpawnFlags,
        stdio: &TaskStdio,
        priority: Option<ContextPriority>,
    ) -> Self {
        let name = unsafe { RawSlice::from_slice(name.as_bytes()) };
        let argv = unsafe { RawSliceMut::from_slices(argv) };
        let env = unsafe { RawSliceMut::from_slices(env) };

        Self {
            revision: 2,
            name,
            argv,
            env,
            flags,
            stdio: stdio as *const TaskStdio,
            priority: priority.into(),
        }
    }
}

/// configuration for the thread spawn syscall
/// for now it takes only a single argument pointer which is a pointer to an optional argument, that pointer is going to be passed to the thread as the second argument
#[repr(C)]
pub struct TSpawnConfig {
    pub revision: u32,
    pub argument_ptr: *const (),
    pub priority: Optional<ContextPriority>,
    /// The index of the CPU to append to, if it is None the kernel will choose one, use `0` for the boot CPU
    pub cpu: Optional<usize>,
}

impl TSpawnConfig {
    pub fn into_rust(&self) -> (*const (), Option<ContextPriority>, Option<usize>) {
        (self.argument_ptr, self.priority.into(), self.cpu.into())
    }

    /// Create a new thread spawn configuration with the latest revision
    pub fn new(
        argument_ptr: *const (),
        priority: Option<ContextPriority>,
        cpu: Option<usize>,
    ) -> Self {
        Self {
            revision: 0,
            argument_ptr,
            priority: priority.into(),
            cpu: cpu.into(),
        }
    }
}
