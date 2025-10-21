use std::{fs::File, io, path::Path};

use crate::dhcp::DHCPOffer;

#[derive(Debug)]
pub struct Nic(File);

impl Nic {
    pub fn open(path: &Path) -> io::Result<Self> {
        File::open(path).map(Nic)
    }

    fn cmd(&self, cmd: u16, value: impl Into<u64>) -> io::Result<()> {
        use std::os::safaos::io::IoUtils;
        self.0.send_command(cmd, value.into())
    }

    fn cmd_with_ref<T>(&self, cmd: u16, r: &T) -> io::Result<()> {
        self.cmd(cmd, r as *const T as usize as u64)
    }

    unsafe fn cmd_with_mut<T>(&self, cmd: u16, m: &mut T) -> io::Result<()> {
        self.cmd(cmd, m as *mut T as usize as u64)
    }

    /// Returns the NIC's mac address.
    pub fn mac(&self) -> [u8; 6] {
        const GET_NIC_MAC_ADDR: u16 = 0x1003;
        let mut mac = [0; 6];
        unsafe {
            self.cmd_with_mut(GET_NIC_MAC_ADDR, &mut mac)
                .expect("Getting a MAC address should never fail on a NIC")
        };
        mac
    }

    /// Configures the NIC to use the offer `offer`.
    pub fn configure_with_offer(&self, offer: &DHCPOffer) -> io::Result<()> {
        use std::net::Ipv4Addr;
        #[derive(Debug, Clone, Copy)]
        #[repr(C)]
        pub struct NicAddrInfoV4 {
            pub ipv4_address: Ipv4Addr,
            pub gateway_address: Ipv4Addr,
            pub subnet_mask: Ipv4Addr,
            __0: u32,
            __1: u64,
        }

        const CMD_SET_NIC_ADDR_INFO: u16 = 0x1002;
        let nic_info = NicAddrInfoV4 {
            ipv4_address: offer.our_addr,
            gateway_address: offer.router,
            subnet_mask: offer.subnet_mask,
            __0: 0,
            __1: 0,
        };

        self.cmd_with_ref(CMD_SET_NIC_ADDR_INFO, &nic_info)
    }
}
