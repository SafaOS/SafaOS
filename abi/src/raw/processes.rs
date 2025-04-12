use core::ops::BitOr;

use super::{Optional, RawSlice, RawSliceMut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct TaskMetadata {
    pub stdout: Optional<usize>,
    pub stdin: Optional<usize>,
    pub stderr: Optional<usize>,
}

impl TaskMetadata {
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

/// configuration for the spawn syscall
#[repr(C)]
pub struct SpawnConfig {
    /// config version for compatibility
    /// added in kernel version 0.2.1 and therfore breaking compatibility with any program compiled for version below 0.2.1
    pub version: u8,
    pub name: RawSlice<u8>,
    pub argv: RawSliceMut<RawSlice<u8>>,
    pub flags: SpawnFlags,
    pub metadata: *const TaskMetadata,
    pub env: RawSliceMut<RawSlice<u8>>,
}

impl SpawnConfig {
    pub fn new(
        name: &str,
        argv: *mut [&[u8]],
        env: *mut [&[u8]],
        flags: SpawnFlags,
        metadata: Option<&TaskMetadata>,
    ) -> Self {
        let name = unsafe { RawSlice::from_slice(name.as_bytes()) };
        let argv = unsafe { RawSliceMut::from_slices(argv) };
        let env = unsafe { RawSliceMut::from_slices(env) };

        Self {
            version: 1,
            name,
            argv,
            env,
            flags,
            metadata: metadata
                .map(|x| x as *const TaskMetadata)
                .unwrap_or(core::ptr::null()),
        }
    }
}
