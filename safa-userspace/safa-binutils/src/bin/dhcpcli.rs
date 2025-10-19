use std::{
    fs::File,
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    path::Path,
};

use extra::tri_io;
use safa_api::errors::SysResult;

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
struct DHCPOp(u8);

impl DHCPOp {
    pub const DISCOVER: Self = Self(1);
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
struct HType(u8);

impl HType {
    pub const ETHERNET: Self = Self(1);
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
struct DHCPOpt(u8);

impl DHCPOpt {
    pub const MSG_TYPE: Self = Self(53);
    pub const DHCPDISCOVER: Self = Self(1);
    pub const PARAMETER_REQ: Self = Self(55);
    pub const DNS: Self = Self(6);
    pub const SUBNET_MASK: Self = Self(1);
    pub const ROUTER: Self = Self(3);
    pub const END: Self = Self(255);
}

#[derive(Debug)]
#[repr(C)]
struct DHCPPacket {
    op: DHCPOp,
    htype: HType,
    hlen: u8,
    hops: u8,
    xid: [u8; 4],
    secs: u16,
    flags: u16,
    /// Client IP Address
    ciaddr: Ipv4Addr,
    /// Your IP Address
    yiaddr: Ipv4Addr,
    /// Server IP Address
    siaddr: Ipv4Addr,
    /// Gateway IP Address
    giaddr: Ipv4Addr,
    /// Client Hardware Address
    chaddr: [u8; 16],
    server_name: [u8; 64],
    boot_file_name: [u8; 128],
    magic: [u8; 4],
    options: [DHCPOpt; 256],
}

impl DHCPPacket {
    const MAGIC: u32 = 0x63825363u32;

    pub fn new(op: DHCPOp, mac: [u8; 6], xid: u32, options: &[DHCPOpt]) -> Self {
        let mut raw_options = [DHCPOpt(0); 256];
        raw_options[..options.len()].copy_from_slice(options);

        let mut chaddr = [0; 16];
        chaddr[..mac.len()].copy_from_slice(&mac);

        Self {
            op,
            htype: HType::ETHERNET,
            hlen: 6,
            hops: 0,
            xid: xid.to_be_bytes(),
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            server_name: [0; 64],
            boot_file_name: [0; 128],
            magic: Self::MAGIC.to_be_bytes(),
            options: raw_options,
        }
    }

    pub const fn from_bytes(bytes: [u8; size_of::<Self>()]) -> Result<Self, ()> {
        let this: Self = unsafe { std::mem::transmute(bytes) };
        if this.magic() != Self::MAGIC {
            return Err(());
        }
        Ok(this)
    }

    pub const fn magic(&self) -> u32 {
        u32::from_be_bytes(self.magic)
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { &*(self as *const Self as *const [u8; size_of::<Self>()]) }
    }
}

const GET_NIC_MAC_ADDR: u16 = 0x1003;

fn run(path: &Path) -> io::Result<()> {
    use std::os::safaos::io::IoUtils;

    let nic = File::open(path).expect("Failed to open nic device file");
    let mut mac = [0; 6];
    IoUtils::send_command(&nic, GET_NIC_MAC_ADDR, &raw mut mac as usize as u64)?;

    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 68))?;
    socket.set_broadcast(true)?;
    let discover = DHCPPacket::new(
        DHCPOp::DISCOVER,
        mac,
        0xDEADBEEF,
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
    );

    socket.send_to(
        discover.as_bytes(),
        SocketAddrV4::new(Ipv4Addr::BROADCAST, 67),
    )?;

    let mut packet_bytes = [0; size_of::<DHCPPacket>()];
    let (_, from) = socket.recv_from(&mut packet_bytes)?;

    let packet = DHCPPacket::from_bytes(packet_bytes).expect("Invalid DHCP packet received");
    println!("Received DHCP Packet from: {from}, op: {:?}", packet.op);
    Ok(())
}

fn main() -> SysResult {
    let mut args = std::env::args();
    let _program_name = args.next().expect("Expected program name");
    let path_string = args.next().expect("Expected NIC path to configure");
    let path = Path::new(&path_string);

    tri_io!(run(path));
    SysResult::ok(0)
}
