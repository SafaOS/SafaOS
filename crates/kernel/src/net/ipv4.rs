use core::{
    fmt::{self, Debug},
    net::Ipv4Addr,
    ops::{Deref, DerefMut},
};

use bitfield_struct::bitfield;
use macros::display_consts;

use crate::{
    debug,
    net::{
        interface::NetworkInterface,
        udp::{UDPHeader, UDPPacket},
    },
    utils::alloc::PageVec,
    warn,
};

/// Version and header length.
#[bitfield(u8)]
pub struct VersionIHL {
    /// Header length, the header by default is 20 bytes but it can extend up to 60 bytes with options.
    #[bits(4)]
    ihl: u8,
    /// Version of the IP protocol, 4 for IPv4.
    #[bits(4)]
    version: u8,
}

#[bitfield(u8)]
pub struct DSCPECN {
    /// This field allows end-to-end notification of network congestion without dropping packets.
    /// ECN is an optional feature available when both endpoints support it and effective when also supported by the underlying network.
    #[bits(2)]
    ecn: u8,
    /// Originally defined as the type of service (ToS), this field specifies differentiated services (DiffServ).
    ///
    /// Real-time data streaming makes use of the DSCP field.
    ///
    /// An example is Voice over IP (VoIP), which is used for interactive voice services.
    #[bits(6)]
    dscp: u8,
}

#[bitfield(u16)]
pub struct FragmentFlags {
    #[bits(13)]
    fragment_offset: u16,
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
    pub const fn new(payload_len: u16, dst_addr: Ipv4Addr, protocol: IPv4Protocol) -> Self {
        let total_len = payload_len + size_of::<Self>() as u16;
        let this = Self {
            version_ihl: VersionIHL::new()
                .with_version(4)
                .with_ihl((size_of::<Self>() / 4) as u8),
            dscp_ecn: DSCPECN::new(),
            total_length: total_len.to_be_bytes(),
            identification: [0; 2],
            fragment_flags: FragmentFlags::new(),
            time_to_live: 64,
            protocol,
            header_checksum: [0; 2],
            src_addr: Ipv4Addr::UNSPECIFIED,
            dst_addr,
        };

        this
    }

    pub const fn calculate_checksum(&self) -> u16 {
        let mut sum = 0u32;
        let bytes = self.as_bytes();

        let (as_u16, _) = bytes.as_chunks::<2>();

        let mut i = 0;
        while i < as_u16.len() {
            let chunk = as_u16[i];

            sum += u16::from_be_bytes(chunk) as u32;
            i += 1;
        }

        while sum >> 16 != 0 {
            sum = (sum >> 16) + (sum & u16::MAX as u32);
        }

        !(sum as u16)
    }

    /// Calculates the checksum and puts it into the header.
    fn put_checksum(&mut self) {
        self.header_checksum = self.calculate_checksum().to_be_bytes();
    }

    pub const fn total_length(&self) -> usize {
        u16::from_be_bytes(self.total_length) as usize
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { core::mem::transmute::<&Self, &[u8; size_of::<Self>()]>(self) }
    }
}

const _: () = assert!(size_of::<IPv4Header>() == 20);
const _: () = assert!(align_of::<IPv4Header>() == 1);

#[repr(C)]
pub struct IPv4Packet([u8]);

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

    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.0[size_of::<IPv4Header>()..]
    }

    pub fn header(&self) -> &IPv4Header {
        unsafe { &*(self.0.as_ptr() as *const IPv4Header) }
    }

    pub fn header_mut(&mut self) -> &mut IPv4Header {
        unsafe { &mut *(self.0.as_mut_ptr() as *mut IPv4Header) }
    }

    /// Calculates the checksum and puts it into the IPv4 header and the underlying protocol's header.
    pub fn put_checksum(&mut self) {
        let header = self.header_mut();
        header.put_checksum();
        match header.protocol {
            IPv4Protocol::UDP => {
                let src_addr = header.src_addr;
                let dst_addr = header.dst_addr;

                let udp_packet = UDPPacket::try_from_bytes_mut(self.payload_mut())
                    .expect("Invalid UDP packet: too small");
                udp_packet.put_checksum(src_addr, dst_addr);
            }
            _ => {}
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
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

/// Owned IPv4 packet, the allocation size is in pages
#[repr(C)]
pub struct PageIPv4Packet(PageVec<u8>);
impl PageIPv4Packet {
    pub fn new(header: IPv4Header) -> Self {
        let mut packet = PageVec::with_capacity(size_of::<IPv4Header>());
        packet.extend_from_slice(header.as_bytes());
        Self(packet)
    }

    pub fn push(&mut self, data: &[u8]) {
        self.0.reserve(data.len());
        self.0.extend_from_slice(data);
    }

    /// Constructs a new Owned IPv4 packet with the given UDP header and payload.
    pub fn new_udp(payload: &[u8], src_port: u16, dst_port: u16, dst_addr: Ipv4Addr) -> Self {
        let udp_header = UDPHeader::new(src_port, dst_port, payload.len() as u16);
        let udp_len = udp_header.length();

        let ipv4_header = IPv4Header::new(udp_len, dst_addr, IPv4Protocol::UDP);
        let mut this = Self::new(ipv4_header);
        this.push(udp_header.as_bytes());
        this.push(payload);
        this
    }
}

impl Deref for PageIPv4Packet {
    type Target = IPv4Packet;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.0.as_slice() as *const [u8] as *const IPv4Packet) }
    }
}

impl DerefMut for PageIPv4Packet {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.0.as_mut_slice() as *mut [u8] as *mut IPv4Packet) }
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
                warn!(
                    IPv4Manager,
                    "Invalid IPv4 packet: invalid length, ignoring..."
                );
                return;
            }
            Ok(packet) if packet.header().calculate_checksum() != 0 => {
                warn!(
                    IPv4Manager,
                    "Invalid IPv4 packet: invalid checksum, ignoring..."
                );
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
