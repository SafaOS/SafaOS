use core::net::Ipv4Addr;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct NicAddrInfoV4 {
    pub ipv4_address: Ipv4Addr,
    pub gateway_address: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    __0: u32,
    __1: u64,
}

impl NicAddrInfoV4 {
    pub const fn new(
        ipv4_address: Ipv4Addr,
        gateway_address: Ipv4Addr,
        subnet_mask: Ipv4Addr,
    ) -> Self {
        Self {
            ipv4_address,
            gateway_address,
            subnet_mask,
            __0: 0,
            __1: 0,
        }
    }

    /// Returns the default uninitialized value.
    pub const fn default() -> Self {
        Self {
            ipv4_address: Ipv4Addr::UNSPECIFIED,
            gateway_address: Ipv4Addr::UNSPECIFIED,
            subnet_mask: Ipv4Addr::UNSPECIFIED,
            __0: 0,
            __1: 0,
        }
    }
}

impl Default for NicAddrInfoV4 {
    fn default() -> Self {
        Self::default()
    }
}
