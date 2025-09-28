use core::net::Ipv4Addr;

use macros::display_consts;

use crate::net::MacAddress;

/// The type of the hardware layer the packet is destined for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ARPHtype(u16);

#[display_consts]
impl ARPHtype {
    pub const ETHERNET: Self = Self(0x1);
}

/// The type of the protocol address that the ARP request uses.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ARPPtype(u16);

#[display_consts]
impl ARPPtype {
    pub const IP: Self = Self(0x0800);
}

/// The operation of the ARP packet.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ARPOp(u16);

#[display_consts]
impl ARPOp {
    pub const REQUEST: Self = Self(0x1);
    pub const REPLY: Self = Self(0x2);
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
