use rand::Rng;
use thiserror::Error;

use crate::{
    nic::Nic,
    packet::{DHCPOp, DHCPOpt, DHCPPacket},
};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::Duration,
};

#[derive(Error, Debug)]
pub enum DHCPError {
    #[error("Failed to Send packet: {0}.")]
    Send(io::Error),
    #[error("Failed to Recv packet: {0}.")]
    Recv(io::Error),
    #[error("Discover operation failed: {0}.")]
    DiscoverFailed(&'static str),
    #[error("Request operation failed: {0}.")]
    RequestFailed(&'static str),
}

impl Into<io::Error> for DHCPError {
    fn into(self) -> io::Error {
        match self {
            Self::DiscoverFailed(str) => io::Error::new(io::ErrorKind::Other, str),
            Self::RequestFailed(str) => io::Error::new(io::ErrorKind::Other, str),
            Self::Send(e) => e,
            Self::Recv(e) => e,
        }
    }
}

pub type Result<T> = std::result::Result<T, DHCPError>;

#[derive(Debug, Clone)]
pub struct DHCPOffer {
    #[allow(unused)]
    pub dns: Vec<Ipv4Addr>,
    pub router: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub our_addr: Ipv4Addr,
    pub server_addr: Ipv4Addr,
    pub lease_time: u32,
    pub offered_from: SocketAddr,
}

/// An Instance of a DHCP Client that receives and sends DHCP packets from and to the DHCP server.
pub struct DHCPClient {
    socket: UdpSocket,
    trans_id: u32,
    packet_storage: [u8; size_of::<DHCPPacket>()],
    mac: [u8; 6],
}

impl DHCPClient {
    pub fn create(nic: &Nic) -> io::Result<Self> {
        let mac = nic.mac();
        println!(
            "Creating a DHCPClient for mac: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 68))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        let mut rng = rand::rng();
        let trans_id = rng.random::<u32>();
        Ok(Self {
            trans_id,
            socket,
            packet_storage: [0u8; size_of::<DHCPPacket>()],
            mac,
        })
    }

    #[inline(always)]
    const unsafe fn last_packet_unchecked(&self) -> &DHCPPacket {
        unsafe { core::mem::transmute::<_, &DHCPPacket>(&self.packet_storage) }
    }

    #[inline]
    pub fn last_packet(&self) -> Option<&DHCPPacket> {
        let last_packet = unsafe { self.last_packet_unchecked() };
        last_packet.is_valid().then_some(last_packet)
    }

    pub fn recv_packet(
        &mut self,
        recv_from: Option<SocketAddr>,
    ) -> Result<(&DHCPPacket, SocketAddr)> {
        let trans_id = self.trans_id;

        loop {
            let socket = &self.socket;
            let packet_storage = &mut self.packet_storage;

            let (len, addr) = socket
                .recv_from(packet_storage)
                .map_err(|e| DHCPError::Recv(e))?;
            if let Some(to_accept) = recv_from
                && addr != to_accept
            {
                continue;
            }
            assert_ne!(len, 0);

            if let Some(packet) = self.last_packet()
                && packet.xid() == trans_id
            {
                // Workaround lifetime errors...
                return Ok((unsafe { self.last_packet_unchecked() }, addr));
            }
        }
    }

    pub fn broadcast(&mut self, packet: &DHCPPacket) -> Result<()> {
        let payload = packet.as_bytes();
        match self
            .socket
            .send_to(payload, SocketAddrV4::new(Ipv4Addr::BROADCAST, 67))
        {
            Err(e) => return Err(DHCPError::Send(e)),
            Ok(am) => assert_eq!(am, payload.len()),
        }
        Ok(())
    }

    /// Sends a DISCOVER dhcp operation and tries to find an offer, Returning an error on failure.
    pub fn discover(&mut self) -> Result<DHCPOffer> {
        let packet = DHCPPacket::new(
            DHCPOp::DISCOVER,
            self.mac,
            self.trans_id,
            &[
                DHCPOpt::MSG_TYPE,
                DHCPOpt(1),
                DHCPOpt::DHCPDISCOVER,
                DHCPOpt::PARAMETER_REQ,
                DHCPOpt(3),
                DHCPOpt::DNS,
                DHCPOpt::SUBNET_MASK,
                DHCPOpt::ROUTER,
                DHCPOpt::END,
            ],
            Ipv4Addr::UNSPECIFIED,
        );

        self.broadcast(&packet)?;
        let (received, addr) = self.recv_packet(None)?;
        let options = received.parse_known_options();
        if received.op() != DHCPOp::OFFER || options.msg_type != Some(DHCPOpt::DHCPOFFER) {
            return Err(DHCPError::DiscoverFailed("Expected an OFFER"));
        }

        let (dns, router, subnet_mask, lease_time) = (|| {
            Some((
                options.dns?,
                options.router?,
                options.subnet_mask?,
                options.lease_time?,
            ))
        })()
        .ok_or(DHCPError::DiscoverFailed("OFFER missing options"))?;

        Ok(DHCPOffer {
            dns,
            router,
            subnet_mask,
            lease_time,
            our_addr: received.yiaddr(),
            server_addr: received.siaddr(),
            offered_from: addr,
        })
    }

    /// Send a DHCP Request Operation requesting an `ip`, the reply is an acknowledgement.
    ///
    /// Should be done after [`Self::discover`], which returns an offer containing:
    /// ip: Offered IP Address
    /// server_addr: The IP Address of the DHCP Server
    /// accept_from: The IP address that sent the offer, should be the same as `server_addr`.
    pub fn request(
        &mut self,
        ip: Ipv4Addr,
        server_addr: Ipv4Addr,
        accept_from: SocketAddr,
    ) -> Result<()> {
        let mac = self.mac;
        let trans_id = self.trans_id;

        let our_ip_octets = ip.octets();
        let server_ip_octets = server_addr.octets();

        let request = DHCPPacket::new(
            DHCPOp::DISCOVER,
            mac,
            trans_id,
            &[
                DHCPOpt::MSG_TYPE,
                DHCPOpt(1),
                DHCPOpt::DHCPREQUEST,
                DHCPOpt::REQUEST_IP,
                DHCPOpt(4),
                DHCPOpt(our_ip_octets[0]),
                DHCPOpt(our_ip_octets[1]),
                DHCPOpt(our_ip_octets[2]),
                DHCPOpt(our_ip_octets[3]),
                DHCPOpt::DHCP_SERVER_ID,
                DHCPOpt(4),
                DHCPOpt(server_ip_octets[0]),
                DHCPOpt(server_ip_octets[1]),
                DHCPOpt(server_ip_octets[2]),
                DHCPOpt(server_ip_octets[3]),
                DHCPOpt::END,
            ],
            server_addr,
        );

        self.broadcast(&request)?;

        let (received, _) = self.recv_packet(Some(accept_from))?;
        let options = received.parse_known_options();
        if received.op() != DHCPOp::OFFER || options.msg_type != Some(DHCPOpt::DHCPACK) {
            return Err(DHCPError::RequestFailed("Acknowledgement failed"));
        }
        Ok(())
    }
}
