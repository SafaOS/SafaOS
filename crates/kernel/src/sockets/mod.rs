use core::{net::Ipv4Addr, ops::Deref};

use alloc::sync::Arc;
use int_enum::IntEnum;
use safa_abi::{
    errors::{ErrorStatus, IntoErr},
    sockets::SockMsgFlags,
};

use crate::{
    net::manager::NetworkError,
    process::{
        poll::PollID,
        resources::{self, Resource, Ri},
    },
    scheduler::wait_queue::WaitError,
    sockets::{
        udp::UdpSocket,
        unix::{LocalSocket, LocalSocketKind},
    },
    syscalls::ffi::SyscallFFI,
    utils::types::Name,
};

pub mod udp;
pub mod unix;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    /// Should Block because the Socket is empty and we are trying to read from it
    WouldBlockEmpty,
    /// Should Block because the Socket is full and we are trying to append to it
    WouldBlockFull,
    /// Should Block because of an attempted accept while there are no connection requests pending
    WouldBlockNoConnectionRequests,
    /// One side closed the connection
    ConnectionClosed,
    /// Connection refused for some reason
    ConnectionRefused,
    OperationNotSupported,
    ForceTerminated,
    AddressInUse,
    AlreadyBinded,
    UnknownAddress,
    InvalidArgument,
    TypeMismatch,
    NotBound,
}

impl From<WaitError> for SocketError {
    fn from(value: WaitError) -> Self {
        match value {
            WaitError::ForceTerminated => Self::ForceTerminated,
            WaitError::Timeout => unreachable!(),
        }
    }
}

impl From<NetworkError> for SocketError {
    fn from(value: NetworkError) -> Self {
        match value {
            NetworkError::AddressNotFound | NetworkError::NoInterface => Self::UnknownAddress,
            NetworkError::PayloadTooLarge => Self::InvalidArgument,
        }
    }
}

impl IntoErr for SocketError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::WouldBlockEmpty | Self::WouldBlockFull | Self::WouldBlockNoConnectionRequests => {
                safa_abi::errors::ErrorStatus::WouldBlock
            }
            Self::ConnectionRefused => safa_abi::errors::ErrorStatus::ConnectionRefused,
            Self::ConnectionClosed => safa_abi::errors::ErrorStatus::ConnectionClosed,
            Self::OperationNotSupported | Self::AlreadyBinded => {
                safa_abi::errors::ErrorStatus::OperationNotSupported
            }
            Self::ForceTerminated => safa_abi::errors::ErrorStatus::ForceTerminated,
            Self::AddressInUse => safa_abi::errors::ErrorStatus::AddressAlreadyInUse,
            Self::UnknownAddress => safa_abi::errors::ErrorStatus::AddressNotFound,
            Self::InvalidArgument => safa_abi::errors::ErrorStatus::InvalidArgument,
            Self::TypeMismatch => safa_abi::errors::ErrorStatus::TypeMismatch,
            Self::NotBound => safa_abi::errors::ErrorStatus::NotBound,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SocketAddrRef<'a> {
    Abstract(&'a str),
    Ip { addr: Ipv4Addr, port: u16 },
}

#[derive(Debug, Clone)]
pub enum OwnedSocketAddr {
    #[allow(unused)]
    Abstract(Name),
    Ip {
        addr: Ipv4Addr,
        port: u16,
    },
}

use safa_abi::sockets::SocketAddr as AbiSockAddr;
impl<'a> SocketAddrRef<'a> {
    pub fn from_raw(
        addr: &'a safa_abi::sockets::SocketAddr,
        addr_struct_size: usize,
    ) -> Result<Self, ErrorStatus> {
        use safa_abi::sockets::InetV4SocketAddr as AbiIpV4Addr;
        use safa_abi::sockets::LocalSocketAddr as AbiAbstractAddr;

        match addr.family {
            AbiAbstractAddr::FAMILY => {
                let name_len = addr_struct_size
                    .checked_sub(size_of::<AbiSockAddr>())
                    .ok_or(ErrorStatus::InvalidArgument)?;

                let as_abs: &'a AbiAbstractAddr = unsafe { core::mem::transmute(addr) };
                let name_bytes = &as_abs.name[..name_len];
                let name_utf8 = str::from_utf8(name_bytes)?;

                Ok(SocketAddrRef::Abstract(name_utf8))
            }
            AbiIpV4Addr::FAMILY => {
                if size_of::<AbiIpV4Addr>() != addr_struct_size {
                    return Err(ErrorStatus::InvalidArgument);
                }

                let as_ipv4: &'a AbiIpV4Addr = unsafe { core::mem::transmute(addr) };
                Ok(SocketAddrRef::Ip {
                    addr: as_ipv4.ip,
                    port: as_ipv4.port,
                })
            }
            _ => Err(ErrorStatus::InvalidArgument),
        }
    }
}

impl<'a> SyscallFFI for SocketAddrRef<'a> {
    type Args = (*const AbiSockAddr, usize);
    fn make((abi_ptr_raw, addr_struct_size): Self::Args) -> Result<Self, ErrorStatus> {
        let abi_ref: &AbiSockAddr = SyscallFFI::make(abi_ptr_raw)?;
        SocketAddrRef::from_raw(abi_ref, addr_struct_size)
    }
}

impl<'a> SyscallFFI for Option<SocketAddrRef<'a>> {
    type Args = (*const AbiSockAddr, usize);
    fn make((abi_ptr_raw, addr_struct_size): Self::Args) -> Result<Self, ErrorStatus> {
        let abi_ref: Option<&AbiSockAddr> = SyscallFFI::make(abi_ptr_raw)?;
        if let Some(abi_ref) = abi_ref {
            SocketAddrRef::from_raw(abi_ref, addr_struct_size).map(|ok| Some(ok))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntEnum)]
#[repr(u16)]
pub enum SocketOpt {
    Blocking = 0,
    ReadTimeout = 1,
    WriteTimeout = 2,
    IpTTL = 3,
    SockError = 4,
}

pub trait Socket: 'static {
    fn listen(&self, backlog: usize) -> Result<(), SocketError>;
    fn accept(&self) -> Result<Arc<Self>, SocketError>;
    fn accept_and_get_addr(&self) -> Result<(Arc<Self>, Option<OwnedSocketAddr>), SocketError> {
        self.accept().map(|ok| (ok, None))
    }
    fn bind(&self, addr: SocketAddrRef) -> Result<(), SocketError>;
    fn connect(&self, addr: SocketAddrRef) -> Result<(), SocketError>;

    fn receive(&self, buf: &mut [u8], flags: SockMsgFlags) -> Result<usize, SocketError>;
    fn send(&self, buf: &[u8], flags: SockMsgFlags) -> Result<usize, SocketError>;

    fn send_to(
        &self,
        buf: &[u8],
        flags: SockMsgFlags,
        addr: SocketAddrRef,
    ) -> Result<usize, SocketError>;
    fn recv_from(
        &self,
        buf: &mut [u8],
        flags: SockMsgFlags,
    ) -> Result<(usize, Option<OwnedSocketAddr>), SocketError> {
        self.receive(buf, flags).map(|ok| (ok, None))
    }

    // TODO: use SocketError here?
    fn set_sock_opt(&self, opt: SocketOpt, value: u64) -> Result<(), ErrorStatus>;
    fn get_sock_opt(&self, opt: SocketOpt, to_usr_ptr: *mut ()) -> Result<(), ErrorStatus>;
    fn poll_id(&self) -> PollID;
    /// Clean-up function when the socket's resource is dropped
    fn on_close(&self);
}

/// Wrapper around a [`Socket`], represents a Resource thats a socket.
pub struct SocketResource<T: Socket>(Arc<T>);

impl<T: Socket> SocketResource<T> {
    #[allow(unused)]
    pub const unsafe fn inner(&self) -> &Arc<T> {
        &self.0
    }
}

impl<T: Socket> Drop for SocketResource<T> {
    fn drop(&mut self) {
        self.0.on_close();
    }
}

impl<T: Socket> SocketResource<T> {
    fn accept(&self) -> Result<Self, SocketError> {
        Ok(SocketResource(self.0.accept()?))
    }
    fn accept_and_get_addr(&self) -> Result<(Self, Option<OwnedSocketAddr>), SocketError> {
        self.0
            .accept_and_get_addr()
            .map(|(r, a)| (SocketResource(r), a))
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.0.receive(buf, SockMsgFlags::NONE)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, SocketError> {
        self.0.send(buf, SockMsgFlags::NONE)
    }
}

impl<T: Socket> Deref for SocketResource<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// A trait which describes all [`SocketResource<T>`]s
pub trait SocketResourceTrait: 'static {
    fn listen(&self, backlog: usize) -> Result<(), SocketError>;
    fn accept(&self) -> Result<Ri, SocketError>;
    fn accept_and_get_addr(&self) -> Result<(Ri, Option<OwnedSocketAddr>), SocketError>;
    fn bind(&self, addr: SocketAddrRef) -> Result<(), SocketError>;
    fn connect(&self, addr: SocketAddrRef) -> Result<(), SocketError>;

    fn receive(&self, buf: &mut [u8], flags: SockMsgFlags) -> Result<usize, SocketError>;
    fn send(&self, buf: &[u8], flags: SockMsgFlags) -> Result<usize, SocketError>;

    fn send_to(
        &self,
        buf: &[u8],
        flags: SockMsgFlags,
        addr: SocketAddrRef,
    ) -> Result<usize, SocketError>;
    fn recv_from(
        &self,
        buf: &mut [u8],
        flags: SockMsgFlags,
    ) -> Result<(usize, Option<OwnedSocketAddr>), SocketError>;
}

impl<T: Socket> Resource for SocketResource<T> {
    fn read(
        &self,
        off: crate::drivers::vfs::SeekOffset,
        buf: &mut [u8],
    ) -> Result<usize, safa_abi::errors::ErrorStatus> {
        _ = off;
        Ok(self.read(buf)?)
    }

    fn write(
        &self,
        off: crate::drivers::vfs::SeekOffset,
        buf: &[u8],
    ) -> Result<usize, safa_abi::errors::ErrorStatus> {
        _ = off;
        Ok(self.write(buf)?)
    }

    fn poll_id(&self) -> Option<crate::process::poll::PollID> {
        Some(self.0.poll_id())
    }

    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), safa_abi::errors::ErrorStatus> {
        let cmd = cmd as i16;
        let is_get = cmd.is_negative();

        let opt = SocketOpt::try_from(cmd.unsigned_abs())
            .map_err(|_| safa_abi::errors::ErrorStatus::InvalidCommand)?;
        if is_get {
            self.get_sock_opt(opt, arg as *mut ())
        } else {
            self.set_sock_opt(opt, arg)
        }?;
        Ok(())
    }

    fn address_space_generic(&self) -> bool {
        false
    }

    fn as_socket(&self) -> Option<&dyn SocketResourceTrait> {
        Some(self)
    }
}

impl<T: Socket> SocketResourceTrait for SocketResource<T> {
    fn accept(&self) -> Result<Ri, SocketError> {
        self.accept()
            .map(|good| resources::add_global_resource(good))
    }
    fn accept_and_get_addr(&self) -> Result<(Ri, Option<OwnedSocketAddr>), SocketError> {
        self.accept_and_get_addr()
            .map(|(good, addr)| (resources::add_global_resource(good), addr))
    }

    fn listen(&self, backlog: usize) -> Result<(), SocketError> {
        self.0.listen(backlog)
    }

    fn bind(&self, addr: SocketAddrRef) -> Result<(), SocketError> {
        self.0.bind(addr)
    }

    fn connect(&self, addr: SocketAddrRef) -> Result<(), SocketError> {
        self.0.connect(addr)
    }

    fn recv_from(
        &self,
        buf: &mut [u8],
        flags: SockMsgFlags,
    ) -> Result<(usize, Option<OwnedSocketAddr>), SocketError> {
        self.0.recv_from(buf, flags)
    }

    fn send_to(
        &self,
        buf: &[u8],
        flags: SockMsgFlags,
        addr: SocketAddrRef,
    ) -> Result<usize, SocketError> {
        self.0.send_to(buf, flags, addr)
    }

    fn receive(&self, buf: &mut [u8], flags: SockMsgFlags) -> Result<usize, SocketError> {
        self.0.receive(buf, flags)
    }
    fn send(&self, buf: &[u8], flags: SockMsgFlags) -> Result<usize, SocketError> {
        self.0.send(buf, flags)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SocketFamily {
    Local,
    Net,
}

#[derive(Debug, Clone, Copy)]
pub enum SocketKind {
    Stream,
    SeqPacket,
    Datagram,
}

pub fn create_socket(
    family: SocketFamily,
    kind: SocketKind,
    protocol: u32,
    can_block: bool,
) -> Result<Ri, ErrorStatus> {
    _ = protocol;
    match (family, kind) {
        (SocketFamily::Local, kind) => {
            let local_socket_kind = match kind {
                SocketKind::SeqPacket => LocalSocketKind::SeqPacket,
                SocketKind::Stream => LocalSocketKind::Stream,
                SocketKind::Datagram => return Err(ErrorStatus::TypeMismatch),
            };

            let local_socket = LocalSocket::create(local_socket_kind, can_block);
            let socket_resource = SocketResource(local_socket);
            Ok(resources::add_global_resource(socket_resource))
        }
        (SocketFamily::Net, SocketKind::Datagram) => {
            let udp_socket = UdpSocket::create(can_block);
            let socket_resource = SocketResource(udp_socket);
            Ok(resources::add_global_resource(socket_resource))
        }
        (SocketFamily::Net, _) => return Err(ErrorStatus::TypeMismatch),
    }
}
