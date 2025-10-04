use alloc::sync::{Arc, Weak};
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;

use crate::{
    sockets::{Socket, SocketError},
    utils::locks::RwLock,
    warn,
};

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

    // TODO: verify packets...
    pub fn length(&self) -> u16 {
        u16::from_be_bytes(self.length)
    }

    pub fn as_bytes(&self) -> &[u8] {
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
            Ok(unsafe { &*(bytes as *const [u8] as *const Self) })
        }
    }

    pub const fn header(&self) -> &UDPHeader {
        unsafe { &*(self.0.as_ptr() as *const UDPHeader) }
    }

    pub fn payload(&self) -> &[u8] {
        &self.0[size_of::<UDPHeader>()..]
    }
}

static UDP_PORTS: RwLock<HashMap<u16, Weak<Socket>, FxBuildHasher>> =
    RwLock::new(HashMap::with_hasher(FxBuildHasher));

pub fn handle_udp_packet(bytes: &[u8]) {
    let Ok(packet) = UDPPacket::try_from_bytes(bytes) else {
        warn!("Invalid UDP Packet ignoring...");
        return;
    };

    let header = packet.header();
    let dst_port = header.dst_port();

    let ports = UDP_PORTS.read();
    if let Some(socket) = ports.get(&dst_port) {
        let upgraded = socket.upgrade();
        drop(ports);

        if let Some(socket) = upgraded {
            match socket.write_socket(packet.payload()) {
                Err(SocketError::WouldBlockFull) => {} // vola packet lost!
                Err(e) => {
                    crate::error!("Failed to write to socket: {e:?}, at udp port {dst_port}");
                }
                Ok(am) => {
                    if am != bytes.len() {
                        warn!(
                            "FIXME: wrote only {am} bytes of {} in UDP socket",
                            bytes.len()
                        )
                    }
                }
            }
        } else {
            UDP_PORTS.write().remove(&dst_port);
        }
    }
}

pub fn remove_socket(port: u16) -> bool {
    let mut ports = UDP_PORTS.write();
    ports.remove(&port).is_some()
}

/// Attempts to bind a socket to a UDP port, returning an error if the port is already in use.
pub fn bind_socket(port: u16, socket: &Arc<Socket>) -> Result<(), ()> {
    let mut ports = UDP_PORTS.write();
    ports
        .try_insert(port, Arc::downgrade(socket))
        .map_err(|_| ())?;
    socket.set_udp_port(port);
    Ok(())
}
