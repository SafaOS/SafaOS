use core::fmt::{Debug, Display};

pub mod arp;
pub mod ethernet;
pub mod manager;
pub use manager::{add_interface, handle_packet};
pub mod interface;
pub mod ipv4;

/// Represents a MAC address.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MacAddress {
    bytes: [u8; 6],
}

impl MacAddress {
    pub const BROADCAST: Self = Self { bytes: [0xFF; 6] };
    pub const ZERO: Self = Self { bytes: [0; 6] };

    pub const fn new(bytes: [u8; 6]) -> Self {
        Self { bytes }
    }
}

impl Display for MacAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.bytes[0],
            self.bytes[1],
            self.bytes[2],
            self.bytes[3],
            self.bytes[4],
            self.bytes[5]
        )
    }
}

impl Debug for MacAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "MacAddress({self})")
    }
}
