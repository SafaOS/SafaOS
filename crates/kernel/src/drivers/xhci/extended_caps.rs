use bitfield_struct::bitfield;

use crate::drivers::pci::extended_caps::{ExtendedCaptability, GenericCaptability};

#[bitfield(u32)]
struct XHCIUSBSupportedCapD3 {
    #[bits(4)]
    slot_type: u8,
    #[bits(28)]
    __: (),
}

#[repr(C)]
pub struct XHCIUSBSupportedProtocolCap {
    // dword 0
    header: GenericCaptability,
    minor_revision_version: u8,
    major_revision_version: u8,
    // dword 1
    name: u32,
    // dword 2
    compatible_port_offset: u8,
    compatible_port_count: u8,
    protocol_defined: u8,
    protocol_speed_id_count: u8,
    dword3: XHCIUSBSupportedCapD3,
}

impl ExtendedCaptability for XHCIUSBSupportedProtocolCap {
    fn id() -> u8 {
        0x2
    }

    fn header(&self) -> &crate::drivers::pci::extended_caps::GenericCaptability {
        &self.header
    }
}

impl XHCIUSBSupportedProtocolCap {
    /// Returns a ZERO-baased port index representing the first port compatible with this captability
    pub const fn first_compatible_port(&self) -> u8 {
        self.compatible_port_offset - 1
    }

    /// Returns a ZERO-baased port index representing the last port compatible with this captability
    pub const fn last_compatible_port(&self) -> u8 {
        self.first_compatible_port() + self.compatible_port_count - 1
    }

    /// Returns the major revision version of this captability, eg. 3 for USB3
    pub const fn major_version(&self) -> u8 {
        self.major_revision_version
    }
}
