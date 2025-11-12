use std::{
    net::ToSocketAddrs,
    time::{Duration, Instant},
};

use safa_api::{
    abi::sockets::SockMsgFlags,
    errors::{ErrorStatus, SysResult},
    sockets::{Socket, SocketDomain, SocketKind, socket::SocketOpt},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ICMPType(u8);

impl ICMPType {
    const ECHO_REQUEST: Self = Self(8);
    const ECHO_REPLY: Self = Self(0);
}

#[repr(C)]
struct EchoIcmpPacket {
    ty: ICMPType,
    code: u8,
    checksum: u16,
    identifier: u16,
    sequence_number: u16,
    data: [u8; 56],
}

impl EchoIcmpPacket {
    pub const fn new(ty: ICMPType, code: u8, seq_num: u16, data: [u8; 56]) -> Self {
        Self {
            ty,
            code,
            checksum: 0,
            identifier: 0,
            sequence_number: seq_num.to_be(),
            data,
        }
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { core::mem::transmute::<&Self, &[u8; size_of::<Self>()]>(self) }
    }

    pub const fn data(&self) -> &[u8; 56] {
        &self.data
    }

    pub const fn seq_num(&self) -> u16 {
        u16::from_be(self.sequence_number)
    }

    pub const fn set_seq(&mut self, seq: u16) {
        self.sequence_number = seq.to_be();
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bytes
            .first_chunk::<{ size_of::<Self>() }>()
            .map(|chunk| unsafe { core::mem::transmute(*chunk) })
    }
}

const TIMEOUT_SECONDS: u64 = 1;

fn main() -> SysResult {
    let mut args = std::env::args();

    let program = args.next().expect("no program name provided");
    let Some(hostname) = args.next() else {
        println!("Usage: {} <hostname>", program);
        std::process::exit(1);
    };

    let addr = (hostname.as_str(), 0)
        .to_socket_addrs()
        .expect("Failed to resolve hostname")
        .next()
        .expect("Failed to resolve hostname");

    let socket = Socket::builder(
        SocketDomain::Ipv4,
        SocketKind::Datagram,
        1, /* IPV4 ICMP Protocol */
    )
    .build()
    .expect("Failed to create socket");
    // Timeout after 1 second
    socket
        .set_sock_opt(SocketOpt::ReadTimeout, TIMEOUT_SECONDS * 1000)
        .expect("Failed to setup socket");

    let data: [u8; 56] = std::array::from_fn(|index| b'a' + index as u8);

    let mut curr_seq = 1;
    let mut packet = EchoIcmpPacket::new(ICMPType::ECHO_REQUEST, 0, curr_seq, data);

    println!(
        "pinging {hostname} ({}) with {} bytes",
        addr.ip(),
        data.len()
    );

    let mut recv_buf = [0u8; size_of::<EchoIcmpPacket>()];
    loop {
        packet.set_seq(curr_seq);
        curr_seq += 1;

        if let Err(e) = socket.send_to_addr(packet.as_bytes(), SockMsgFlags::NONE, addr) {
            println!("error sending packet: {}", e.as_str());
            return SysResult::err(e);
        }

        let instat = Instant::now();
        let (am, from_addr) = match socket.recv_from_addr(&mut recv_buf, SockMsgFlags::NONE) {
            Err(ErrorStatus::Timeout) => {
                println!("timeout sending packet, retrying...");
                continue;
            }
            Err(e) => {
                println!("error receiving packet: {}, retrying...", e.as_str());
                let elapsed = instat.elapsed();
                if let Some(dur) = Duration::from_secs(TIMEOUT_SECONDS).checked_sub(elapsed) {
                    std::thread::sleep(dur);
                }
                continue;
            }
            Ok(k) => k,
        };

        let elapsed = instat.elapsed();
        let Some(received_packet) = EchoIcmpPacket::from_bytes(&recv_buf[..am]) else {
            println!("invalid packet received");
            continue;
        };

        if received_packet.ty == ICMPType::ECHO_REPLY {
            if received_packet.data() != &data {
                println!(
                    "received {} invalid bytes from {}: icmp_seq={} time={}ms",
                    am,
                    from_addr.ip(),
                    received_packet.seq_num(),
                    elapsed.as_millis()
                );
            } else {
                println!(
                    "received {am} bytes from {}: icmp_seq={} time={}ms",
                    from_addr.ip(),
                    received_packet.seq_num(),
                    elapsed.as_millis()
                );
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}
