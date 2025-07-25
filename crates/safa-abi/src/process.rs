//! Process & Thread related ABI structures
use core::num::NonZero;
use core::{ops::BitOr, ptr::NonNull};

use crate::ffi::num::ShouldNotBeZero;
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
pub enum RawContextPriority {
    /// Default priority, used when no custom priority is specified, the behaviour is not clearly defined if you choose this priority, but it should be the default,
    /// currently the kernel will use the Medium priority if this was passed to the SysPSpawn syscall, later on it could inherit the priority of the parent process
    ///
    /// Incase of the SysTSpawn syscall, the kernel will inherit the priority of the parent process
    Default = 0,
    Low = 1,
    Medium = 2,
    High = 3,
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
    _reserved: [u8; 7],
    pub name: OptZero<Str>,
    pub argv: OptZero<Slice<Str>>,
    pub flags: SpawnFlags,
    _reserved1: [u8; 7],
    pub stdio: OptZero<FFINonNull<ProcessStdio>>,
    /// revision 1 and above
    pub env: OptZero<Slice<Slice<u8>>>,
    /// revision 2 and above
    pub priority: RawContextPriority,
    _reserved2: [u8; 7],
    /// revision 3 and above
    pub custom_stack_size: OptZero<ShouldNotBeZero<usize>>,
}

impl RawPSpawnConfig {
    /// Creates a new process spawn configuration with the latest revision from raw FFI values
    #[inline(always)]
    pub const fn new_from_raw(
        name: OptZero<Str>,
        argv: OptZero<Slice<Str>>,
        env: OptZero<Slice<Slice<u8>>>,
        flags: SpawnFlags,
        stdio: OptZero<FFINonNull<ProcessStdio>>,
        priority: RawContextPriority,
        custom_stack_size: OptZero<ShouldNotBeZero<usize>>,
    ) -> Self {
        Self {
            revision: 3,
            name,
            argv,
            env,
            flags,
            stdio,
            priority,
            custom_stack_size,
            _reserved2: [0; 7],
            _reserved1: [0; 7],
            _reserved: [0; 7],
        }
    }

    #[inline]
    /// Construct a new process spawn configuration from the given rust parameters.
    ///
    /// # Safety
    /// The given parameters must live as long as the returned value is alive, because currently this doesn't have any lifetime bounds.
    ///
    /// `argv` and `envv` may be reused to store their FFI representations,
    /// even though nothing should change because the layout matches the rust representation, it is still not exactly guaranteed
    /// but this should be resolved through this [RFC](https://github.com/rust-lang/rfcs/pull/3775)
    pub unsafe fn new<'a>(
        name: Option<&'a str>,
        argv: Option<&'a mut [*mut str]>,
        envv: Option<&'a mut [*mut [u8]]>,
        flags: SpawnFlags,
        stdio: Option<&'a ProcessStdio>,
        priority: RawContextPriority,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Self {
        Self::new_from_raw(
            OptZero::from_option(name.map(|s| Str::from_str(s))),
            OptZero::from_option(argv.map(|v| unsafe { Slice::from_str_slices_mut(v) })),
            OptZero::from_option(envv.map(|v| unsafe { Slice::from_slices_ptr_mut(v) })),
            flags,
            OptZero::from_option(
                stdio.map(|v| unsafe { FFINonNull::new_unchecked(v as *const _ as *mut _) }),
            ),
            priority,
            OptZero::from_option(custom_stack_size.map(|v| v.into())),
        )
    }
}

/// configuration for the thread spawn syscall
/// for now it takes only a single argument pointer which is a pointer to an optional argument, that pointer is going to be passed to the thread as the second argument
#[repr(C)]
pub struct RawTSpawnConfig {
    /// revision 1: added custom stack size
    pub revision: u32,
    _reserved: u32,
    pub argument_ptr: *const (),
    pub priority: RawContextPriority,
    /// The index of the CPU to append to, if it is None the kernel will choose one, use `0` for the boot CPU
    pub cpu: COption<u8>,
    _reserved1: [u8; 5],
    /// revision 1 and above
    pub custom_stack_size: OptZero<ShouldNotBeZero<usize>>,
}

impl RawTSpawnConfig {
    #[inline(always)]
    /// Creates a new thread spawn configuration with the latest revision from raw FFI values
    pub const fn new_from_raw(
        argument_ptr: *const (),
        priority: RawContextPriority,
        cpu: COption<u8>,
        custom_stack_size: OptZero<ShouldNotBeZero<usize>>,
    ) -> Self {
        Self {
            revision: 1,
            _reserved1: [0; 5],
            _reserved: 0,
            argument_ptr,
            priority,
            cpu,
            custom_stack_size,
        }
    }

    #[inline]
    /// Create a new thread spawn configuration with the latest revision
    pub fn new(
        argument_ptr: *const (),
        priority: RawContextPriority,
        cpu: Option<u8>,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Self {
        Self::new_from_raw(
            argument_ptr,
            priority.into(),
            cpu.into(),
            match custom_stack_size {
                Some(size) => OptZero::some(unsafe { ShouldNotBeZero::new_unchecked(size.get()) }),
                None => OptZero::none(),
            },
        )
    }
}
