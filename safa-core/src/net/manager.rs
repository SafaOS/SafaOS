use core::net::Ipv4Addr;

use safa_abi::errors::{ErrorStatus, IntoErr};

use crate::{
    debug,
    drivers::vfs::VFS_STRUCT,
    net::{
        MacAddress,
        arp::ARP,
        ethernet::{EthernetFrame, EthernetType},
        interface::{NetIntError, NetworkInterface, NetworkInterfaceSuper},
        ipv4::{self, IPv4Packet},
    },
    utils::locks::{RwLock, RwLockReadGuard},
    warn,
};

#[derive(Debug, Clone, Copy)]
pub enum NetworkError {
    PayloadTooLarge,
    NoInterface,
    AddressNotFound,
}

impl IntoErr for NetworkError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::PayloadTooLarge => ErrorStatus::StrTooLong,
            Self::NoInterface => ErrorStatus::Unknown,
            Self::AddressNotFound => ErrorStatus::AddressNotFound,
        }
    }
}

impl From<NetIntError> for NetworkError {
    fn from(error: NetIntError) -> Self {
        match error {
            NetIntError::PacketTooLarge => NetworkError::PayloadTooLarge,
        }
    }
}

pub struct NetworkManager {
    interfaces: RwLock<heapless::Vec<&'static dyn NetworkInterface, 256>>,
}

static MANAGER: NetworkManager = NetworkManager {
    interfaces: RwLock::new(heapless::Vec::new()),
};

impl NetworkManager {
    pub fn interfaces(
        &self,
    ) -> RwLockReadGuard<'_, heapless::Vec<&'static dyn NetworkInterface, 256>> {
        self.interfaces.read()
    }

    /// Adds a network interface to the manager.
    pub fn add_interface(&self, interface: &'static dyn NetworkInterfaceSuper) {
        self.interfaces
            .write()
            .push(interface)
            .map_err(|_| ())
            .expect("Too much network interfaces");
        crate::devices::add_device_at(&*VFS_STRUCT.read(), interface, "net");
    }

    /// Handles incoming packets.
    pub fn handle_packet(&self, interface: &'static dyn NetworkInterface, packet: &mut [u8]) {
        if packet.len() < 14 {
            warn!(NetworkManager, "Received packet too short, dropping...");
            return;
        }

        let frame = EthernetFrame::from_bytes_mut(packet);
        // TODO: Handle the packet
        debug!(NetworkManager, "Received packet: {:#x?}", frame);
        match frame.header.ethertype {
            EthernetType::IPV4 => ipv4::handle_ipv4_packet(interface, &mut frame.payload),
            other => debug!(NetworkManager, "Unknown ethertype: {:#x?}", other),
        }
    }

    /// Sends an Ethernet frame through the network interface.
    pub fn send_ethernet(
        &self,
        interface: &dyn NetworkInterface,
        ethertype: EthernetType,
        target_mac: MacAddress,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        interface.send_ethernet(target_mac, ethertype, payload)?;
        Ok(())
    }

    pub fn send_ipv4_request(
        &self,
        src_addr: Ipv4Addr,
        packet: &mut IPv4Packet,
    ) -> Result<(), NetworkError> {
        let _header = packet.header();
        // TODO: Arp
        for interface in &*self.interfaces.read() {
            let ip = interface.ipv4_address();
            if src_addr == Ipv4Addr::UNSPECIFIED || ip == src_addr {
                packet.header_mut().src_addr = interface.nic_info().ipv4_address;
                packet.put_checksum();

                interface.send_ethernet(
                    MacAddress::BROADCAST,
                    EthernetType::IPV4,
                    packet.as_bytes(),
                )?;
            }
        }

        Ok(())
    }
}

/// Sends an Ethernet frame through the network interface.
pub fn send_ethernet(
    interface: &dyn NetworkInterface,
    ethertype: EthernetType,
    target_mac: MacAddress,
    payload: &[u8],
) -> Result<(), NetworkError> {
    MANAGER.send_ethernet(interface, ethertype, target_mac, payload)
}

/// Handles incoming packets.
pub fn handle_packet(interface: &'static dyn NetworkInterface, packet: &mut [u8]) {
    MANAGER.handle_packet(interface, packet);
}

/// Adds a network interface to the manager.
pub fn add_interface(interface: &'static dyn NetworkInterfaceSuper) {
    MANAGER.add_interface(interface);
}

/// Sends an IPv4 packet through a network interface which can receive it, returns [`NetworkError::AddressNotFound`] if the Ip Address isn't supported by any interface.
pub fn send_ipv4_packet(src_addr: Ipv4Addr, packet: &mut IPv4Packet) -> Result<(), NetworkError> {
    MANAGER.send_ipv4_request(src_addr, packet)
}

#[allow(dead_code)]
/// Sends an ARP request to identify the MAC address of a given IP address.
pub fn send_arp(target_ip: Ipv4Addr) -> Result<(), NetworkError> {
    let interfaces = &*MANAGER.interfaces();
    if interfaces.is_empty() {
        return Err(NetworkError::NoInterface);
    }

    for int in interfaces {
        send_ethernet(
            *int,
            EthernetType::ARP,
            MacAddress::BROADCAST,
            ARP::new_request(int.mac_address(), int.nic_info().ipv4_address, target_ip).as_bytes(),
        )?;
    }
    Ok(())
}

#[test_case]
fn a_test_send_arp() {
    use crate::warn;

    match send_arp(Ipv4Addr::new(192, 168, 42, 42)) {
        Ok(()) => (),
        Err(NetworkError::NoInterface) => warn!("No network interface found"),
        Err(e) => panic!("Failed to send ARP request: {:?}", e),
    }
}
