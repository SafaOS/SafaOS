use core::{
    net::Ipv4Addr,
    ops::{Deref, DerefMut},
};

use alloc::sync::Arc;
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;

use crate::{
    debug,
    net::{
        self, calculate_checksum_of,
        ipv4::{IPv4Header, IPv4Protocol, PageIPv4Packet},
        manager::{NetworkError, NetworkManager},
    },
    sockets::{SocketError, udp::UdpSocket},
    utils::locks::Mutex,
    warn,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ICMPType(pub u8);

impl ICMPType {
    pub const ECHO_REQUEST: ICMPType = ICMPType(8);
    pub const ECHO_REPLY: ICMPType = ICMPType(0);
    pub const DESTINATION_UNREACHABLE: ICMPType = ICMPType(3);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ICMPCode(u8);
impl ICMPCode {
    pub const NETWORK_UNREACHABLE: ICMPCode = ICMPCode(0);
    pub const HOST_UNREACHABLE: ICMPCode = ICMPCode(1);
    pub const PROTOCOL_UNREACHABLE: ICMPCode = ICMPCode(2);
    pub const PORT_UNREACHABLE: ICMPCode = ICMPCode(3);
    pub const FRAGMENTATION_NEEDED: ICMPCode = ICMPCode(4);
    pub const SOURCE_ROUTE_FAILED: ICMPCode = ICMPCode(5);
    pub const DESTINATION_NETWORK_UNKNOWN: ICMPCode = ICMPCode(6);
    pub const DESTINATION_HOST_UNKNOWN: ICMPCode = ICMPCode(7);
    pub const SOURCE_HOST_ISOLATED: ICMPCode = ICMPCode(8);
    pub const NETWORK_ADMINISTRATIVELY_PROHIBITED: ICMPCode = ICMPCode(9);
    pub const HOST_ADMINISTRATIVELY_PROHIBITED: ICMPCode = ICMPCode(10);
    pub const NETWORK_UNREACHABLE_FOR_TOS: ICMPCode = ICMPCode(11);
    pub const HOST_UNREACHABLE_FOR_TOS: ICMPCode = ICMPCode(12);
    pub const COMMUNICATION_ADMINISTRATIVELY_PROHIBITED: ICMPCode = ICMPCode(13);
    pub const HOST_PRECEDENCE_VIOLATION: ICMPCode = ICMPCode(14);
    pub const PRECEDENCE_CUTOFF_IN_EFFECT: ICMPCode = ICMPCode(15);
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IcmpHeader {
    ty: ICMPType,
    code: ICMPCode,
    checksum: u16,
}

impl IcmpHeader {
    pub const fn ty(&self) -> ICMPType {
        self.ty
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct IcmpPacket([u8]);

impl IcmpPacket {
    pub const fn new<'a>(data: &'a [u8]) -> Option<&'a Self> {
        if data.len() < size_of::<IcmpHeader>() {
            None
        } else {
            Some(unsafe { core::mem::transmute::<&[u8], &Self>(data) })
        }
    }

    pub const fn new_mut<'a>(data: &'a mut [u8]) -> Option<&'a mut Self> {
        if data.len() < size_of::<IcmpHeader>() {
            None
        } else {
            Some(unsafe { core::mem::transmute::<&mut [u8], &mut Self>(data) })
        }
    }

    const fn raw_header(&self) -> &[u8; size_of::<IcmpHeader>()] {
        unsafe { self.0.first_chunk().unwrap_unchecked() }
    }

    pub const fn header(&self) -> IcmpHeader {
        unsafe { core::mem::transmute(*self.raw_header()) }
    }

    /// Returns the data portion of the ICMP packet.
    pub const fn data<'b>(&'b self) -> &'b [u8] {
        let (_, data) = self.0.split_at(size_of::<IcmpHeader>());
        data
    }

    /// Calculates the checksum of the ICMP packet.
    pub const fn calculate_checksum(&self) -> u16 {
        calculate_checksum_of(self.as_bytes())
    }

    const fn raw_header_mut(&mut self) -> &mut [u8; size_of::<IcmpHeader>()] {
        unsafe { self.0.first_chunk_mut().unwrap_unchecked() }
    }

    pub const fn header_mut(&mut self) -> &mut IcmpHeader {
        unsafe { core::mem::transmute(self.raw_header_mut()) }
    }

    pub fn put_checksum(&mut self) {
        let checksum = self.calculate_checksum();
        self.header_mut().checksum = checksum.to_be();
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        let (_, data) = self.0.split_at_mut(size_of::<IcmpHeader>());
        data
    }

    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct EchoHeader {
    id: u16,
    seq: u16,
}

impl EchoHeader {
    pub const fn id(&self) -> u16 {
        u16::from_be(self.id)
    }

    pub const fn set_id(&mut self, id: u16) {
        self.id = id.to_be();
    }

    pub const fn from_array(bytes: &[u8; size_of::<EchoHeader>()]) -> &Self {
        unsafe { core::mem::transmute::<&[u8; size_of::<EchoHeader>()], &EchoHeader>(bytes) }
    }

    pub const fn from_array_mut(bytes: &mut [u8; size_of::<EchoHeader>()]) -> &mut Self {
        unsafe {
            core::mem::transmute::<&mut [u8; size_of::<EchoHeader>()], &mut EchoHeader>(bytes)
        }
    }

    pub const fn from_bytes(bytes: &[u8]) -> Option<&Self> {
        match bytes.first_chunk::<{ size_of::<EchoHeader>() }>() {
            Some(bytes) => Some(Self::from_array(bytes)),
            None => None,
        }
    }

    pub const fn from_bytes_mut(bytes: &mut [u8]) -> Option<&mut Self> {
        match bytes.first_chunk_mut::<{ size_of::<EchoHeader>() }>() {
            Some(bytes) => Some(Self::from_array_mut(bytes)),
            None => None,
        }
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct EchoIcmpPacket(IcmpPacket);

impl EchoIcmpPacket {
    /// Create a new Echo ICMP packet from a an IcmpPacket, return None if the data is too small.
    pub const fn new<'a>(packet: &'a IcmpPacket) -> Option<&'a Self> {
        if packet.data().len() < size_of::<EchoHeader>() {
            None
        } else {
            Some(unsafe { core::mem::transmute::<&IcmpPacket, &Self>(packet) })
        }
    }

    const fn new_mut<'a>(data: &'a mut [u8]) -> Option<&'a mut Self> {
        match IcmpPacket::new_mut(data) {
            Some(packet) => {
                if packet.data().len() < size_of::<EchoHeader>() {
                    None
                } else {
                    Some(unsafe {
                        core::mem::transmute::<&mut IcmpPacket, &mut EchoIcmpPacket>(packet)
                    })
                }
            }
            None => None,
        }
    }

    pub fn header(&self) -> &EchoHeader {
        unsafe { EchoHeader::from_bytes(self.data()).unwrap_unchecked() }
    }

    pub fn header_mut(&mut self) -> &mut EchoHeader {
        unsafe { EchoHeader::from_bytes_mut(self.data_mut()).unwrap_unchecked() }
    }
}

impl Deref for EchoIcmpPacket {
    type Target = IcmpPacket;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for EchoIcmpPacket {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct DestUnreachableICMPHeader {
    empty: u16,
    next_mtu: u16,
    ip_header: IPv4Header,
}

#[derive(Debug)]
#[repr(transparent)]
pub struct DestUnreachableICMPPacket(IcmpPacket);

impl DestUnreachableICMPPacket {
    /// Create a new Destination Unreachable ICMP packet from a an IcmpPacket, return None if the data is too small.
    pub const fn new<'a>(packet: &'a IcmpPacket) -> Option<&'a Self> {
        if packet.data().len() < size_of::<DestUnreachableICMPHeader>() {
            None
        } else {
            Some(unsafe { core::mem::transmute::<&IcmpPacket, &Self>(packet) })
        }
    }

    /// Returns a reference to the Destination Unreachable ICMP header.
    pub const fn header(&self) -> &DestUnreachableICMPHeader {
        unsafe { &*self.0.data().as_ptr().cast() }
    }

    pub fn data(&self) -> &[u8] {
        &self.0.data()[size_of::<DestUnreachableICMPHeader>()..]
    }
}

static PENDING_ICMP_REPLY: Mutex<HashMap<(Ipv4Addr, u16), Arc<UdpSocket>, FxBuildHasher>> =
    Mutex::new(HashMap::with_hasher(FxBuildHasher));

/// Cleans up any reference to the given ICMP Socket
pub fn cleanup_icmp_socket(s: &Arc<UdpSocket>) {
    let mut pending_replies = PENDING_ICMP_REPLY.lock();
    pending_replies.retain(|_, socket| !Arc::ptr_eq(socket, s));
}

/// Sends an ICMP echo request to the specified IP address.
pub fn send_echo_request(
    to: Ipv4Addr,
    socket: Arc<UdpSocket>,
    packet: &EchoIcmpPacket,
    replace_with_id: Option<u16>,
    ttl: u8,
) -> Result<(), NetworkError> {
    // Copying this packet
    let mut ipv4_packet = PageIPv4Packet::new_icmp(packet, to, ttl);

    // replacing the ID and checksum with our own
    if let Some(id) = replace_with_id {
        let new_packet_payload = ipv4_packet.payload_mut();
        let new_echo_packet = EchoIcmpPacket::new_mut(new_packet_payload).unwrap();
        let new_header = new_echo_packet.header_mut();

        new_header.set_id(id);
        new_echo_packet.0.header_mut().checksum = 0;
        new_echo_packet.put_checksum();

        PENDING_ICMP_REPLY.lock().insert((to, id), socket);
    }

    net::manager::send_ipv4_packet(Ipv4Addr::UNSPECIFIED, &mut ipv4_packet)
}

pub fn handle_icmp_packet(from: Ipv4Addr, our_ip: Ipv4Addr, packet_bytes: &mut [u8]) {
    let Some(packet) = IcmpPacket::new_mut(packet_bytes) else {
        debug!(NetworkManager, "Received ICMP Packet too small");
        return;
    };

    if packet.calculate_checksum() != 0 {
        debug!(NetworkManager, "Received ICMP Packet with invalid checksum");
        return;
    }

    let packet_size = packet.data().len();
    let header_mut = packet.header_mut();
    match header_mut.ty() {
        ICMPType::ECHO_REQUEST => {
            if packet_size < size_of::<EchoHeader>() {
                debug!(
                    NetworkManager,
                    "Received ICMP Echo Request payload invalid size: {packet_size}"
                );
                return;
            };

            // Reply and echo have the same header
            header_mut.ty = ICMPType::ECHO_REPLY;
            header_mut.checksum = 0;

            packet.put_checksum();

            let mut packet = PageIPv4Packet::new_icmp(&packet, from, 115);
            if let Err(err) = net::manager::send_ipv4_packet(our_ip, &mut packet) {
                warn!(
                    NetworkManager,
                    "Failed to send ICMP Echo Reply to {from}: {err:?}"
                );
            }
        }
        ICMPType::ECHO_REPLY => {
            let Some(echo_packet) = EchoIcmpPacket::new(packet) else {
                debug!(
                    NetworkManager,
                    "Received ICMP Echo Reply payload invalid size: {packet_size}"
                );
                return;
            };

            let header = echo_packet.header();
            let id = header.id();

            let mut waiting_for_replies = PENDING_ICMP_REPLY.lock();
            if let Some(socket) = waiting_for_replies.remove(&(from, id)) {
                socket.write(from, 0, echo_packet.as_bytes());
            }
        }
        ICMPType::DESTINATION_UNREACHABLE => {
            let Some(dest_unreachable_packet) = DestUnreachableICMPPacket::new(packet) else {
                debug!(
                    NetworkManager,
                    "Received ICMP Destination Unreachable payload invalid size: {packet_size}"
                );
                return;
            };

            handle_dest_unreachable(dest_unreachable_packet)
        }
        o => {
            debug!(
                NetworkManager,
                "Received ICMP Packet with unknown type: {o:?}"
            );
        }
    }
}

fn handle_dest_unreachable(packet: &DestUnreachableICMPPacket) {
    let code = packet.0.header().code;

    let error = match code {
        ICMPCode::NETWORK_UNREACHABLE
        | ICMPCode::DESTINATION_NETWORK_UNKNOWN
        | ICMPCode::SOURCE_HOST_ISOLATED
        | ICMPCode::NETWORK_UNREACHABLE_FOR_TOS => SocketError::NetworkUnreachable,

        ICMPCode::HOST_UNREACHABLE
        | ICMPCode::SOURCE_ROUTE_FAILED
        | ICMPCode::DESTINATION_HOST_UNKNOWN
        | ICMPCode::HOST_UNREACHABLE_FOR_TOS
        | ICMPCode::HOST_PRECEDENCE_VIOLATION
        | ICMPCode::PRECEDENCE_CUTOFF_IN_EFFECT => SocketError::HostUnreachable,

        ICMPCode::FRAGMENTATION_NEEDED => SocketError::InvalidSize,
        ICMPCode::PORT_UNREACHABLE => SocketError::ConnectionRefused,
        ICMPCode::PROTOCOL_UNREACHABLE => SocketError::ProtocolUnreachable,

        ICMPCode::HOST_ADMINISTRATIVELY_PROHIBITED
        | ICMPCode::NETWORK_ADMINISTRATIVELY_PROHIBITED
        | ICMPCode::COMMUNICATION_ADMINISTRATIVELY_PROHIBITED => SocketError::MissingPermissions,

        other => {
            warn!("Received an unknown ICMP Destination unreachable code: {other:?}");
            return;
        }
    };

    let dest_header = packet.header();
    let data = packet.data();

    let ip_header = dest_header.ip_header;
    let dst_addr = ip_header.dst_addr;

    let handler = match ip_header.protocol() {
        IPv4Protocol::ICMP => handle_dest_unreachable_for_icmp,
        IPv4Protocol::UDP => net::udp::handle_dest_unreachable_udp,
        _ => return, /* consider it done */
    };

    if let Err(()) = handler(error, dst_addr, data) {
        debug!(
            NetworkManager,
            "Couldn't handle Destination unreachable, because the payload is too small"
        );
    }
}
/// ICMP's handler for a destination unreachable ICMP packet.
///
/// returns an Error if the packet is too small to be an ICMP Packet.
fn handle_dest_unreachable_for_icmp(
    error: SocketError,
    target_ip: Ipv4Addr,
    icmp_packet_data: &[u8],
) -> Result<(), ()> {
    let packet = IcmpPacket::new(icmp_packet_data).ok_or(())?;

    let header = packet.header();
    match header.ty {
        ICMPType::ECHO_REQUEST => {
            let echo_packet = EchoIcmpPacket::new(packet).ok_or(())?;
            let echo_header = echo_packet.header();
            let id = echo_header.id;

            let mut waiting_for_replies = PENDING_ICMP_REPLY.lock();
            if let Some(socket) = waiting_for_replies.remove(&(target_ip, id)) {
                socket.set_error(error);
            }
        }
        _ => {} /* consider it done */
    }
    Ok(())
}
