use crossbeam_channel::SendError;
use laminar::{Packet, Socket, SocketEvent};
use mirai_core::v1::{server::*, SERVER_PORT};
use std::{collections::HashSet, env, net::SocketAddr};

fn main() -> Result {
    let args: Vec<_> = env::args().collect();
    let local_ip = &args[1];
    let local_addr = SocketAddr::new(local_ip.parse().unwrap(), SERVER_PORT);
    let socket = Socket::bind(local_addr)?;
    with_socket(socket)
}

fn with_socket(mut socket: Socket) -> Result {
    println!("starting server at {:?}", socket.local_addr().unwrap());
    let packet_sender = socket.get_packet_sender();
    let event_receiver = socket.get_event_receiver();
    let _thread = std::thread::spawn(move || socket.start_polling());
    let mut queue = HashSet::<SocketAddr>::new();
    println!("started server");

    loop {
        match event_receiver.recv() {
            Ok(event) => match event {
                SocketEvent::Packet(packet) => {
                    let source = packet.addr();
                    let payload = packet.payload();
                    // try to deserialize the payload
                    match bincode::deserialize::<FromClient>(payload) {
                        Ok(msg) => match msg {
                            FromClient::StatusCheck => {
                                let msg = bincode::serialize(&ToClient::Alive)?;
                                packet_sender.send(Packet::reliable_unordered(source, msg))?;
                            }
                            FromClient::Queue => {
                                if !queue.contains(&source) {
                                    println!("queuing {}", source);
                                    let msg = bincode::serialize(&ToClient::Peers(queue.clone()))?;
                                    packet_sender.send(Packet::reliable_unordered(source, msg))?;
                                    for &client in &queue {
                                        let msg = bincode::serialize(&ToClient::Queued(source))?;
                                        packet_sender
                                            .send(Packet::reliable_unordered(client, msg))?;
                                    }
                                    queue.insert(source);
                                } else {
                                    let mut queue = queue.clone();
                                    queue.remove(&source);
                                    let msg = bincode::serialize(&ToClient::Peers(queue))?;
                                    packet_sender.send(Packet::reliable_unordered(source, msg))?;
                                }
                            }
                            FromClient::Dequeue => {
                                if queue.remove(&source) {
                                    for &client in &queue {
                                        let msg = bincode::serialize(&ToClient::Dequeued(source))?;
                                        packet_sender
                                            .send(Packet::reliable_unordered(client, msg))?;
                                    }
                                }
                            }
                            FromClient::Heartbeat => { /* heartbeat, ignore */ }
                        },
                        Err(_) => { /* invalid message */ }
                    }
                }
                SocketEvent::Connect(_connect_addr) => {}
                SocketEvent::Timeout(timeout_addr) => {
                    queue.remove(&timeout_addr);
                }
            },
            Err(_) => { /* "something went wrong */ }
        }
    }
}

// error handling types
type Result = std::result::Result<(), ServerError<Packet>>;
type IoError = std::io::Error;
type LaminarError = laminar::ErrorKind;
type BincodeError = std::boxed::Box<bincode::ErrorKind>;

#[derive(Debug)]
enum ServerError<E> {
    Io(IoError),
    Laminar(LaminarError),
    Bincode(BincodeError),
    CrossbeamError(SendError<E>),
}

impl<E> From<IoError> for ServerError<E> {
    fn from(io: IoError) -> ServerError<E> {
        ServerError::Io(io)
    }
}

impl<E> From<LaminarError> for ServerError<E> {
    fn from(laminar: LaminarError) -> ServerError<E> {
        ServerError::Laminar(laminar)
    }
}

impl<E> From<BincodeError> for ServerError<E> {
    fn from(bincode: BincodeError) -> ServerError<E> {
        ServerError::Bincode(bincode)
    }
}

impl<E> From<SendError<E>> for ServerError<E> {
    fn from(crossbeam: SendError<E>) -> ServerError<E> {
        ServerError::CrossbeamError(crossbeam)
    }
}

#[cfg(test)]
#[macro_use]
extern crate lazy_static;
#[cfg(test)]
mod test {
    use super::*;
    use std::time::{Duration, Instant};

    fn start_test_server(socket: Socket) {
        std::thread::spawn(move || with_socket(socket));
    }

    fn wait_for_server(server_addr: SocketAddr) {
        let mut socket = Socket::bind_any().unwrap();
        loop {
            let msg = bincode::serialize(&FromClient::StatusCheck).unwrap();
            socket
                .send(Packet::reliable_unordered(server_addr, msg))
                .unwrap();
            socket.manual_poll(std::time::Instant::now());
            if let Some(event) = socket.recv() {
                match event {
                    SocketEvent::Packet(packet) => {
                        let msg = bincode::deserialize::<ToClient>(packet.payload()).unwrap();
                        assert_eq!(msg, ToClient::Alive);
                        println!("server is alive");
                        break;
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    fn send(socket: &mut Socket, msg: FromClient, server_addr: SocketAddr) {
        let ser = bincode::serialize(&msg).unwrap();
        socket
            .send(Packet::reliable_unordered(server_addr, ser))
            .unwrap();
        socket.manual_poll(std::time::Instant::now());
    }

    fn recv_msg(socket: &mut Socket) -> Option<ToClient> {
        let timer = Duration::from_millis(500);
        let now = Instant::now();
        loop {
            if now.elapsed() > timer {
                return None;
            }
            socket.manual_poll(std::time::Instant::now());
            match socket.recv() {
                Some(event) => match event {
                    SocketEvent::Packet(packet) => {
                        let msg = bincode::deserialize::<ToClient>(packet.payload()).unwrap();
                        return Some(msg);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn expect_msg(socket: &mut Socket, msg: ToClient) -> Option<ToClient> {
        loop {
            let recvd = recv_msg(socket)?;
            if std::mem::discriminant(&msg) == std::mem::discriminant(&recvd) {
                return Some(recvd);
            }
        }
    }

    #[test]
    fn basic_queue_test() {
        let server_socket = Socket::bind_any().unwrap();
        let server_addr = server_socket.local_addr().unwrap();
        start_test_server(server_socket);
        let mut socket_1 = Socket::bind_any().unwrap();
        let mut socket_2 = Socket::bind_any().unwrap();
        let mut socket_3 = Socket::bind_any().unwrap();
        let addr_1 = socket_1.local_addr().unwrap();
        let addr_2 = socket_2.local_addr().unwrap();
        let addr_3 = socket_3.local_addr().unwrap();
        println!("{:?}", addr_1);
        println!("{:?}", addr_2);
        println!("{:?}", addr_3);
        wait_for_server(server_addr);

        // first to queue gets an empty peer set
        send(&mut socket_1, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_1, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peer_list) = peers {
            assert_eq!(peer_list, HashSet::new());
        } else {
            unreachable!()
        }

        // second to queue gets the first peer in a set
        send(&mut socket_2, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_2, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peer_list) = peers {
            let mut expected = HashSet::new();
            expected.insert(addr_1);
            assert_eq!(peer_list, expected);
        } else {
            unreachable!()
        }

        // first peer is notified of second peer
        let queued = expect_msg(&mut socket_1, ToClient::Queued(addr_2)).unwrap();
        if let ToClient::Queued(addr) = queued {
            assert_eq!(addr, addr_2);
        } else {
            unreachable!()
        }

        // third to queue receivers both previous peers in a set
        send(&mut socket_3, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_3, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peer_list) = peers {
            let mut expected = HashSet::new();
            expected.insert(addr_1);
            expected.insert(addr_2);
            assert_eq!(peer_list, expected);
        } else {
            unreachable!()
        }

        // first peer is notified of third
        let queued = expect_msg(&mut socket_1, ToClient::Queued(addr_3)).unwrap();
        if let ToClient::Queued(addr) = queued {
            assert_eq!(addr, addr_3);
        } else {
            unreachable!()
        }

        // second peer is notified of third
        let queued = expect_msg(&mut socket_2, ToClient::Queued(addr_3)).unwrap();
        if let ToClient::Queued(addr) = queued {
            assert_eq!(addr, addr_3);
        } else {
            unreachable!()
        }
    }

    #[test]
    fn basic_dequeue_test() {
        let server_socket = Socket::bind_any().unwrap();
        let server_addr = server_socket.local_addr().unwrap();
        start_test_server(server_socket);
        let mut socket_1 = Socket::bind_any().unwrap();
        let mut socket_2 = Socket::bind_any().unwrap();
        let mut socket_3 = Socket::bind_any().unwrap();
        let addr_2 = socket_2.local_addr().unwrap();
        wait_for_server(server_addr);

        send(&mut socket_1, FromClient::Queue, server_addr);
        send(&mut socket_2, FromClient::Queue, server_addr);
        send(&mut socket_3, FromClient::Queue, server_addr);
        send(&mut socket_2, FromClient::Dequeue, server_addr);

        println!("peer 1 is notified of 2's dequeue");
        let dequeued = expect_msg(&mut socket_1, ToClient::Dequeued(addr_2)).unwrap();
        if let ToClient::Dequeued(addr) = dequeued {
            assert_eq!(addr, addr_2)
        } else {
            unreachable!()
        }
        println!("peer 3 is notified of 2's dequeue");
        let dequeued = expect_msg(&mut socket_3, ToClient::Dequeued(addr_2)).unwrap();
        if let ToClient::Dequeued(addr) = dequeued {
            assert_eq!(addr, addr_2)
        } else {
            unreachable!()
        }
    }

    #[test]
    fn timeout_test() {
        let server_socket = Socket::bind_any().unwrap();
        let server_addr = server_socket.local_addr().unwrap();
        start_test_server(server_socket);
        let mut socket_1 = Socket::bind_any().unwrap();
        let mut socket_2 = Socket::bind_any().unwrap();
        wait_for_server(server_addr);

        send(&mut socket_1, FromClient::Queue, server_addr);
        std::thread::sleep(std::time::Duration::from_secs(6));

        // first client should have timed out of the queue
        send(&mut socket_2, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_2, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peers) = peers {
            assert_eq!(peers, HashSet::new());
        }
    }
}
