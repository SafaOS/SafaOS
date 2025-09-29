use core::net::Ipv4Addr;

use lazy_static::lazy_static;

use crate::{
    debug,
    drivers::{net::e1000::E1000NetCard, pci::E1000_DEVICE},
    net::{
        MacAddress,
        arp::ARP,
        ethernet::{EthernetFrame, EthernetType},
    },
    warn,
};

#[derive(Debug, Clone, Copy)]
pub enum NetworkError {
    PayloadTooLarge,
    NoInterface,
}

#[derive(Debug)]
pub struct NetworkManager {
    chip: &'static E1000NetCard,
}

lazy_static! {
    static ref MANAGER: Option<NetworkManager> =
        // TODO: Handle more Nics at the same time
        E1000_DEVICE.as_ref().map(|device| NetworkManager { chip: device });
}

impl NetworkManager {
    /// Handles incoming packets.
    pub fn handle_packet(&self, packet: &[u8]) {
        if packet.len() < 14 {
            warn!(NetworkManager, "Received packet too short, dropping...");
            return;
        }

        let frame = EthernetFrame::from_bytes(packet);
        // TODO: Handle the packet
        debug!(NetworkManager, "Received packet: {:#x?}", frame);
    }

    pub fn mac(&self) -> MacAddress {
        self.chip.mac_address()
    }

    /// Sends an Ethernet frame through the network interface.
    pub fn send_ethernet(
        &self,
        ethertype: EthernetType,
        target_mac: MacAddress,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        self.chip
            .send_ethernet(target_mac, ethertype, payload)
            .map_err(|()| NetworkError::PayloadTooLarge)
    }
}

/// Sends an Ethernet frame through the network interface.
pub fn send_ethernet(
    ethertype: EthernetType,
    target_mac: MacAddress,
    payload: &[u8],
) -> Result<(), NetworkError> {
    MANAGER
        .as_ref()
        .ok_or(NetworkError::NoInterface)?
        .send_ethernet(ethertype, target_mac, payload)
}

/// Handles incoming packets.
pub fn handle_packet(packet: &[u8]) {
    MANAGER.as_ref().unwrap().handle_packet(packet);
}

#[allow(dead_code)]
/// Sends an ARP request to identify the MAC address of a given IP address.
pub fn send_arp(target_ip: Ipv4Addr) -> Result<(), NetworkError> {
    send_ethernet(
        EthernetType::ARP,
        MacAddress::BROADCAST,
        ARP::new_request(
            MANAGER.as_ref().unwrap().mac(),
            Ipv4Addr::new(192, 168, 69, 69),
            target_ip,
        )
        .as_bytes(),
    )
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
