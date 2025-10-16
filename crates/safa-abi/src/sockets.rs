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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockBindInetV4Addr {
    kind: u32,
    pub port: u16,
    pub ip: Ipv4Addr,
}

impl SockBindInetV4Addr {
    pub const KIND: u32 = 1;

    pub const fn new(port: u16, addr: Ipv4Addr) -> Self {
        Self {
            kind: Self::KIND,
            port,
            ip: addr,
        }
    }
}

use core::{
    net::Ipv4Addr,
    ops::{BitAnd, BitOr, Not},
};

/// Domain given to [`crate::syscalls::SyscallTable::SysSockCreate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SockDomain(u8);

impl SockDomain {
    /// Unix Domain sockets
    pub const LOCAL: Self = Self(0);
    /// The Internet Domain, IPv4
    pub const INETV4: Self = Self(1);
}

/// Flags given to [`crate::syscalls::SyscallTable::SysSockCreate`],
/// Also contains information about the Socket Type, by default the Socket Type is SOCK_STREAM and blocking unless a flag was given
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SockCreateKind(u16);

impl SockCreateKind {
    /// A SeqPacket Socket, unlike Stream Sockets which are the default for local sockets, this preserves messages boundaries
    pub const SOCK_SEQPACKET: Self = Self(1);
    /// A Datagram Socket, only allowed for network domain sockets, UDP by default and preserves messages boundaries.
    pub const SOCK_DGRAM: Self = Self(2);
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

impl BitOr for SockCreateKind {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Flags for a message transmitted to and received from a socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SockMsgFlags(u32);

impl SockMsgFlags {
    pub const NONE: Self = Self(0);
    /// Return an error if sending/receiving the message would block instead of blocking.
    pub const DONT_WAIT: Self = Self(1);
    /// For a receive operation, only read the message without removing it from the queue, so another receive operation would read the same exact message.
    pub const PEEK: Self = Self(1);

    /// Returns true If self contains the flags other containsa
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn from_bits_retaining(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn into_bits(self) -> u32 {
        self.0
    }
}

impl BitOr for SockMsgFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for SockMsgFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl Not for SockMsgFlags {
    type Output = Self;
    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}
