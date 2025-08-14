use crate::{
    process::resources::{self, Resource, ResourceData, Ri},
    sockets::{self, SocketDomain, SocketKind},
    utils::types::Name,
};

use super::{ErrorStatus, SyscallFFI};
use alloc::sync::Arc;
use macros::syscall_handler;
use safa_abi::sockets::{SockBindAbstractAddr, SockBindAddr, SockCreateFlags};

impl SyscallFFI for SockCreateFlags {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(Self::from_bits_retaining(args as u16))
    }
}

enum Addr {
    Abstract(Name),
}

fn compute_addr(addr: &SockBindAddr, addr_struct_size: usize) -> Result<Addr, ErrorStatus> {
    match addr.kind {
        SockBindAbstractAddr::KIND => {
            let name_length = addr_struct_size
                .checked_sub(size_of::<SockBindAddr>())
                .ok_or(ErrorStatus::TooShort)?;

            let addr = unsafe { &*(addr as *const SockBindAddr as *const SockBindAbstractAddr) };
            let name_bytes = &addr.name[..name_length];

            Ok(Addr::Abstract(
                Name::from_utf8(
                    heapless::Vec::from_slice(name_bytes).map_err(|()| ErrorStatus::StrTooLong)?,
                )
                .map_err(|_| ErrorStatus::InvalidStr)?,
            ))
        }
        _ => Err(ErrorStatus::InvalidArgument),
    }
}

#[syscall_handler]
fn syssock_create(
    domain: u8,
    flags: SockCreateFlags,
    protocol: u32,
    out_resource: Option<&mut Ri>,
) -> Result<(), ErrorStatus> {
    _ = protocol;
    if domain != 0 {
        return Err(ErrorStatus::InvalidArgument);
    }
    let domain = SocketDomain::Unix;

    let kind = if flags.contains(SockCreateFlags::SOCK_SEQPACKET) {
        SocketKind::SeqPacket
    } else {
        SocketKind::Stream
    };

    let can_block = !flags.contains(SockCreateFlags::SOCK_NON_BLOCKING);
    let resource_state = ResourceData::SocketDesc {
        domain,
        kind,
        can_block,
    };

    let resource_id = resources::add_global_resource(resource_state);
    if let Some(out_res) = out_resource {
        *out_res = resource_id;
    }
    Ok(())
}

#[syscall_handler]
fn syssock_listen(sock_resource: Ri, backlog: usize) -> Result<(), ErrorStatus> {
    resources::get_resource_reference(sock_resource, |r| match r.data() {
        ResourceData::ServerSocket(s) => Ok(s.configure_listen_queue(backlog)),
        _ => Err(ErrorStatus::UnsupportedResource),
    })
    .ok_or(ErrorStatus::UnknownResource)
    .flatten()
}

#[syscall_handler]
fn syssock_accept(
    sock_resource: Ri,
    addr: Option<&mut SockBindAddr>,
    addr_struct_size: Option<&mut usize>,
    out_connection_id: Option<&mut Ri>,
) -> Result<(), ErrorStatus> {
    _ = addr_struct_size;
    assert!(
        addr.is_none(),
        "Accepting from a specific Address is unimplemented"
    );

    resources::get_resource(sock_resource, |r| match r.data() {
        ResourceData::ServerSocket(serv) => {
            let conn = serv.accept()?;
            let conn_ri = resources::add_global_resource(ResourceData::ServerSocketConn(conn));
            if let Some(out) = out_connection_id {
                *out = conn_ri;
            }
            Ok(())
        }
        _ => Err(ErrorStatus::UnsupportedResource),
    })
}

#[syscall_handler]
fn syssock_connect(
    sock_resource: Ri,
    addr: &SockBindAddr,
    addr_struct_size: usize,
    out_connection_id: Option<&mut Ri>,
) -> Result<(), ErrorStatus> {
    let (domain, kind, can_block) =
        resources::get_resource_reference(sock_resource, |res| match res.data() {
            ResourceData::SocketDesc {
                domain,
                kind,
                can_block,
            } => Ok((*domain, *kind, *can_block)),
            _ => Err(ErrorStatus::UnsupportedResource),
        })
        .ok_or(ErrorStatus::UnknownResource)
        .flatten()?;

    let addr = compute_addr(addr, addr_struct_size)?;
    let sock_id = match addr {
        Addr::Abstract(ref name) => sockets::get_abstract_binding(name),
    }
    .ok_or(ErrorStatus::AddressNotFound)?;

    let client_sock = sockets::get_client_socket(sock_id)
        .expect("Socket dropped but the binded address wasn't dropped");

    if (client_sock.can_block() != can_block)
        || (client_sock.domain() != domain)
        || (client_sock.sock_type() != kind)
    {
        return Err(ErrorStatus::TypeMismatch);
    }

    let client_conn = client_sock.connect()?;

    let ri = resources::add_global_resource(ResourceData::ClientSocketConn(client_conn));
    if let Some(out) = out_connection_id {
        *out = ri;
    }
    Ok(())
}

#[syscall_handler]
fn syssock_bind(
    sock_resource: Ri,
    addr: &SockBindAddr,
    addr_struct_size: usize,
) -> Result<(), ErrorStatus> {
    if addr_struct_size < size_of::<SockBindAddr>() {
        return Err(ErrorStatus::TooShort);
    }

    // Operation is non blocking so it is ok to do this
    let (id, addr) = resources::get_resource_mut(sock_resource, |res| match res.data() {
        ResourceData::SocketDesc {
            domain,
            kind,
            can_block,
        } => {
            let addr = compute_addr(addr, addr_struct_size)?;
            let created_socket = sockets::create_socket(*domain, *kind, *can_block);
            let id = created_socket.id;
            *res = Arc::new(Resource::new_global(ResourceData::ServerSocket(
                created_socket,
            )));

            Ok((id, addr))
        }
        ResourceData::ServerSocket(s) => Ok((s.id, compute_addr(addr, addr_struct_size)?)),
        _ => Err(ErrorStatus::UnsupportedResource),
    })
    .ok_or(ErrorStatus::UnknownResource)
    .flatten()?;

    match addr {
        Addr::Abstract(abs) => sockets::bind_abstract_socket(abs.clone(), id),
    }

    Ok(())
}
