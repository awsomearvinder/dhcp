use std::net::{Ipv6Addr, SocketAddrV6};

pub mod codec;

pub const ALL_DHCP_RELAY_AGENTS_AND_SERVERS: Ipv6Addr =
    Ipv6Addr::new(0xff02, 0x0, 0, 0, 0, 0, 0x01, 0x02);
pub const SERVER_PORT: u16 = 547;
pub const CLIENT_PORT: u16 = 546;

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Server {
    pub id: Vec<u8>,
    pub addr: SocketAddrV6,
}

impl Server {
    /// Constructs a server using the `ServerId` option in msg, and the addr
    pub fn from_msg(msg: &dhcproto::v6::Message, addr: SocketAddrV6) -> Option<Self> {
        msg.opts()
            .get(dhcproto::v6::OptionCode::ServerId)
            .map(|opt| match opt {
                dhcproto::v6::DhcpOption::ServerId(id) => Self {
                    id: id.clone(),
                    addr,
                },
                _ => unreachable! {},
            })
    }
}
