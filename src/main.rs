use std::net::{Ipv6Addr, SocketAddrV6};

use dhcproto::v6::{DhcpOption, DhcpOptions, IAPrefix, MessageType, OptionCode};
use futures::FutureExt as _;
use nix::{net::if_::InterfaceFlags, sys::socket::SockaddrLike};
use router::dhcp::Server;

async fn choose_advertisement(
    resps: &mut tokio::sync::mpsc::Receiver<(dhcproto::v6::Message, SocketAddrV6)>,
    timeout_token: tokio_util::sync::CancellationToken,
) -> Option<(dhcproto::v6::Message, SocketAddrV6)> {
    let mut chosen_pref = 0;
    let mut chosen_advertise = None;
    let advertise_chooser_loop = async {
        while let Some((resp, addr)) = resps.recv().await {
            if resp.msg_type() != MessageType::Advertise {
                eprintln!("Expected Advertise... got {:?}", resp.msg_type());
                eprintln!("Ignoring...");
                continue;
            }
            eprintln!("{}", resp);
            let options = resp.opts();
            let Some(dhcproto::v6::DhcpOption::Preference(pref)) =
                options.get(dhcproto::v6::OptionCode::Preference)
            else {
                eprintln!("Failed to get DHCP Preference option... continuing.");
                if chosen_advertise == None {
                    chosen_advertise = Some((resp, addr));
                }
                continue;
            };
            if *pref > chosen_pref {
                chosen_pref = *pref;
                chosen_advertise = Some((resp, addr));
            }
        }
    };
    futures::select! {
        _ = std::pin::pin!(timeout_token.cancelled().fuse()) => {
            return chosen_advertise;
        }
        _ = std::pin::pin!(advertise_chooser_loop.fuse()) => {
            unreachable!() // this should listen forever, until we're done looking for advertisements...
        }
    }
}
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

        let response_timeout_tok = tokio_util::sync::CancellationToken::new();

        tokio::spawn({
            let response_timeout_tok = response_timeout_tok.clone();
            async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                response_timeout_tok.cancel();
            }
        });
        let chosen_advertise = choose_advertisement(&mut resps, response_timeout_tok.clone()).await;
        eprintln!("No longer listening for ADVERTISEMENT.");
        let Some((advertise, addr)) = chosen_advertise else {
            eprintln!("Failed to get a advertise...");
            return;
        };

        let server = Server::from_msg(&advertise, addr).expect("No Server Id Found");

        let Some(req_iapd) = advertise.opts().get(OptionCode::IAPD) else {
            panic!("Server didn't give us an IAPD...");
        };

        let mut req_opts = DhcpOptions::new();
        req_opts.insert(req_iapd.clone());

        let resp = client.request(&server, req_opts).await;
        dbg! {resp};
        clients.push(client);
        eprintln!("les gooo");
    }
    eprintln!("Hello, world!");
    loop {}
}
