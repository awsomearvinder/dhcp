use std::net::Ipv6Addr;

pub mod decoder;

pub const ALL_DHCP_RELAY_AGENTS_AND_SERVERS: Ipv6Addr =
    Ipv6Addr::new(0xff02, 0x0, 0, 0, 0, 0, 0x01, 0x02);
pub const SERVER_PORT: u16 = 547;
pub const CLIENT_PORT: u16 = 546;
