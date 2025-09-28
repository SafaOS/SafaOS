use crate::net::MacAddress;
use macros::display_consts;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct EthernetType(u16);

#[display_consts]
impl EthernetType {
    pub const ARP: Self = Self(0x0806);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct EthernetHeader {
    pub dest: MacAddress,
    pub src: MacAddress,
    pub ethertype: EthernetType,
}

impl EthernetHeader {
    pub const fn new(dest: MacAddress, src: MacAddress, ethertype: EthernetType) -> Self {
        Self {
            dest,
            src,
            ethertype,
        }
    }

    pub const fn into_bytes(self) -> [u8; size_of::<Self>()] {
        unsafe { core::mem::transmute(self) }
    }
}

#[derive(Debug, PartialEq, Eq)]
#[repr(C)]
pub struct EthernetFrame {
    pub header: EthernetHeader,
    pub payload: [u8],
}

impl EthernetFrame {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        assert!(bytes.len() >= 14);
        let payload = &bytes[14..];
        unsafe { &*((bytes as *const [u8]).with_metadata_of(payload) as *const Self) }
    }

    pub const fn size(&self) -> usize {
        size_of_val(self)
    }

    pub const fn payload_len(&self) -> usize {
        self.payload.len()
    }
}
