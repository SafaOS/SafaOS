use core::ops::{BitAnd, BitOr};

/// Passed to [`crate::syscalls::SyscallTable::SysMemMap`], to describe the address, the size, of the area to map to, and the resource it is associated with
#[repr(C)]
pub struct RawMemMapConfig {
    /// Address hint can be null to indicate no hint
    pub addr_hint: *const (),
    /// The amount of pages to map
    pub page_count: usize,
    /// The amount of unmapped guard pages before and after the area
    pub guard_pages_count: usize,
    /// The ID of the associated Resource to map, ignored unless the flag [`MemMapFlags::MAP_RESOURCE`] was given
    pub resource_to_map: usize,
    /// If Mapping A Resource, Describes the offset to where the memory mapping
    pub resource_off: isize,
}

/// Flags passed to [`crate::syscalls::SyscallTable::SysMemMap`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct MemMapFlags(u8);

impl MemMapFlags {
    /// The given address is a just a hint unless specified with this flag
    pub const FIXED: Self = Self(1 << 0);
    /// Allows the memory to be written to, makes it RW, by default it is only R
    pub const WRITE: Self = Self(1 << 1);
    /// Don't allow the memory to be executed
    pub const DISABLE_EXEC: Self = Self(1 << 2);
    /// The given Resource ID is a valid resource to map, by default the given resource ID is ignored and the memory is treated as not associated with anything
    pub const MAP_RESOURCE: Self = Self(1 << 3);
}

impl MemMapFlags {
    pub fn from_bits_retaining(bits: u8) -> Self {
        Self(bits)
    }

    pub fn contains(self, other: MemMapFlags) -> bool {
        (self & other) == other
    }
}

impl BitOr for MemMapFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for MemMapFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

/// Flags passed to [`crate::syscalls::SyscallTable::SysMemShmCreate`] and [`crate::syscalls::SyscallTable::SysMemShmOpen`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ShmFlags(u32);

impl ShmFlags {
    /// The opened Resource's lifetime is bound by the Current Thread and not the Process
    pub const LOCAL: Self = Self(1 << 0);
}

impl ShmFlags {
    pub const fn from_bits_retaining(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl BitOr for ShmFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for ShmFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}
