use self::ClientToClient as ToClient;
use self::ClientToClient as FromClient;
use crossbeam_channel::{unbounded, Receiver, Sender};
use laminar::{Packet, Socket, SocketEvent};
use mirai_core::v1::{client::*, CLIENT_PORT, SERVER_PORT};
use serde::{Deserialize, Serialize};
use snafu::{ErrorCompat, ResultExt, Snafu};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
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
    handle: JoinHandle<(Receiver<SocketEvent>, Sender<Packet>)>,
}

impl Client {
    pub fn new(addr: IpAddr, server_ip: IpAddr) -> Self {
        let socket_addr = SocketAddr::new(addr, CLIENT_PORT);
        let server_addr = SocketAddr::new(server_ip, SERVER_PORT);
        let mut socket = Socket::bind(socket_addr).unwrap();
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
        Self {
            status,
            server_addr,
            server_connection,
            message_sender,
            packet_sender,
            peers,
            outgoing_challenges,
            incoming_challenges,
            handle,
        }
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
    ) -> (Receiver<SocketEvent>, Sender<Packet>) {
        let start_time = Instant::now();
        let mut ping_timer = Instant::now() - Duration::from_millis(PING_TIMER_MILLIS);
        loop {
            match event_receiver.try_recv() {
                Ok(SocketEvent::Packet(packet)) => {
                    if packet.addr() != server_addr {
                        match bincode::deserialize::<FromClient>(packet.payload()) {
                            Ok(FromClient::Challenge) => {
                                incoming_challenges.lock().unwrap().insert(packet.addr());
                            }
                            Ok(FromClient::Accept) => {
                                let mut status = status.lock().unwrap();
                                if let Status::Queued = *status {
                                    if outgoing_challenges.lock().unwrap().contains(&packet.addr())
                                    {
                                        let msg = bincode::serialize(&ToClient::Start(0)).unwrap();
                                        packet_sender
                                            .send(Packet::reliable_unordered(packet.addr(), msg))
                                            .unwrap();
                                        *status = Status::MatchPending(packet.addr());
                                    }
                                }
                            }
                            Ok(FromClient::Decline) => {
                                outgoing_challenges.lock().unwrap().remove(&packet.addr());
                                let mut status = status.lock().unwrap();
                                if let Status::MatchPending(addr) = *status {
                                    if addr == packet.addr() {
                                        // got declined by someone we sent Start to
                                        *status = Status::Queued;
                                    }
                                }
                            }
                            Ok(FromClient::Start(time)) => {
                                let mut status = status.lock().unwrap();
                                if let Status::Queued = *status {
                                    // they are match pending
                                    let msg = bincode::serialize(&ToClient::Start(0)).unwrap();
                                    packet_sender
                                        .send(Packet::reliable_unordered(packet.addr(), msg))
                                        .unwrap();
                                    incoming_challenges.lock().unwrap().clear();
                                    outgoing_challenges.lock().unwrap().clear();
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
                                    .unwrap();
                                packet_sender
                                    .send(Packet::unreliable(packet.addr(), msg))
                                    .unwrap();
                            }
                            Ok(FromClient::PingResponse(past_local_time)) => {
                                let mut peers = peers.lock().unwrap();
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
                                let mut peers = peers.lock().unwrap();
                                for peer in new_peers {
                                    peers.insert(peer, Peer::new(peer));
                                }

                                let mut status = status.lock().unwrap();
                                if let Status::QueuePending = *status {
                                    *status = Status::Queued;
                                }
                            }
                            Ok(FromServer::Queued(addr)) => {
                                peers.lock().unwrap().insert(addr, Peer::new(addr));
                            }
                            Ok(FromServer::Dequeued(addr)) => {
                                peers.lock().unwrap().remove(&addr);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(SocketEvent::Connect(addr)) => {
                    if addr == server_addr {
                        *server_connection.lock().unwrap() = ServerConnection::Connected;
                    }
                }
                Ok(SocketEvent::Timeout(addr)) => {
                    if addr == server_addr {
                        *server_connection.lock().unwrap() = ServerConnection::Disconnected;
                    }
                }
                Err(_) => {}
            }
            match message_receiver.try_recv() {
                Ok(Message::Quit) => return (event_receiver, packet_sender),
                Err(_) => {}
            }
            if ping_timer.elapsed() > Duration::from_millis(PING_TIMER_MILLIS) {
                for peer in peers.lock().unwrap().values() {
                    let msg = bincode::serialize(&ToClient::Ping(start_time.elapsed().as_nanos()))
                        .unwrap();
                    packet_sender
                        .send(Packet::unreliable(peer.addr, msg))
                        .unwrap();
                }
                ping_timer = Instant::now();
            }
        }
    }

    pub fn queue(&mut self) {
        let mut status = self.status.lock().unwrap();
        if let Status::Idle = *status {
            let msg = bincode::serialize(&ToServer::Queue).unwrap();
            self.packet_sender
                .send(Packet::reliable_unordered(self.server_addr, msg))
                .unwrap();
            let mut server_connection = self.server_connection.lock().unwrap();
            if let ServerConnection::Disconnected = *server_connection {
                *server_connection = ServerConnection::Connecting;
            }
            *status = Status::QueuePending;
        }
    }

    pub fn dequeue(&self) {
        let mut status = self.status.lock().unwrap();
        if let Status::QueuePending | Status::Queued = *status {
            let msg = bincode::serialize(&ToServer::Dequeue).unwrap();
            self.packet_sender
                .send(Packet::reliable_unordered(self.server_addr, msg))
                .unwrap();
            *status = Status::Idle;
            *self.server_connection.lock().unwrap() = ServerConnection::Disconnected;
        }
    }

    pub fn challenge(&self, peer: &mut Peer) {
        let msg = bincode::serialize(&ToClient::Challenge).unwrap();
        self.packet_sender
            .send(Packet::reliable_unordered(peer.addr, msg))
            .unwrap();
        peer.status = PeerStatus::OutgoingChallenge;
        self.outgoing_challenges.lock().unwrap().insert(peer.addr);
    }

    pub fn accept(&self, peer: &mut Peer) {
        if self
            .incoming_challenges
            .lock()
            .unwrap()
            .contains(&peer.addr)
        {
            let msg = bincode::serialize(&ToClient::Accept).unwrap();
            self.packet_sender
                .send(Packet::reliable_unordered(peer.addr, msg))
                .unwrap();
        }
    }

    pub fn decline(&self, addr: SocketAddr) {
        if self.incoming_challenges.lock().unwrap().remove(&addr) {
            let msg = bincode::serialize(&ToClient::Decline).unwrap();
            self.packet_sender
                .send(Packet::reliable_unordered(addr, msg))
                .unwrap();
        }
    }

    pub fn close(self) -> (Receiver<SocketEvent>, Sender<Packet>) {
        self.message_sender.send(Message::Quit).unwrap();
        self.handle.join().unwrap()
    }

    pub fn peers(&self) -> HashSet<Peer> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    pub fn incoming_challenges(&self) -> HashSet<SocketAddr> {
        self.incoming_challenges.lock().unwrap().clone()
    }

    pub fn outgoing_challenges(&self) -> HashSet<SocketAddr> {
        self.outgoing_challenges.lock().unwrap().clone()
    }

    pub fn check_match(&self) -> Option<SocketAddr> {
        if let Status::MatchConfirmed(peer) = *self.status.lock().unwrap() {
            Some(peer)
        } else {
            None
        }
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
        let mut client1 = Client::new(ip1, ip1);
        let mut client2 = Client::new(ip2, ip1);

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
        for mut peer in client1.peers() {
            client1.challenge(&mut peer);
        }
        for mut peer in client2.peers() {
            client2.challenge(&mut peer);
        }

        thread::sleep(Duration::from_millis(400));
        println!("{:?}", client1.status.lock());
        println!("{:?}", client2.status.lock());
        unimplemented!();
    }
}
