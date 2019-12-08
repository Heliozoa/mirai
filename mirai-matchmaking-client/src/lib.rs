//! The Mirai matchmaking client facilitates pairing clients by some criteria
//! aided by a matchmaking server that provides peer discovery.
//!
//! The client can send a queue request to a server, after which it will receive
//! some set of peers that the server has selected for it. The client can challenge
//! peers, and accept or decline challenges. The client may receive further
//! peers from the server or request a new set by requeueing.
//!
//! Meanwhile, the clients are evaluating the connection quality to each of its peers
//! by sending ping messages back and forth.

use self::ClientToClient as ToClient;
use self::ClientToClient as FromClient;
use crossbeam_channel::SendError;
use crossbeam_channel::{unbounded, Receiver, Sender};
use laminar::{Packet, Socket, SocketEvent};
use mirai_core::v1::{client::*, CLIENT_PORT, SERVER_PORT};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::collections::{HashMap, HashSet};
use std::convert::From;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::PoisonError;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PING_TIMER_MILLIS: u64 = 100;

type ArMu<T> = Arc<Mutex<T>>;

fn armu<T>(t: T) -> ArMu<T> {
    Arc::new(Mutex::new(t))
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
pub enum ClientToClient {
    Ping(u128),
    PingResponse(u128),
    Challenge,
    Accept,
    Decline,
    Start(u128),
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum PeerStatus {
    None,
    OutgoingChallenge,
    IncomingChallenge,
    Confirmed,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Peer {
    addr: SocketAddr,
    latency: Option<u128>,
    ping_count: u32,
    status: PeerStatus,
}

impl Peer {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            latency: None,
            ping_count: 0,
            status: PeerStatus::None,
        }
    }

    pub fn add_ping(&mut self, ping_latency: u128) {
        self.ping_count += 1;
        match self.latency {
            Some(latency) => self.latency = Some(latency / 2 + ping_latency / 2),
            None => self.latency = Some(ping_latency),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn latency(&self) -> Option<u128> {
        self.latency
    }

    pub fn status(&self) -> PeerStatus {
        self.status
    }
}

impl Hash for Peer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.addr.hash(state);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ServerConnection {
    Connected,
    Disconnected,
    Connecting,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Idle,
    QueuePending,
    Queued,
    MatchPending(SocketAddr),
    MatchConfirmed(SocketAddr),
}

enum Message {
    Quit,
}

pub struct Client {
    status: ArMu<Status>,
    server_addr: SocketAddr,
    server_connection: ArMu<ServerConnection>,
    message_sender: Sender<Message>,
    packet_sender: Sender<Packet>,
    peers: ArMu<HashMap<SocketAddr, Peer>>,
    incoming_challenges: ArMu<HashSet<SocketAddr>>,
    outgoing_challenges: ArMu<HashSet<SocketAddr>>,
    handle: JoinHandle<Result<(Receiver<SocketEvent>, Sender<Packet>), ClientError>>,
}

impl Client {
    pub fn new(addr: IpAddr, server_ip: IpAddr) -> Result<Self, CreateError> {
        let socket_addr = SocketAddr::new(addr, CLIENT_PORT);
        let server_addr = SocketAddr::new(server_ip, SERVER_PORT);
        let mut socket = Socket::bind(socket_addr).context(BindError)?;
        let event_receiver = socket.get_event_receiver();
        let packet_sender = socket.get_packet_sender();
        let thread_packet_sender = socket.get_packet_sender();
        let handle = thread::spawn(move || socket.start_polling());

        let peers = armu(HashMap::new());
        let incoming_challenges = armu(HashSet::new());
        let outgoing_challenges = armu(HashSet::new());
        let thread_peers = Arc::clone(&peers);
        let thread_incoming_challenges = Arc::clone(&incoming_challenges);
        let thread_outgoing_challenges = Arc::clone(&outgoing_challenges);

        let (message_sender, message_receiver) = unbounded();
        let status = armu(Status::Idle);
        let server_connection = armu(ServerConnection::Disconnected);
        let thread_status = Arc::clone(&status);
        let thread_server_connection = Arc::clone(&server_connection);
        let handle = thread::spawn(move || {
            Self::handler(
                server_addr,
                thread_packet_sender,
                event_receiver,
                message_receiver,
                thread_peers,
                thread_outgoing_challenges,
                thread_incoming_challenges,
                thread_status,
                thread_server_connection,
            )
        });
        Ok(Self {
            status,
            server_addr,
            server_connection,
            message_sender,
            packet_sender,
            peers,
            outgoing_challenges,
            incoming_challenges,
            handle,
        })
    }

    fn handler(
        server_addr: SocketAddr,
        packet_sender: Sender<Packet>,
        event_receiver: Receiver<SocketEvent>,
        message_receiver: Receiver<Message>,
        peers: ArMu<HashMap<SocketAddr, Peer>>,
        outgoing_challenges: ArMu<HashSet<SocketAddr>>,
        incoming_challenges: ArMu<HashSet<SocketAddr>>,
        status: ArMu<Status>,
        server_connection: ArMu<ServerConnection>,
    ) -> Result<(Receiver<SocketEvent>, Sender<Packet>), ClientError> {
        let start_time = Instant::now();
        let mut ping_timer = Instant::now() - Duration::from_millis(PING_TIMER_MILLIS);
        loop {
            match event_receiver.try_recv() {
                Ok(SocketEvent::Packet(packet)) => {
                    if packet.addr() != server_addr {
                        match bincode::deserialize::<FromClient>(packet.payload()) {
                            Ok(FromClient::Challenge) => {
                                incoming_challenges.lock()?.insert(packet.addr());
                            }
                            Ok(FromClient::Accept) => {
                                let mut status = status.lock().unwrap();
                                if let Status::Queued = *status {
                                    if outgoing_challenges.lock()?.contains(&packet.addr()) {
                                        let msg = bincode::serialize(&ToClient::Start(0))
                                            .context(SerializeError)?;
                                        packet_sender
                                            .send(Packet::reliable_unordered(packet.addr(), msg))?;
                                        *status = Status::MatchPending(packet.addr());
                                    }
                                }
                            }
                            Ok(FromClient::Decline) => {
                                outgoing_challenges.lock()?.remove(&packet.addr());
                                let mut status = status.lock()?;
                                if let Status::MatchPending(addr) = *status {
                                    if addr == packet.addr() {
                                        // got declined by someone we sent Start to
                                        *status = Status::Queued;
                                    }
                                }
                            }
                            Ok(FromClient::Start(time)) => {
                                let mut status = status.lock()?;
                                if let Status::Queued = *status {
                                    // they are match pending
                                    let msg = bincode::serialize(&ToClient::Start(0))
                                        .context(SerializeError)?;
                                    packet_sender
                                        .send(Packet::reliable_unordered(packet.addr(), msg))?;
                                    incoming_challenges.lock()?.clear();
                                    outgoing_challenges.lock()?.clear();
                                    *status = Status::MatchConfirmed(packet.addr());
                                } else if let Status::MatchPending(addr) = *status {
                                    if addr == packet.addr() {
                                        // pending match confirmed
                                        *status = Status::MatchConfirmed(packet.addr());
                                    }
                                }
                            }
                            Ok(FromClient::Ping(remote_time)) => {
                                let msg = bincode::serialize(&ToClient::PingResponse(remote_time))
                                    .context(SerializeError)?;
                                packet_sender.send(Packet::unreliable(packet.addr(), msg))?;
                            }
                            Ok(FromClient::PingResponse(past_local_time)) => {
                                let mut peers = peers.lock()?;
                                if let Some(peer) = peers.get_mut(&packet.addr()) {
                                    let local_time = start_time.elapsed().as_nanos();
                                    let latency = (local_time - past_local_time) / 2;
                                    peer.add_ping(latency);
                                }
                            }
                            Err(_) => {}
                        }
                    } else {
                        match bincode::deserialize::<FromServer>(packet.payload()) {
                            Ok(FromServer::Peers(new_peers)) => {
                                let mut peers = peers.lock()?;
                                for peer in new_peers {
                                    peers.insert(peer, Peer::new(peer));
                                }

                                let mut status = status.lock()?;
                                if let Status::QueuePending = *status {
                                    *status = Status::Queued;
                                }
                            }
                            Ok(FromServer::Queued(addr)) => {
                                peers.lock()?.insert(addr, Peer::new(addr));
                            }
                            Ok(FromServer::Dequeued(addr)) => {
                                peers.lock()?.remove(&addr);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(SocketEvent::Connect(addr)) => {
                    if addr == server_addr {
                        *server_connection.lock()? = ServerConnection::Connected;
                    }
                }
                Ok(SocketEvent::Timeout(addr)) => {
                    if addr == server_addr {
                        *server_connection.lock()? = ServerConnection::Disconnected;
                    }
                }
                Err(_) => {}
            }
            match message_receiver.try_recv() {
                Ok(Message::Quit) => return Ok((event_receiver, packet_sender)),
                Err(_) => {}
            }
            if ping_timer.elapsed() > Duration::from_millis(PING_TIMER_MILLIS) {
                for peer in peers.lock()?.values() {
                    let msg = bincode::serialize(&ToClient::Ping(start_time.elapsed().as_nanos()))
                        .context(SerializeError)?;
                    packet_sender.send(Packet::unreliable(peer.addr, msg))?;
                }
                ping_timer = Instant::now();
            }
        }
    }

    pub fn queue(&mut self) -> Result<(), ClientError> {
        let mut status = self.status.lock()?;
        if let Status::Idle = *status {
            let msg = bincode::serialize(&ToServer::Queue).context(SerializeError)?;
            self.packet_sender
                .send(Packet::reliable_unordered(self.server_addr, msg))?;
            let mut server_connection = self.server_connection.lock()?;
            if let ServerConnection::Disconnected = *server_connection {
                *server_connection = ServerConnection::Connecting;
            }
            *status = Status::QueuePending;
        }
        Ok(())
    }

    pub fn dequeue(&self) -> Result<(), ClientError> {
        let mut status = self.status.lock()?;
        if let Status::QueuePending | Status::Queued = *status {
            let msg = bincode::serialize(&ToServer::Dequeue).context(SerializeError)?;
            self.packet_sender
                .send(Packet::reliable_unordered(self.server_addr, msg))?;
            *status = Status::Idle;
            *self.server_connection.lock().unwrap() = ServerConnection::Disconnected;
        }
        Ok(())
    }

    pub fn challenge(&self, peer: &mut Peer) -> Result<(), ClientError> {
        let msg = bincode::serialize(&ToClient::Challenge).context(SerializeError)?;
        self.packet_sender
            .send(Packet::reliable_unordered(peer.addr, msg))?;
        peer.status = PeerStatus::OutgoingChallenge;
        self.outgoing_challenges.lock().unwrap().insert(peer.addr);
        Ok(())
    }

    pub fn accept(&self, peer: &mut Peer) -> Result<(), ClientError> {
        if self.incoming_challenges.lock()?.contains(&peer.addr) {
            let msg = bincode::serialize(&ToClient::Accept).context(SerializeError)?;
            self.packet_sender
                .send(Packet::reliable_unordered(peer.addr, msg))?;
        }
        Ok(())
    }

    pub fn decline(&self, addr: SocketAddr) -> Result<(), ClientError> {
        if self.incoming_challenges.lock().unwrap().remove(&addr) {
            let msg = bincode::serialize(&ToClient::Decline).unwrap();
            self.packet_sender
                .send(Packet::reliable_unordered(addr, msg))?;
        }
        Ok(())
    }

    pub fn close(self) -> Result<(Receiver<SocketEvent>, Sender<Packet>), ClientError> {
        self.message_sender.send(Message::Quit).unwrap();
        self.handle.join().unwrap()
    }

    pub fn peers(&self) -> Result<HashSet<Peer>, ClientError> {
        Ok(self.peers.lock()?.values().cloned().collect())
    }

    pub fn incoming_challenges(&self) -> Result<HashSet<SocketAddr>, ClientError> {
        Ok(self.incoming_challenges.lock()?.clone())
    }

    pub fn outgoing_challenges(&self) -> Result<HashSet<SocketAddr>, ClientError> {
        Ok(self.outgoing_challenges.lock()?.clone())
    }

    pub fn check_match(&self) -> Result<Option<SocketAddr>, ClientError> {
        if let Status::MatchConfirmed(peer) = *self.status.lock()? {
            Ok(Some(peer))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Snafu)]
pub enum CreateError {
    BindError { source: laminar::ErrorKind },
}

#[derive(Debug, Snafu)]
pub enum ClientError {
    MutexError,
    SenderError,
    SerializeError { source: Box<bincode::ErrorKind> },
}

impl<T> From<PoisonError<T>> for ClientError {
    fn from(_: PoisonError<T>) -> Self {
        ClientError::MutexError
    }
}

impl<T> From<SendError<T>> for ClientError {
    fn from(_: SendError<T>) -> Self {
        ClientError::SenderError
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn sample_test() {
        let ip1 = "127.0.0.1".parse().unwrap();
        let ip2 = "127.0.0.2".parse().unwrap();
        let addr1 = SocketAddr::new(ip1, CLIENT_PORT);
        let addr2 = SocketAddr::new(ip2, CLIENT_PORT);
        let mut client1 = Client::new(ip1, ip1).unwrap();
        let mut client2 = Client::new(ip2, ip1).unwrap();

        let mut server = Socket::bind((ip1, SERVER_PORT)).unwrap();

        client1.queue();
        client2.queue();

        thread::sleep(Duration::from_millis(100));
        server.manual_poll(Instant::now());
        while let Some(event) = server.recv() {
            if let SocketEvent::Packet(packet) = event {
                if packet.addr() == addr1 {
                    let mut peers = HashSet::new();
                    peers.insert(addr2);
                    let payload = bincode::serialize(&FromServer::Peers(peers)).unwrap();
                    let response = Packet::reliable_unordered(packet.addr(), payload);
                    server.send(response).unwrap();
                    server.manual_poll(Instant::now());
                } else {
                    let mut peers = HashSet::new();
                    peers.insert(addr1);
                    let payload = bincode::serialize(&FromServer::Peers(peers)).unwrap();
                    let response = Packet::reliable_unordered(packet.addr(), payload);
                    server.send(response).unwrap();
                    server.manual_poll(Instant::now());
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
        for mut peer in client1.peers().unwrap() {
            client1.challenge(&mut peer).unwrap();
        }
        for mut peer in client2.peers().unwrap() {
            client2.challenge(&mut peer).unwrap();
        }

        thread::sleep(Duration::from_millis(400));
        println!("{:?}", client1.status.lock());
        println!("{:?}", client2.status.lock());
        unimplemented!();
    }
}
