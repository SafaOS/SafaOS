use core::{
    fmt::{self, Debug},
    net::Ipv4Addr,
};

use bitfield_struct::bitfield;
use macros::display_consts;

use crate::{debug, net::interface::NetworkInterface, warn};

/// Version and header length.
#[bitfield(u8)]
pub struct VersionIHL {
    /// Version of the IP protocol, 4 for IPv4.
    #[bits(4)]
    version: u8,
    /// Header length, the header by default is 20 bytes but it can extend up to 60 bytes with options.
    #[bits(4)]
    ihl: u8,
}

#[bitfield(u8)]
pub struct DSCPECN {
    #[bits(6)]
    dscp: u8,
    #[bits(2)]
    ecn: u8,
}

#[bitfield(u16)]
pub struct FragmentFlags {
    #[bits(1)]
    __rsz0: (),
    /// This field specifies whether the datagram can be fragmented or not.
    /// This can be used when sending packets to a host that does not have resources to perform reassembly of fragments.
    /// It can also be used for path MTU discovery, either automatically by the host IP software,
    /// or manually using diagnostic tools such as ping or traceroute.
    /// If the DF flag is set, and fragmentation is required to route the packet, then the packet is dropped.
    df: bool,
    /// For unfragmented packets, the MF flag is cleared.
    ///
    /// For fragmented packets, all fragments except the last have the MF flag set.
    ///
    /// The last fragment has a non-zero Fragment Offset field, so it can still be differentiated from an unfragmented packet.
    mf: bool,
    #[bits(13)]
    fragment_offset: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct IPv4Protocol(u8);

#[display_consts]
impl IPv4Protocol {
    pub const ICMP: Self = Self(1);
    pub const IGMP: Self = Self(2);
    pub const TCP: Self = Self(6);
    pub const UDP: Self = Self(17);
    pub const ENCAP: Self = Self(41);
    pub const OSPF: Self = Self(89);
    pub const SCTP: Self = Self(132);
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IPv4Header {
    version_ihl: VersionIHL,
    dscp_ecn: DSCPECN,
    total_length: [u8; 2],
    /// This field is an identification field and is primarily used for uniquely identifying the group of fragments of a single IP datagram.
    identification: [u8; 2],
    /// See [`FragmentFlags`].
    fragment_flags: FragmentFlags,
    /// The time to live field limits a datagram's lifetime to prevent network failure in the event of a routing loop.
    /// It is specified in seconds, but time intervals less than 1 second are rounded up to 1.
    time_to_live: u8,
    // This field defines the transport layer protocol used in the data portion of the IP datagram.
    protocol: IPv4Protocol,
    header_checksum: [u8; 2],

    pub src_addr: Ipv4Addr,
    pub dst_addr: Ipv4Addr,
}

impl IPv4Header {
    pub fn total_length(&self) -> usize {
        u16::from_be_bytes(self.total_length) as usize
    }
}

const _: () = assert!(size_of::<IPv4Header>() == 20);
const _: () = assert!(align_of::<IPv4Header>() == 1);

#[repr(C)]
struct IPv4Packet([u8]);

impl IPv4Packet {
    /// Creates a new pending IPv4 packet from a byte slice, returns an error if the slice is too small to hold a [`IPv4Header`].
    pub fn try_from_bytes(bytes: &[u8]) -> Result<&Self, ()> {
        if bytes.len() < size_of::<IPv4Header>() {
            return Err(());
        }

        unsafe { Ok(Self::from_bytes_unchecked(bytes)) }
    }

    unsafe fn from_bytes_unchecked(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub const fn total_size(&self) -> usize {
        self.0.len()
    }

    pub fn payload(&self) -> &[u8] {
        &self.0[size_of::<IPv4Header>()..]
    }

    pub fn header(&self) -> &IPv4Header {
        unsafe { &*(self.0.as_ptr() as *const IPv4Header) }
    }
}

impl Debug for IPv4Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(stringify!(IPv4Packet))
            .field("header", &self.header())
            .field("data", &self.payload())
            .finish()
    }
}

pub struct IPv4Manager;

impl IPv4Manager {
    pub const fn new() -> Self {
        Self
    }

    fn handle_packet(&self, int: &'static dyn NetworkInterface, raw_packet: &[u8]) {
        let packet = match IPv4Packet::try_from_bytes(raw_packet) {
            Ok(packet) if packet.header().total_length() > packet.total_size() => {
                warn!(IPv4Manager, "Invalid IPv4 packet");
                return;
            }
            Ok(packet) => packet,
            Err(_) => {
                warn!(IPv4Manager, "Invalid IPv4 packet");
                return;
            }
        };

        match packet.header().protocol {
            IPv4Protocol::UDP => super::udp::handle_udp_packet(packet.payload()),
            _ => {
                debug!(
                    IPv4Manager,
                    "Got unknown packet: {packet:#?}, interface: {}",
                    int.name()
                );
            }
        }
    }
}

static IPV4_MANAGER: IPv4Manager = IPv4Manager::new();

/// Handles an incoming IPv4 packet.
/// TODO: Its probably smarter to handle packets in a separate thread.
pub fn handle_ipv4_packet(int: &'static dyn NetworkInterface, packet: &[u8]) {
    IPV4_MANAGER.handle_packet(int, packet);
}
