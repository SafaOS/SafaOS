use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct DHCPOp(u8);

impl DHCPOp {
    pub const DISCOVER: Self = Self(1);
    pub const OFFER: Self = Self(2);
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
struct HType(u8);

impl HType {
    pub const ETHERNET: Self = Self(1);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct DHCPOpt(pub u8);

impl DHCPOpt {
    pub const MSG_TYPE: Self = Self(53);
    pub const DHCPDISCOVER: Self = Self(1);
    pub const DHCPOFFER: Self = Self(2);
    pub const DHCPREQUEST: Self = Self(3);
    pub const DHCPACK: Self = Self(5);
    pub const PARAMETER_REQ: Self = Self(55);
    pub const DNS: Self = Self(6);
    pub const SUBNET_MASK: Self = Self(1);
    pub const ROUTER: Self = Self(3);
    pub const DHCP_SERVER_ID: Self = Self(54);
    pub const LEASE_TIME: Self = Self(51);
    pub const REQUEST_IP: Self = Self(50);
    pub const END: Self = Self(255);
}

#[derive(Debug, Clone, Default)]
pub struct ParsedDHCPOptions {
    pub msg_type: Option<DHCPOpt>,
    pub subnet_mask: Option<Ipv4Addr>,
    pub dhcp_server_addr: Option<Ipv4Addr>,
    pub lease_time: Option<u32>,
    pub dns: Option<Vec<Ipv4Addr>>,
    pub router: Option<Ipv4Addr>,
}

#[derive(Debug)]
#[repr(C)]
pub struct DHCPPacket {
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
    options: [u8; 256],
}

impl DHCPPacket {
    pub const MAGIC: u32 = 0x63825363u32;

    pub fn new(
        op: DHCPOp,
        mac: [u8; 6],
        xid: u32,
        options: &[DHCPOpt],
        server_ip_addr: Ipv4Addr,
    ) -> Self {
        let mut raw_options = [0; 256];
        raw_options[..options.len()].copy_from_slice(unsafe { core::mem::transmute(options) });

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
            siaddr: server_ip_addr,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            server_name: [0; 64],
            boot_file_name: [0; 128],
            magic: Self::MAGIC.to_be_bytes(),
            options: raw_options,
        }
    }

    #[inline]
    pub const fn op(&self) -> DHCPOp {
        self.op
    }

    #[inline]
    pub const fn magic(&self) -> u32 {
        u32::from_be_bytes(self.magic)
    }

    #[inline]
    pub const fn is_valid(&self) -> bool {
        self.magic() == Self::MAGIC
    }

    #[inline]
    pub const fn xid(&self) -> u32 {
        u32::from_be_bytes(self.xid)
    }

    #[inline]
    /// Your IP Address
    pub const fn yiaddr(&self) -> Ipv4Addr {
        self.yiaddr
    }
    #[inline]
    /// Server IP Address
    pub const fn siaddr(&self) -> Ipv4Addr {
        self.siaddr
    }

    pub const fn as_bytes(&self) -> &[u8] {
        unsafe { &*(self as *const Self as *const [u8; size_of::<Self>()]) }
    }

    pub fn parse_known_options(&self) -> ParsedDHCPOptions {
        let mut parsed = ParsedDHCPOptions::default();
        let slice = &self.options;
        let mut iter = slice.iter().copied();

        while let Some(opt) = iter.next() {
            let opt = DHCPOpt(opt);

            let mut try_parse_option = || {
                macro_rules! make_u32 {
                    () => {{
                        let a = iter.next()?;
                        let b = iter.next()?;
                        let c = iter.next()?;
                        let d = iter.next()?;
                        Some(u32::from_be_bytes([a, b, c, d]))
                    }};
                }
                macro_rules! make_ipv4 {
                    () => {{
                        let bits = make_u32!()?;
                        Some(Ipv4Addr::from_bits(bits))
                    }};
                }

                match opt {
                    DHCPOpt::MSG_TYPE => {
                        let size = iter.next()?;
                        if size == 1 {
                            let msg_type = iter.next()?;
                            parsed.msg_type = Some(DHCPOpt(msg_type));
                        }
                    }
                    DHCPOpt::ROUTER => {
                        let size = iter.next()?;
                        if let Some(count) = size.is_multiple_of(4).then_some(size as usize / 4)
                            && count > 0
                        {
                            parsed.router = Some(make_ipv4!()?);
                            for _ in 1..count {
                                // Only takes the first router's IP
                                _ = make_ipv4!()?;
                            }
                        }
                    }
                    DHCPOpt::SUBNET_MASK => {
                        let size = iter.next()?;
                        if size == 4 {
                            parsed.subnet_mask = Some(make_ipv4!()?);
                        }
                    }
                    DHCPOpt::DNS => {
                        let size = iter.next()?;
                        if let Some(count) = size.is_multiple_of(4).then_some(size as usize / 4) {
                            let mut dns_servers = Vec::with_capacity(count);
                            for _ in 0..count {
                                dns_servers.push(make_ipv4!()?);
                            }
                            parsed.dns = Some(dns_servers);
                        }
                    }
                    DHCPOpt::LEASE_TIME => {
                        let size = iter.next()?;
                        if size == 4 {
                            parsed.lease_time = Some(make_u32!()?);
                        }
                    }
                    DHCPOpt::DHCP_SERVER_ID => {
                        let size = iter.next()?;
                        if size == 4 {
                            parsed.dhcp_server_addr = Some(make_ipv4!()?);
                        }
                    }
                    DHCPOpt::END => return None,
                    _ => {}
                };

                Some(())
            };

            if try_parse_option().is_none() {
                break;
            }
        }
        parsed
    }
}
