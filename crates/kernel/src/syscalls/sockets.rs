use core::net::Ipv4Addr;

use crate::{
    net::ipv4::PageIPv4Packet,
    process::resources::{self, ResourceNode, Ri},
    sockets::{
        self, SocketDomain, SocketKind,
        desc::{ServerSocketDesc, SocketDesc},
    },
    utils::types::Name,
};

use super::{ErrorStatus, SyscallFFI};
use macros::syscall_handler;
use safa_abi::sockets::{SockBindAbstractAddr, SockBindAddr, SockBindInetV4Addr, SockCreateFlags};

impl SyscallFFI for SockCreateFlags {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(Self::from_bits_retaining(args as u16))
    }
}

enum Addr {
    Abstract(Name),
    Inet {
        #[allow(unused)]
        ipv4: Ipv4Addr,
        port: u16,
    },
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
        SockBindInetV4Addr::KIND if addr_struct_size == size_of::<SockBindInetV4Addr>() => {
            let addr = unsafe { &*(addr as *const SockBindAddr as *const SockBindInetV4Addr) };
            let ipv4 = addr.ip;
            let port = addr.port;

            Ok(Addr::Inet { ipv4, port })
        }
        _ => Err(ErrorStatus::InvalidArgument),
    }
}

impl SyscallFFI for safa_abi::sockets::SockDomain {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        unsafe { Ok(core::mem::transmute(args as u8)) }
    }
}

#[syscall_handler]
fn syssock_sendto(
    sock_ri: Ri,
    payload: &[u8],
    addr: &SockBindAddr,
    addr_len: usize,
) -> Result<(), ErrorStatus> {
    let addr = compute_addr(&addr, addr_len)?;
    match addr {
        Addr::Inet { ipv4, port } => {
            let src_port = resources::get_ref(sock_ri, |resource| {
                resource
                    .data()
                    .as_ref::<ServerSocketDesc>()
                    .and_then(|desc| Some((desc.udp_port()?, desc.domain(), desc.sock_type())))
                    .or_else(|| {
                        resource
                            .data()
                            .as_ref::<SocketDesc>()
                            .map(|desc| (0, desc.domain, desc.kind))
                    })
                    .and_then(|(port, domain, kind)| {
                        (domain == SocketDomain::Net && kind == SocketKind::Datagram)
                            .then_some(port)
                    })
                    .ok_or(ErrorStatus::UnsupportedResource)
            })
            .ok_or(ErrorStatus::UnknownResource)
            .flatten()?;

            let mut packet = PageIPv4Packet::new_udp(payload, src_port, port, ipv4);
            crate::net::manager::send_ipv4_packet(&mut *packet)?;
            Ok(())
        }
        Addr::Abstract(_) => Err(ErrorStatus::InvalidArgument),
    }
}

#[syscall_handler]
fn syssock_create(
    domain: safa_abi::sockets::SockDomain,
    flags: SockCreateFlags,
    protocol: u32,
    out_resource: Option<&mut Ri>,
) -> Result<(), ErrorStatus> {
    _ = protocol;

    let domain = match domain {
        safa_abi::sockets::SockDomain::LOCAL => SocketDomain::Unix,
        safa_abi::sockets::SockDomain::INETV4 => SocketDomain::Net,
        _ => return Err(ErrorStatus::InvalidArgument),
    };

    let is_seqpacket = flags.contains(SockCreateFlags::SOCK_SEQPACKET);
    let kind = match domain {
        SocketDomain::Unix => {
            if is_seqpacket {
                SocketKind::SeqPacket
            } else {
                SocketKind::Stream
            }
        }
        SocketDomain::Net if is_seqpacket => return Err(ErrorStatus::TypeMismatch),
        SocketDomain::Net => SocketKind::Datagram,
    };

    let can_block = !flags.contains(SockCreateFlags::SOCK_NON_BLOCKING);

    let res = SocketDesc {
        domain,
        kind,
        can_block,
    };
    let resource_id = resources::add_global_resource(res);

    if let Some(out_res) = out_resource {
        *out_res = resource_id;
    }
    Ok(())
}

#[syscall_handler]
fn syssock_listen(sock_resource: Ri, backlog: usize) -> Result<(), ErrorStatus> {
    resources::get_ref(sock_resource, |r| {
        let s = r.data().as_ref_expected::<ServerSocketDesc>()?;
        Ok(s.configure_listen_queue(backlog)?)
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

    let resource = resources::get_expected(sock_resource)?;

    let serv = resource.data().as_ref_expected::<ServerSocketDesc>()?;
    let conn = serv.accept()?;

    let conn_ri = resources::add_global_resource(conn);
    if let Some(out) = out_connection_id {
        *out = conn_ri;
    }
    Ok(())
}

#[syscall_handler]
fn syssock_connect(
    sock_resource: Ri,
    addr: &SockBindAddr,
    addr_struct_size: usize,
) -> Result<(), ErrorStatus> {
    let socket_desc = resources::get_ref(sock_resource, |res| {
        res.data().as_ref_expected::<SocketDesc>().copied()
    })
    .ok_or(ErrorStatus::UnknownResource)
    .flatten()?;

    let domain = socket_desc.domain;
    let kind = socket_desc.kind;

    let addr = compute_addr(addr, addr_struct_size)?;
    let sock_id = match addr {
        Addr::Abstract(ref name) => sockets::get_abstract_binding(name),
        Addr::Inet { .. } => None, // TODO: TCP sockets...,
    }
    .ok_or(ErrorStatus::AddressNotFound)?;

    let client_sock = sockets::get_client_socket(sock_id)
        .expect("Socket dropped but the binded address wasn't dropped");

    if (client_sock.domain() != domain) || (client_sock.sock_type() != kind) {
        return Err(ErrorStatus::TypeMismatch);
    }

    let client_conn = client_sock.connect(client_sock.can_block())?;
    resources::get_mut(sock_resource, |res| {
        *res = ResourceNode::create(client_conn, res.is_global())
    });
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

    let addr = compute_addr(addr, addr_struct_size)?;
    // Operation is non blocking so it is ok to do this
    let (socket, id) = resources::get_mut(sock_resource, |res| {
        match (
            res.data().as_ref::<ServerSocketDesc>(),
            res.data().as_ref::<SocketDesc>(),
        ) {
            (
                None,
                Some(SocketDesc {
                    domain,
                    kind,
                    can_block,
                }),
            ) => {
                let created_socket_desc = sockets::create_socket(*domain, *kind, *can_block);
                let id = created_socket_desc.id;
                let socket = created_socket_desc.socket().clone();
                *res = ResourceNode::create_global(created_socket_desc);

                Ok((socket, id))
            }
            (Some(s), None) => Ok((s.socket().clone(), s.id)),
            (Some(_), Some(_)) => unreachable!("Schrödinger Resource"),
            (None, None) => Err(ErrorStatus::UnsupportedResource),
        }
    })
    .ok_or(ErrorStatus::UnknownResource)
    .flatten()?;

    let domain = socket.domain();
    let kind = socket.sock_type();

    match addr {
        Addr::Abstract(abs) if domain == SocketDomain::Unix => {
            sockets::bind_abstract_socket(abs.clone(), id)
        }
        // TODO: use the IpV4 field
        Addr::Inet { port, .. } if domain == SocketDomain::Net && kind == SocketKind::Datagram => {
            crate::net::udp::bind_socket(port, &socket).map_err(|()| ErrorStatus::AlreadyExists)?
        }
        _ => Err(ErrorStatus::TypeMismatch)?,
    }

    Ok(())
}
