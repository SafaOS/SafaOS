use crate::net::MacAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct EthernetType([u8; 2]);

impl EthernetType {
    pub const ARP: Self = Self([0x08, 0x06]);
    pub const IPV4: Self = Self([0x08, 0x00]);
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
}

#[derive(Debug, PartialEq, Eq)]
#[repr(C)]
pub struct EthernetFrame {
    pub header: EthernetHeader,
    pub payload: [u8],
}

impl EthernetFrame {
    // pub fn from_bytes(bytes: &[u8]) -> &Self {
    //     assert!(bytes.len() >= 14);
    //     let payload = &bytes[14..];
    //     unsafe { &*((bytes as *const [u8]).with_metadata_of(payload) as *const Self) }
    // }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        assert!(bytes.len() >= 14);
        let ptr = bytes as *mut [u8];
        let payload = &mut bytes[14..];

        unsafe { &mut *((ptr).with_metadata_of(payload) as *mut Self) }
    }
}
