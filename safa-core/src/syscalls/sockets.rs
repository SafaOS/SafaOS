use core::{ops::Deref, ptr::NonNull};

use crate::{
    process::resources::{self, Ri},
    sockets::{
        OwnedSocketAddr, SocketAddrRef, SocketError, SocketFamily, SocketKind, SocketResource,
    },
    syscalls::ffi::ExpectedResource,
};

use super::{ErrorStatus, SyscallFFI};
use macros::syscall_handler;
use safa_abi::sockets::{InetV4SocketAddr, SockCreateKind, SockMsgFlags, SocketAddr};

impl SyscallFFI for SockCreateKind {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(Self::from_bits(args as u16))
    }
}

impl SyscallFFI for safa_abi::sockets::SockDomain {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        unsafe { Ok(core::mem::transmute(args as u8)) }
    }
}

impl SyscallFFI for safa_abi::sockets::SockMsgFlags {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(SockMsgFlags::from_bits(args as u32))
    }
}

pub struct SocketRi(ExpectedResource<SocketResource>);

impl SyscallFFI for SocketRi {
    type Args = Ri;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        ExpectedResource::make(args).map(Self)
    }
}

impl Deref for SocketRi {
    type Target = SocketResource;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[syscall_handler]
fn syssock_sendto(
    sock: SocketRi,
    buf: &[u8],
    flags: SockMsgFlags,
    addr: Option<SocketAddrRef>,
) -> Result<usize, SocketError> {
    match addr {
        Some(addr) => sock.send_to(buf, flags, addr),
        None => sock.send(buf, flags),
    }
}

#[syscall_handler]
fn syssock_create(
    domain: safa_abi::sockets::SockDomain,
    flags: SockCreateKind,
    protocol: u32,
) -> Result<Ri, ErrorStatus> {
    _ = protocol;

    let family = match domain {
        safa_abi::sockets::SockDomain::LOCAL => SocketFamily::Local,
        safa_abi::sockets::SockDomain::INETV4 => SocketFamily::Net,
        _ => return Err(ErrorStatus::InvalidArgument),
    };

    let kind = if flags.contains(SockCreateKind::SOCK_SEQPACKET) {
        SocketKind::SeqPacket
    } else if flags.contains(SockCreateKind::SOCK_DGRAM) {
        SocketKind::Datagram
    } else {
        SocketKind::Stream
    };

    let can_block = !flags.contains(SockCreateKind::SOCK_NON_BLOCKING);
    crate::sockets::create_socket(family, kind, protocol, can_block)
        .map(resources::add_global_resource)
}

#[syscall_handler]
fn syssock_listen(sock: SocketRi, backlog: usize) -> Result<(), SocketError> {
    sock.listen(backlog)
}

fn out_addr(
    out_sock_addr_ptr: NonNull<SocketAddr>,
    out_sock_addr_size: &mut usize,
    value: OwnedSocketAddr,
) {
    let given_size = *out_sock_addr_size;
    let mut out_sock_addr_size = NonNull::from_mut(out_sock_addr_size);
    let out_bytes = unsafe {
        core::slice::from_raw_parts_mut(out_sock_addr_ptr.cast::<u8>().as_ptr(), given_size)
    };

    let size = match value {
        OwnedSocketAddr::Ip { addr, port } => {
            let abi_struct = InetV4SocketAddr::new(port, addr);
            let abi_struct_size = size_of::<InetV4SocketAddr>();

            let abi_bytes = abi_struct.as_bytes();
            let copy_len = out_bytes.len().min(abi_struct_size);

            out_bytes[..copy_len].copy_from_slice(&abi_bytes[..copy_len]);
            abi_struct_size
        }
    };

    unsafe { *out_sock_addr_size.as_mut() = size }
}

#[syscall_handler]
fn syssock_recv_from(
    sock: SocketRi,
    buf: &mut [u8],
    flags: SockMsgFlags,
    out_sock_addr: Option<&mut (Option<NonNull<SocketAddr>>, usize)>,
) -> Result<usize, SocketError> {
    match out_sock_addr {
        Some((Some(sock_addr_ptr), sock_addr_size)) => {
            let (received, addr) = sock.recv_from(buf, flags)?;
            if let Some(owned_addr) = addr {
                out_addr(*sock_addr_ptr, sock_addr_size, owned_addr);
            } else {
                *sock_addr_size = 0;
            }

            Ok(received)
        }
        _ => sock.receive(buf, flags),
    }
}

#[syscall_handler]
fn syssock_accept(
    sock: SocketRi,
    out_sock_addr: Option<&mut (Option<NonNull<SocketAddr>>, usize)>,
) -> Result<Ri, SocketError> {
    let resource = match out_sock_addr {
        Some((Some(sock_addr_ptr), sock_addr_size)) => {
            let (ri, addr) = sock.accept_and_get_addr()?;
            if let Some(owned_addr) = addr {
                out_addr(*sock_addr_ptr, sock_addr_size, owned_addr);
            } else {
                *sock_addr_size = 0;
            }

            ri
        }
        _ => sock.accept()?,
    };
    Ok(resources::add_global_resource(resource))
}

#[syscall_handler]
fn syssock_connect(sock: SocketRi, addr: SocketAddrRef) -> Result<(), SocketError> {
    sock.connect(addr)
}

#[syscall_handler]
fn syssock_bind(sock: SocketRi, addr: SocketAddrRef) -> Result<(), SocketError> {
    sock.bind(addr)
}
