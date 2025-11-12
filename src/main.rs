use std::net::{Ipv6Addr, SocketAddrV6};

use dhcproto::v6::{DhcpOption, DhcpOptions, IAPrefix, MessageType};
use nix::{net::if_::InterfaceFlags, sys::socket::SockaddrLike};

#[tokio::main]
async fn main() {
    let mut clients = vec![];
    for iface in nix::ifaddrs::getifaddrs().unwrap() {
        // for testing purposes...
        if iface.interface_name == "eth0" {
            continue;
        }
        let flags = iface.flags.bits();
        let Some(addr) = iface.address else { continue };
        if addr.family() != Some(nix::sys::socket::AddressFamily::Inet6) {
            continue;
        }
        // if it's not up or it's a loopback address, we gtfo
        if flags & InterfaceFlags::IFF_UP.bits() == 0
            || flags & InterfaceFlags::IFF_LOOPBACK.bits() != 0
        {
            continue;
        }
        let addr = addr.as_sockaddr_in6().unwrap();
        let is_link_local = addr.ip().is_unicast_link_local();
        if !is_link_local {
            continue;
        }
        eprintln!("Listening on DHCP for {}:{}", iface.interface_name, addr);
        let addr = dbg!(SocketAddrV6::new(
            addr.ip(),
            router::dhcp::CLIENT_PORT,
            addr.flowinfo(),
            addr.scope_id()
        ));
        let mut client =
            router::client::DhcpClient::new(addr, Vec::from(rand::random::<[u8; 16]>()))
                .await
                .unwrap();
        let options = vec![
            DhcpOption::ElapsedTime(0),
            DhcpOption::IAPD(dhcproto::v6::IAPD {
                id: 0,
                t1: 0,
                t2: 0,
                opts: vec![DhcpOption::IAPrefix(IAPrefix {
                    preferred_lifetime: 0,
                    valid_lifetime: 0,
                    prefix_len: 64,
                    prefix_ip: Ipv6Addr::UNSPECIFIED,
                    opts: DhcpOptions::new(),
                })]
                .into_iter()
                .collect(),
            }),
        ];
        let mut resps = client.solicit(options.into_iter().collect()).await;

        while let Some((resp, addr)) = resps.recv().await {
            assert!(resp.msg_type() == MessageType::Advertise);
            eprintln!("{}", resp);
        }

        clients.push(client);
        eprintln!("les gooo");
    }
    eprintln!("Hello, world!");
    loop {}
}
