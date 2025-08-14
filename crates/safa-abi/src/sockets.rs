use crate::consts::MAX_NAME_LENGTH;

#[repr(C)]
/// Configures the Socket Binding Address
///
/// The actual structure varries for each binding kind, and each family excepts a specific set of kinds
pub struct SockBindAddr {
    pub kind: u32,
}

#[repr(C)]
/// An Abstract binding, converted from [SockBindAddr]
pub struct SockBindAbstractAddr {
    kind: u32,
    /// Must be valid UTF-8, the actual length is provided to SysSockBind
    pub name: [u8; MAX_NAME_LENGTH],
}

impl SockBindAbstractAddr {
    pub const KIND: u32 = 0;
    /// Creates a new abstract binding Addr from a given name bytes,
    /// name[..name_length] must be valid UTF8 where name_length is
    ///
    /// This structures total length - size_of::<[`SockBindAbstractAddr`]>()
    /// The structures total length is passed to SysSockBind
    pub const fn new(name: [u8; MAX_NAME_LENGTH]) -> Self {
        Self {
            kind: Self::KIND,
            name,
        }
    }
}

use core::ops::BitOr;

/// Flags given to [`crate::syscalls::SyscallTable::SysSockCreate`],
/// Also contains information about the Socket Type, by default the Socket Type is SOCK_STREAM and blocking unless a flag was given
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SockCreateFlags(u16);

impl SockCreateFlags {
    /// A SeqPacket Socket, unlike Stream Sockets which are the default, this preserves messages boundaries
    pub const SOCK_SEQPACKET: Self = Self(1);
    /// A Non Blocking Socket, anything that would normally block would return [`crate::errors::ErrorStatus::WouldBlock`] instead of blocking
    /// except for [`crate::syscalls::SyscallTable::SysSockConnect`],
    /// this one is defined by POSIX as not blockable but it is way too hard to implement ._.
    pub const SOCK_NON_BLOCKING: Self = Self(1 << 15);

    /// returns true If self contains the flags other containsa
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn from_bits_retaining(bits: u16) -> Self {
        Self(bits)
    }
}

impl BitOr for SockCreateFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}
