use core::{
    fmt::{self, Debug, Display},
    net::Ipv4Addr,
};

use crate::net::MacAddress;

/// The type of the hardware layer the packet is destined for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ARPHtype([u8; 2]);

impl ARPHtype {
    pub const ETHERNET: Self = Self([0x00, 0x01]);
}

impl Display for ARPHtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}", u16::from_be_bytes(self.0))
    }
}

impl fmt::Debug for ARPHtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ARPHtype({:04x})", u16::from_be_bytes(self.0))
    }
}

/// The type of the protocol address that the ARP request uses.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ARPPtype([u8; 2]);

impl ARPPtype {
    pub const IP: Self = Self([0x08, 0x00]);
}

impl Display for ARPPtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}", u16::from_be_bytes(self.0))
    }
}

impl fmt::Debug for ARPPtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ARPPtype({:04x})", u16::from_be_bytes(self.0))
    }
}

/// The operation of the ARP packet.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ARPOp([u8; 2]);

impl ARPOp {
    pub const REQUEST: Self = Self([0x00, 0x01]);
}

impl Display for ARPOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}", u16::from_be_bytes(self.0))
    }
}

impl Debug for ARPOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ARPOp({:04x})", u16::from_be_bytes(self.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ARPHeader {
    pub htype: ARPHtype,
    pub ptype: ARPPtype,
    pub hlen: u8,
    pub plen: u8,
    pub op: ARPOp,
}

/// Representation of a hardware address that is a part of an ARP packet.
pub trait ARPHardwareAddr: Sized {
    const TYPE: ARPHtype;
    const SIZE: usize = size_of::<Self>();
    const ZERO: Self;
}
/// Representation of a protocol address that is a part of an ARP packet.
pub trait ARPProtocolAddr: Sized {
    const TYPE: ARPPtype;
    const SIZE: usize = size_of::<Self>();
}

impl ARPHardwareAddr for MacAddress {
    const TYPE: ARPHtype = ARPHtype::ETHERNET;
    const SIZE: usize = 6;
    const ZERO: Self = Self::ZERO;
}

impl ARPProtocolAddr for Ipv4Addr {
    const TYPE: ARPPtype = ARPPtype::IP;
    const SIZE: usize = 4;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ARP<H: ARPHardwareAddr, P: ARPProtocolAddr> {
    pub header: ARPHeader,
    pub src_haddr: H,
    pub src_paddr: P,
    pub dst_haddr: H,
    pub dst_paddr: P,
}

impl<H: ARPHardwareAddr, P: ARPProtocolAddr> ARP<H, P> {
    /// Creates a new ARP packet.
    pub const fn new(op: ARPOp, src_haddr: H, src_paddr: P, dst_haddr: H, dst_paddr: P) -> Self {
        Self {
            header: ARPHeader {
                htype: H::TYPE,
                ptype: P::TYPE,
                hlen: H::SIZE as u8,
                plen: P::SIZE as u8,
                op,
            },
            src_haddr,
            src_paddr,
            dst_haddr,
            dst_paddr,
        }
    }

    pub const fn new_request(src_haddr: H, src_paddr: P, dst_paddr: P) -> Self {
        Self::new(ARPOp::REQUEST, src_haddr, src_paddr, H::ZERO, dst_paddr)
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const Self as *const u8, size_of::<Self>()) }
    }
}
