//! Process & Thread related ABI structures
use core::{ops::BitOr, ptr::NonNull};

use crate::ffi::option::{COption, OptZero};
use crate::ffi::ptr::FFINonNull;
use crate::ffi::slice::Slice;
use crate::ffi::str::Str;

/// Describes information about a thread local storage, passed to the thread in the userspace
#[derive(Debug, Clone)]
#[repr(C)]
pub struct UThreadLocalInfo {
    /// A pointer to this structure in the address space the thread belongs to
    /// neccassary in x86_64 (because it sucks)
    pub uthread_ptr: NonNull<Self>,
    /// A pointer to the thread local storage beginning, can be null if there is no TLS
    pub thread_local_storage_ptr: *const (),
    /// The size of the thread local storage area
    pub thread_local_storage_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
/// ABI structures are structures that are passed to processes by the parent process
/// for now only stdio file descriptors are passed
/// you get a pointer to them in the `r8` register at _start (the 5th argument)
pub struct AbiStructures {
    pub stdio: ProcessStdio,
    /// The PID of the parent process of this thread
    pub parent_process_pid: u32,
    /// The number of available CPUs for this process (currently the number of available CPUs in the system)
    pub available_cpus: u32,
}

impl AbiStructures {
    pub fn new(stdio: ProcessStdio, parent_pid: u32, available_cpus: u32) -> Self {
        Self {
            available_cpus,
            stdio,
            parent_process_pid: parent_pid,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ProcessStdio {
    pub stdout: COption<usize>,
    pub stdin: COption<usize>,
    pub stderr: COption<usize>,
}

impl ProcessStdio {
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
pub struct RawPSpawnConfig {
    /// config version for compatibility
    /// added in kernel version 0.2.1 and therefore breaking compatibility with any program compiled for version below 0.2.1
    /// revision 1: added env
    /// revision 2: added priority (v0.4.0)
    /// revision 3: added custom stack size (v0.4.0)
    pub revision: u8,
    pub name: OptZero<Str>,
    pub argv: OptZero<Slice<Str>>,
    pub flags: SpawnFlags,
    _reserved: [u8; 7],
    pub stdio: OptZero<FFINonNull<ProcessStdio>>,
    /// revision 1 and above
    pub env: OptZero<Slice<Slice<u8>>>,
    /// revision 2 and above
    pub priority: COption<ContextPriority>,
    /// revision 3 and above
    pub custom_stack_size: COption<usize>,
}

impl RawPSpawnConfig {
    #[inline]
    pub fn new(
        name: OptZero<Str>,
        argv: OptZero<Slice<Str>>,
        env: OptZero<Slice<Slice<u8>>>,
        flags: SpawnFlags,
        stdio: OptZero<FFINonNull<ProcessStdio>>,
        priority: COption<ContextPriority>,
        custom_stack_size: COption<usize>,
    ) -> Self {
        Self {
            revision: 3,
            name,
            argv,
            env,
            flags,
            _reserved: [0; 7],
            stdio,
            priority: priority.into(),
            custom_stack_size: custom_stack_size.into(),
        }
    }
}

/// configuration for the thread spawn syscall
/// for now it takes only a single argument pointer which is a pointer to an optional argument, that pointer is going to be passed to the thread as the second argument
#[repr(C)]
pub struct RawTSpawnConfig {
    /// revision 1: added custom stack size
    pub revision: u32,
    __reserved: u32,
    pub argument_ptr: *const (),
    pub priority: COption<ContextPriority>,
    /// The index of the CPU to append to, if it is None the kernel will choose one, use `0` for the boot CPU
    pub cpu: COption<u8>,
    /// revision 1 and above
    pub custom_stack_size: COption<usize>,
}

impl RawTSpawnConfig {
    /// Create a new thread spawn configuration with the latest revision
    pub fn new(
        argument_ptr: *const (),
        priority: Option<ContextPriority>,
        cpu: Option<u8>,
        custom_stack_size: Option<usize>,
    ) -> Self {
        Self {
            revision: 1,
            __reserved: 0,
            argument_ptr,
            priority: priority.into(),
            cpu: cpu.into(),
            custom_stack_size: custom_stack_size.into(),
        }
    }
}
