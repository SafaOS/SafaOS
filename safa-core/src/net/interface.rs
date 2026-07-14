use core::net::Ipv4Addr;

use safa_abi::net::NicAddrInfoV4;

use crate::{
    devices::CharDevice,
    drivers::vfs::FSError,
    net::{MacAddress, ethernet::EthernetType},
    syscalls::ffi::SyscallFFI,
};

/// A network interface error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetIntError {
    PacketTooLarge,
}

/// A network interface.
pub trait NetworkInterface: Send + Sync {
    /// Send an Ethernet frame through the network interface.
    fn send_ethernet(
        &self,
        dst_mac: MacAddress,
        ethertype: EthernetType,
        payload: &[u8],
    ) -> Result<(), NetIntError>;
    /// Retrieves the MAC address of the network interface.
    fn mac_address(&self) -> MacAddress;
    fn name(&self) -> &'static str;

    fn nic_info(&self) -> NicAddrInfoV4;
    fn ipv4_address(&self) -> Ipv4Addr;
    fn set_nic_info(&self, info: NicAddrInfoV4);
}

const CMD_GET_NIC_ADDR_INFO: u16 = 0x1001;
const CMD_SET_NIC_ADDR_INFO: u16 = 0x1002;
const CMD_GET_NIC_MAC_ADDR: u16 = 0x1003;

/// Describes a NetworkDevice
pub struct NetworkDev<'a>(pub &'a dyn NetworkInterface);

impl<'a> CharDevice for NetworkDev<'a> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn read(&self, buffer: &mut [u8]) -> crate::drivers::vfs::FSResult<usize> {
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }

    fn write(&self, buffer: &[u8]) -> crate::drivers::vfs::FSResult<usize> {
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }

    fn send_command(&self, cmd: u16, arg: u64) -> crate::drivers::vfs::FSResult<()> {
        match cmd {
            CMD_GET_NIC_ADDR_INFO => {
                let ptr: &mut NicAddrInfoV4 =
                    SyscallFFI::make(arg as *mut NicAddrInfoV4).map_err(|_| FSError::InvalidArg)?;
                *ptr = self.0.nic_info();
                Ok(())
            }
            CMD_SET_NIC_ADDR_INFO => {
                let ptr: &mut NicAddrInfoV4 =
                    SyscallFFI::make(arg as *mut NicAddrInfoV4).map_err(|_| FSError::InvalidArg)?;
                self.0.set_nic_info(*ptr);
                Ok(())
            }
            CMD_GET_NIC_MAC_ADDR => {
                let ptr: &mut MacAddress =
                    SyscallFFI::make(arg as *mut _).map_err(|_| FSError::InvalidArg)?;
                *ptr = self.0.mac_address();
                Ok(())
            }
            _ => Err(FSError::InvalidCmd),
        }
    }
}
