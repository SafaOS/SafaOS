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
        ipv4::PageIPv4Packet,
        manager::{NetworkError, NetworkManager},
    },
    sockets::udp::UdpSocket,
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

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IcmpHeader {
    ty: ICMPType,
    code: u8,
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
            Some(unsafe { core::mem::transmute::<&IcmpPacket, &EchoIcmpPacket>(packet) })
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
        o => {
            debug!(
                NetworkManager,
                "Received ICMP Packet with unknown type: {o:?}"
            );
        }
    }
}
