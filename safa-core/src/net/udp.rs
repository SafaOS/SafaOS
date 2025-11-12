use core::{net::Ipv4Addr, num::NonZero};

use alloc::sync::Arc;
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;

use crate::{
    net::ipv4::IPv4Protocol,
    sockets::{SocketError, udp::UdpSocket},
    utils::locks::RwLock,
    warn,
};

#[repr(C, packed)]
struct PseudoHeader {
    src_addr: Ipv4Addr,
    dst_addr: Ipv4Addr,
    zeros: u8,
    protocol: IPv4Protocol,
    udp_length: [u8; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct UDPHeader {
    src_port: [u8; 2],
    dst_port: [u8; 2],
    length: [u8; 2],
    checksum: [u8; 2],
}

impl UDPHeader {
    pub fn new(src_port: u16, dst_port: u16, payload_length: u16) -> Self {
        let length = payload_length + size_of::<UDPHeader>() as u16;
        Self {
            src_port: src_port.to_be_bytes(),
            dst_port: dst_port.to_be_bytes(),
            length: length.to_be_bytes(),
            checksum: [0; 2],
        }
    }

    pub fn dst_port(&self) -> u16 {
        u16::from_be_bytes(self.dst_port)
    }

    pub fn src_port(&self) -> u16 {
        u16::from_be_bytes(self.src_port)
    }

    // TODO: verify packets...
    pub fn length(&self) -> u16 {
        u16::from_be_bytes(self.length)
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { &*(self as *const Self as *const [u8; size_of::<Self>()]) }
    }
}

#[derive(PartialEq, Eq, Hash)]
#[repr(transparent)]
/// Represents a UDP packet.
pub struct UDPPacket([u8]);
impl UDPPacket {
    pub const fn try_from_bytes(bytes: &[u8]) -> Result<&Self, ()> {
        if bytes.len() < size_of::<UDPHeader>() {
            Err(())
        } else {
            Ok(unsafe { Self::from_bytes_unchecked(bytes) })
        }
    }

    pub const fn try_from_bytes_mut(bytes: &mut [u8]) -> Result<&mut Self, ()> {
        if bytes.len() < size_of::<UDPHeader>() {
            Err(())
        } else {
            Ok(unsafe { Self::from_bytes_unchecked_mut(bytes) })
        }
    }

    pub const unsafe fn from_bytes_unchecked(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub const unsafe fn from_bytes_unchecked_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut Self) }
    }

    pub fn calculate_checksum(&self, src_addr: Ipv4Addr, dst_addr: Ipv4Addr) -> u16 {
        let as_bytes = self.as_bytes();
        let (as_chunks, remaining) = as_bytes.as_chunks::<2>();

        let pseudo_header = PseudoHeader {
            src_addr,
            dst_addr,
            udp_length: (as_bytes.len() as u16).to_be_bytes(),
            protocol: IPv4Protocol::UDP,
            zeros: 0,
        };

        let pseudo_words: [u16; size_of::<PseudoHeader>() / 2] =
            unsafe { core::mem::transmute(pseudo_header) };

        let mut sum = 0u32;
        for word in pseudo_words {
            sum += u16::from_be(word) as u32;
        }

        for chunk in as_chunks {
            let word = u16::from_be_bytes(*chunk);
            sum += word as u32;
        }

        for extra in remaining {
            let word = u16::from_be_bytes([*extra, 0]);
            sum += word as u32;
        }

        while sum >> 16 != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !(sum as u16)
    }

    pub fn put_checksum(&mut self, src_addr: Ipv4Addr, dst_addr: Ipv4Addr) {
        let checksum = self.calculate_checksum(src_addr, dst_addr);
        self.header_mut().checksum = checksum.to_be_bytes();
    }

    pub const fn header(&self) -> &UDPHeader {
        unsafe { &*(self.0.as_ptr() as *const UDPHeader) }
    }

    pub const fn header_mut(&mut self) -> &mut UDPHeader {
        unsafe { &mut *(self.0.as_mut_ptr() as *mut UDPHeader) }
    }

    pub fn payload(&self) -> &[u8] {
        &self.0[size_of::<UDPHeader>()..]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

static UDP_PORTS: RwLock<HashMap<u16, Arc<UdpSocket>, FxBuildHasher>> =
    RwLock::new(HashMap::with_hasher(FxBuildHasher));

pub fn handle_udp_packet(src_ip: Ipv4Addr, bytes: &[u8]) {
    let Ok(packet) = UDPPacket::try_from_bytes(bytes) else {
        warn!("Invalid UDP Packet ignoring...");
        return;
    };

    let header = packet.header();
    let dst_port = header.dst_port();
    let src_port = header.src_port();

    let ports = UDP_PORTS.read();
    if let Some(socket) = ports.get(&dst_port) {
        let payload = packet.payload();
        let am = socket.write(src_ip, src_port, payload);
        if am != payload.len() {
            warn!(
                "FIXME: wrote only {am} bytes of {} in UDP socket",
                payload.len()
            )
        }
    }
}

pub fn remove_socket(port: NonZero<u16>) -> bool {
    let mut ports = UDP_PORTS.write();
    ports.remove(&port.get()).is_some()
}

/// Attempts to bind a socket to a UDP port, returning an error if the port is already in use or if no port is available.
pub fn bind_socket(port: Option<NonZero<u16>>, socket: Arc<UdpSocket>) -> Result<NonZero<u16>, ()> {
    let mut ports = UDP_PORTS.write();
    let port = port.map(|p| Ok(p)).unwrap_or_else(|| {
        let mut curr_port = 1024;
        while curr_port <= u16::MAX {
            // TODO: slowwy >_<
            if !ports.contains_key(&curr_port) {
                return Ok(NonZero::new(curr_port).unwrap());
            }
            curr_port += 1;
        }
        Err(())
    })?;
    ports.try_insert(port.get(), socket).map_err(|_| ())?;
    Ok(port)
}

/// UDP's handler for a destination unreachable ICMP packet.
///
/// returns an Error if the packet is too small to be a UDP Packet.
pub fn handle_dest_unreachable_udp(
    error: SocketError,
    target_ip: Ipv4Addr,
    packet_payload: &[u8],
) -> Result<(), ()> {
    _ = target_ip;
    let udp_packet = UDPPacket::try_from_bytes(packet_payload)?;
    let header = udp_packet.header();

    let src_port = header.src_port();

    let ports = UDP_PORTS.read();
    let Some(socket) = ports.get(&src_port) else {
        return Ok(());
    };

    socket.set_error(error);
    Ok(())
}
