use std::{
    collections::HashMap,
    net::{SocketAddr, SocketAddrV6},
};

mod error;

use crate::dhcp::{Server, codec::DhcpV6Codec};
use futures::{FutureExt as _, SinkExt, StreamExt};
use rand::Rng;
use tokio_util::udp::UdpFramed;

#[derive(Debug)]
enum DhcpActorMsg {
    Solicit(
        dhcproto::v6::Message,
        tokio::sync::mpsc::Sender<(dhcproto::v6::Message, SocketAddrV6)>,
    ),
}

struct DhcpClientWriteActor {
    sink: futures::stream::SplitSink<
        tokio_util::udp::UdpFramed<DhcpV6Codec>,
        (dhcproto::v6::Message, std::net::SocketAddr),
    >,
    sub_channel: tokio::sync::mpsc::Sender<(
        [u8; 3],
        tokio::sync::mpsc::Sender<(dhcproto::v6::Message, std::net::SocketAddrV6)>,
    )>,
    rx: tokio::sync::mpsc::Receiver<DhcpActorMsg>,
}

impl DhcpClientWriteActor {
    async fn new(
        addr: SocketAddrV6,
        rx: tokio::sync::mpsc::Receiver<DhcpActorMsg>,
    ) -> Result<Self, std::io::Error> {
        eprintln!("binding too: {}", addr);
        let sock = tokio::net::UdpSocket::bind(addr).await?;
        eprintln!("binded");
        let (sink, stream) = UdpFramed::new(sock, DhcpV6Codec {}).split();
        let (sub_channel_tx, sub_channel_rx) = tokio::sync::mpsc::channel(10);
        let reader = DhcpClientReadActor::new(stream, sub_channel_rx);

        tokio::spawn(reader.run());
        Ok(Self {
            rx,
            sink,
            sub_channel: sub_channel_tx,
        })
    }
    pub fn run(mut self) {
        eprintln!("Started client...");

        tokio::spawn(async move {
            loop {
                let msg = self.rx.recv().await;

                match msg {
                    None => return,
                    Some(msg) => self.handle_msg(msg).await,
                }
            }
        });
    }
    pub async fn handle_msg(&mut self, msg: DhcpActorMsg) {
        match msg {
            DhcpActorMsg::Solicit(message, sender) => {
                self.sub_channel
                    .send((message.xid(), sender))
                    .await
                    .unwrap();
                eprintln!("Sending solicit");
                self.sink
                    .send((
                        message,
                        SocketAddr::new(
                            std::net::IpAddr::V6(crate::dhcp::ALL_DHCP_RELAY_AGENTS_AND_SERVERS),
                            crate::dhcp::SERVER_PORT,
                        ),
                    ))
                    .await
                    .unwrap();
                eprintln!("sent solicit!");
            }
        }
    }
}

/// This actor receives messages from the UDP socket, and sends them to whatever
/// is subscribed to a message of that transaction ID.
struct DhcpClientReadActor {
    stream: futures::stream::SplitStream<tokio_util::udp::UdpFramed<DhcpV6Codec>>,
    subscribers: HashMap<
        [u8; 3],
        tokio::sync::mpsc::Sender<(dhcproto::v6::Message, std::net::SocketAddrV6)>,
    >,
    subscribe_channel: tokio::sync::mpsc::Receiver<(
        [u8; 3],
        tokio::sync::mpsc::Sender<(dhcproto::v6::Message, std::net::SocketAddrV6)>,
    )>,
}

impl DhcpClientReadActor {
    fn new(
        stream: futures::stream::SplitStream<tokio_util::udp::UdpFramed<DhcpV6Codec>>,
        subscribe_channel: tokio::sync::mpsc::Receiver<(
            [u8; 3],
            tokio::sync::mpsc::Sender<(dhcproto::v6::Message, std::net::SocketAddrV6)>,
        )>,
    ) -> Self {
        Self {
            stream,
            subscribers: std::collections::HashMap::new(),
            subscribe_channel,
        }
    }
    async fn run(self) {
        let mut incoming_dhcp_msg_stream = self.stream;
        let mut subscribe_channel = self.subscribe_channel;
        let mut subscribers = self.subscribers;

        let mut next_dhcp_msg_fut = Box::pin(incoming_dhcp_msg_stream.next().fuse());
        let mut next_sub = Box::pin(subscribe_channel.recv().fuse());

        loop {
            futures::select! {
                dhcp_msg = next_dhcp_msg_fut => {
                    next_dhcp_msg_fut = Box::pin(incoming_dhcp_msg_stream.next().fuse());
                    match dhcp_msg {
                        Some(Ok((msg, addr))) => {
                            let Some(channel) = subscribers.get_mut(&msg.xid()) else {
                                eprintln!("Got message without corresponding transaction ID: {}", msg);
                                eprintln!("No longer listening?");
                                continue;
                            };
                            let SocketAddr::V6(addr) = addr else {
                                panic!("Got message on v4 address: {}?!?!?!", addr);
                            };

                            eprintln!("got a message! {msg}");

                            // send the message, otherwise remove the entry from the map because
                            // the receiver is no longer subscribed if the receiving channel has
                            // been dropped.
                            let xid = msg.xid();
                            let Ok(_) = channel.send((msg, addr)).await else {
                                subscribers.remove_entry(&xid);
                                continue;
                            };
                        }
                        Some(Err(e)) => {
                            eprintln!("{}", e);
                        }
                        // Socket closed? Dunno, but we're done here.
                        None => return,
                    }

                }
                sub = next_sub => {
                    std::mem::drop(next_sub);
                    next_sub = Box::pin(subscribe_channel.recv().fuse());
                    match sub {
                        // we're done!
                        None => return,
                        Some((transaction_id, channel)) => {
                            subscribers.insert(transaction_id, channel);
                        }
                    }
                }
            };
        }
    }
}

/// A DhcpClient bound to a specific address.
/// The DHCP Client fills in client ID on your behalf. This
/// is the *only* thing it fullfills, and aside from guaranteeing
/// you get a response with a matching transaction ID, no further guarantees
/// are made. You *must* validate messages yourself.
pub struct DhcpClient {
    tx: tokio::sync::mpsc::Sender<DhcpActorMsg>,
    rng: rand::rngs::ThreadRng,
    client_id: Vec<u8>,
}
impl DhcpClient {
    /// Create a dhcp client that listens on `addr`.
    /// Manages a spawned DHCP worker task in the background.
    pub async fn new(addr: SocketAddrV6, client_id: Vec<u8>) -> Result<Self, std::io::Error> {
        // TODO:
        // not sure what this should be. Pick a random number.
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let client = DhcpClientWriteActor::new(addr, rx).await?;
        client.run();
        let rng = rand::rng();
        Ok(Self {
            tx,
            client_id,
            rng: rng,
        })
    }
    pub async fn solicit(
        &mut self,
        opts: dhcproto::v6::DhcpOptions,
    ) -> tokio::sync::mpsc::Receiver<(dhcproto::v6::Message, SocketAddrV6)> {
        let mut options = dhcproto::v6::DhcpOptions::new();
        options.insert(dhcproto::v6::DhcpOption::ClientId(self.client_id.clone()));
        let options = options.into_iter().chain(opts).collect();
        let mut msg = dhcproto::v6::Message::new(dhcproto::v6::MessageType::Solicit);
        msg.set_xid(self.rng.random()).set_opts(options);
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        self.tx
            .send(DhcpActorMsg::Solicit(msg, tx))
            .await
            .expect("failed to send SOLICIT over channel");
        rx
    }
}
