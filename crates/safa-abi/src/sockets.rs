use crate::consts::MAX_NAME_LENGTH;

pub unsafe trait ToSocketAddr {
    /// The family of the socket address, corresponding to a [`SockDomain`].
    const FAMILY: u32 = Self::DOMAIN.0 as u32;
    const DOMAIN: SockDomain = SockDomain(Self::FAMILY as u8);
    /// Converts this address to a generic [SocketAddr].
    fn as_generic(&self) -> &SocketAddr {
        unsafe { &*(self as *const Self as *const SocketAddr) }
    }
    /// Converts this address to a generic [SocketAddr].
    fn as_generic_mut(&mut self) -> &mut SocketAddr {
        unsafe { &mut *(self as *mut Self as *mut SocketAddr) }
    }
    /// Converts this address to a generic NonNull pointer to [SocketAddr].
    fn as_non_null(&mut self) -> NonNull<SocketAddr> {
        unsafe { NonNull::new_unchecked(self.as_generic_mut()) }
    }
}

#[repr(C)]
/// A Socket Address
///
/// The actual structure varries for each family.
pub struct SocketAddr {
    pub sin_family: u32,
}

impl SocketAddr {
    pub fn as_known<T: ToSocketAddr>(&self) -> Option<&T> {
        if self.sin_family == T::FAMILY {
            Some(unsafe { &*(self as *const Self as *const T) })
        } else {
            None
        }
    }

    pub fn as_known_mut<T: ToSocketAddr>(&mut self) -> Option<&mut T> {
        if self.sin_family == T::FAMILY {
            Some(unsafe { &mut *(self as *mut Self as *mut T) })
        } else {
            None
        }
    }
}

#[repr(C)]
/// A local family socket address, converted from [SocketAddr]
pub struct LocalSocketAddr {
    sin_family: u32,
    /// Must be valid UTF-8, the actual length is provided to socket syscalls.
    pub sin_name: [u8; MAX_NAME_LENGTH],
}

unsafe impl ToSocketAddr for LocalSocketAddr {
    const DOMAIN: SockDomain = SockDomain::LOCAL;
}

impl LocalSocketAddr {
    /// Creates a new abstract binding Addr from a given name bytes,
    /// name[..name_length] must be valid UTF8 where name_length is
    ///
    /// This structures total length - size_of::<[`SocketAddr`]>()
    /// The structures total length is passed to sockets syscalls.
    pub const fn new(name: [u8; MAX_NAME_LENGTH]) -> Self {
        Self {
            sin_family: Self::FAMILY,
            sin_name: name,
        }
    }

    /// Creates a new abstract binding Addr from a given name,
    /// name must be valid UTF8
    ///
    /// returns the actual structure length.
    ///
    /// Panicks if name.len() > MAX_NAME_LENGTH
    pub fn new_abstract_from(name: &str) -> (Self, usize) {
        let mut bytes = [0u8; MAX_NAME_LENGTH];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        (
            Self {
                sin_family: Self::FAMILY,
                sin_name: bytes,
            },
            name.len() + size_of::<SocketAddr>(),
        )
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { &*(self as *const Self as *const [u8; size_of::<Self>()]) }
    }
}

/// An IpV4 socket address, converted from [SocketAddr]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InetV4SocketAddr {
    sin_family: u32,
    pub sin_port: u16,
    pub sin_addr: Ipv4Addr,
}

unsafe impl ToSocketAddr for InetV4SocketAddr {
    const DOMAIN: SockDomain = SockDomain::INETV4;
}

impl InetV4SocketAddr {
    pub const fn new(port: u16, addr: Ipv4Addr) -> Self {
        Self {
            sin_family: Self::FAMILY,
            sin_port: port.to_be(),
            sin_addr: addr,
        }
    }

    pub const fn ip(&self) -> Ipv4Addr {
        self.sin_addr
    }

    pub const fn port(&self) -> u16 {
        u16::from_be(self.sin_port)
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { &*(self as *const Self as *const [u8; size_of::<Self>()]) }
    }
}

use core::{
    net::Ipv4Addr,
    ops::{BitAnd, BitOr, Not},
    ptr::NonNull,
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
    /// A stream socket, only allowed for local domain sockets.
    pub const SOCK_STREAM: Self = Self(0);
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
