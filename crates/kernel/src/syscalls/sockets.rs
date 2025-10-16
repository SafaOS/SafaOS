use core::{ops::Deref, ptr::NonNull};

use crate::{
    process::resources::Ri,
    sockets::{
        OwnedSocketAddr, SocketAddrRef, SocketError, SocketFamily, SocketKind, SocketResourceTrait,
    },
    syscalls::ffi::ResourceDesc,
};

use super::{ErrorStatus, SyscallFFI};
use macros::syscall_handler;
use safa_abi::sockets::{
    SockBindAbstractAddr, SockBindAddr, SockBindInetV4Addr, SockCreateKind, SockMsgFlags,
};

impl SyscallFFI for SockCreateKind {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(Self::from_bits_retaining(args as u16))
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
        Ok(SockMsgFlags::from_bits_retaining(args as u32))
    }
}

pub struct Socket {
    _inner_desc: ResourceDesc,
    socket_ref: NonNull<dyn SocketResourceTrait>,
}

impl SyscallFFI for Socket {
    type Args = Ri;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let desc = ResourceDesc::make(args)?;
        let socket_ref = desc.as_socket().ok_or(ErrorStatus::UnsupportedResource)?;
        // Safety: ResourceDesc lives on the heap and any borrow of it shall be valid as long as its alive, regardless of moving.
        let socket_ref = NonNull::from_ref(socket_ref);
        Ok(Self {
            _inner_desc: desc,
            socket_ref,
        })
    }
}

impl Deref for Socket {
    type Target = dyn SocketResourceTrait;
    fn deref(&self) -> &Self::Target {
        unsafe { self.socket_ref.as_ref() }
    }
}

#[syscall_handler]
fn syssock_sendto(
    sock: Socket,
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
}

#[syscall_handler]
fn syssock_listen(sock: Socket, backlog: usize) -> Result<(), SocketError> {
    sock.listen(backlog)
}

fn out_addr(
    out_sock_addr_ptr: NonNull<SockBindAddr>,
    out_sock_addr_size: &mut usize,
    value: OwnedSocketAddr,
) {
    let given_size = *out_sock_addr_size;
    let mut out_sock_addr_size = NonNull::from_mut(out_sock_addr_size);
    let out_bytes = unsafe {
        core::slice::from_raw_parts_mut(out_sock_addr_ptr.cast::<u8>().as_ptr(), given_size)
    };

    match value {
        OwnedSocketAddr::Abstract(name) => {
            let name_len = name.len();
            let mut abi_struct = SockBindAbstractAddr::new([0u8; _]);
            abi_struct.name[..name_len].copy_from_slice(name.as_bytes());
            let abi_struct_size = name_len + size_of::<SockBindAddr>();
            let abi_bytes: [u8; size_of::<SockBindAbstractAddr>()] =
                unsafe { core::mem::transmute(abi_struct) };

            let copy_len = out_bytes.len().min(abi_struct_size);
            out_bytes[..copy_len].copy_from_slice(&abi_bytes[..copy_len]);
            unsafe { *out_sock_addr_size.as_mut() = abi_struct_size }
        }
        OwnedSocketAddr::Ip { addr, port } => {
            let abi_struct = SockBindInetV4Addr::new(port, addr);
            let abi_struct_size = size_of::<SockBindInetV4Addr>();

            let abi_bytes: [u8; size_of::<SockBindInetV4Addr>()] =
                unsafe { core::mem::transmute(abi_struct) };
            let copy_len = out_bytes.len().min(abi_struct_size);
            out_bytes[..copy_len].copy_from_slice(&abi_bytes[..copy_len]);
            unsafe { *out_sock_addr_size.as_mut() = abi_struct_size }
        }
    }
}

#[syscall_handler]
fn syssock_recv_from(
    sock: Socket,
    buf: &mut [u8],
    flags: SockMsgFlags,
    out_sock_addr: Option<&mut (Option<NonNull<SockBindAddr>>, usize)>,
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
    sock: Socket,
    out_sock_addr: Option<&mut (Option<NonNull<SockBindAddr>>, usize)>,
) -> Result<Ri, SocketError> {
    let ri = match out_sock_addr {
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
    Ok(ri)
}

#[syscall_handler]
fn syssock_connect(sock: Socket, addr: SocketAddrRef) -> Result<(), SocketError> {
    sock.connect(addr)
}

#[syscall_handler]
fn syssock_bind(sock: Socket, addr: SocketAddrRef) -> Result<(), SocketError> {
    sock.bind(addr)
}
