use crossbeam_channel::SendError;
//use error::{ServerError::*, *};
use laminar::{Packet, Socket, SocketEvent};
use mirai_core::v1::{server::*, SERVER_PORT};
use snafu::{ensure, Backtrace, ErrorCompat, ResultExt, Snafu};
use std::{collections::HashSet, env, net::SocketAddr};

/// The Mirai matchmaking server facilitates peer discovery for Mirai matchmaking clients.
/// The server can receive the following messages:
///     StatusCheck
///         returns Alive to signal that it's running
///     Queue
///         if the client is not already in the queue, adds the client to the queue
///         selects a set of potential matches (currently the entire queue)
///         sends the client's info to all potential matches
///         returns the potential matches to the client
///     Dequeue
///         removes the client from the queue
///     Heartbeat
///         ignored
/// Clients are dequeued when the connection times out.
///
/// Run using cargo run server_ip, e.g. cargo run 127.0.0.1
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e);
        if let Some(backtrace) = ErrorCompat::backtrace(&e) {
            println!("{}", backtrace);
        }
    }
}

fn run() -> Result<(), StartError> {
    let args: Vec<_> = env::args().collect();
    let local_ip = args.get(1).ok_or(StartError::MissingIp)?;
    let local_ip = local_ip.parse().context(InvalidIp { ip: local_ip })?;
    let local_addr = SocketAddr::new(local_ip, SERVER_PORT);
    let socket = Socket::bind(local_addr).context(SocketErr)?;
    with_socket(socket).context(InternalServerError)
}
#[derive(Debug, Snafu)]
pub enum StartError {
    #[snafu(display("missing IP parameter"))]
    MissingIp,
    #[snafu(display("invalid IP '{}': {}", ip, source))]
    InvalidIp {
        ip: String,
        source: std::net::AddrParseError,
    },
    #[snafu(display("binding error: {}", source))]
    SocketErr { source: laminar::ErrorKind },
    #[snafu(display("internal server error: {}", source))]
    InternalServerError { source: ServerError },
}

fn with_socket(mut socket: Socket) -> Result<(), ServerError> {
    println!(
        "starting server at {:?}",
        socket.local_addr().context(SocketError)?
    );
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
                                let msg =
                                    bincode::serialize(&ToClient::Alive).context(SerializeError)?;
                                packet_sender
                                    .send(Packet::reliable_unordered(source, msg))
                                    .context(SenderError)?;
                            }
                            FromClient::Queue => {
                                let mut queue_clone = queue.clone();
                                queue_clone.remove(&source);
                                let msg = bincode::serialize(&ToClient::Peers(queue_clone.clone()))
                                    .context(SerializeError)?;
                                packet_sender
                                    .send(Packet::reliable_unordered(source, msg))
                                    .context(SenderError)?;
                                for &client in &queue_clone {
                                    let msg = bincode::serialize(&ToClient::Queued(source))
                                        .context(SerializeError)?;
                                    packet_sender
                                        .send(Packet::reliable_unordered(client, msg))
                                        .context(SenderError)?;
                                }
                                queue.insert(source);
                            }
                            FromClient::Dequeue => {
                                queue.remove(&source);
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
#[derive(Debug, Snafu)]
pub enum ServerError {
    #[snafu(display("laminar error: {}", source))]
    SocketError { source: laminar::ErrorKind },
    #[snafu(display("error serializing: {}", source))]
    SerializeError {
        source: std::boxed::Box<bincode::ErrorKind>,
    },
    #[snafu(display("error sending: {}", source))]
    SenderError { source: SendError<Packet> },
}

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
        println!("1: {:?}", addr_1);
        println!("2: {:?}", addr_2);
        println!("3: {:?}", addr_3);
        wait_for_server(server_addr);

        send(&mut socket_1, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_1, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peer_list) = peers {
            assert_eq!(
                peer_list,
                HashSet::new(),
                "first to queue gets an empty peer set"
            );
        } else {
            unreachable!("first to queue did not receive peers")
        }

        send(&mut socket_2, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_2, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peer_list) = peers {
            let mut expected = HashSet::new();
            expected.insert(addr_1);
            assert_eq!(
                peer_list, expected,
                "second to queue gets the first peer in a set"
            );
        } else {
            unreachable!("second to queue did not get peers")
        }

        let queued = expect_msg(&mut socket_1, ToClient::Queued(addr_2)).unwrap();
        if let ToClient::Queued(addr) = queued {
            assert_eq!(addr, addr_2, "first peer is notified of second peer");
        } else {
            unreachable!("first peer was not notified")
        }

        send(&mut socket_3, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_3, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peer_list) = peers {
            let mut expected = HashSet::new();
            expected.insert(addr_1);
            expected.insert(addr_2);
            assert_eq!(
                peer_list, expected,
                "third to queue receivers both previous peers in a set"
            );
        } else {
            unreachable!("third to queue did not receive peers")
        }

        let queued = expect_msg(&mut socket_1, ToClient::Queued(addr_3)).unwrap();
        if let ToClient::Queued(addr) = queued {
            assert_eq!(addr, addr_3, "first peer is notified of third");
        } else {
            unreachable!("first peer was not notified")
        }

        let queued = expect_msg(&mut socket_2, ToClient::Queued(addr_3)).unwrap();
        if let ToClient::Queued(addr) = queued {
            assert_eq!(addr, addr_3, "second peer is notified of third");
        } else {
            unreachable!("second peer was not notified")
        }
    }

    #[test]
    fn basic_dequeue_test() {
        let server_socket = Socket::bind_any().unwrap();
        let server_addr = server_socket.local_addr().unwrap();
        start_test_server(server_socket);
        let mut socket_1 = Socket::bind_any().unwrap();
        let mut socket_2 = Socket::bind_any().unwrap();
        wait_for_server(server_addr);

        send(&mut socket_1, FromClient::Queue, server_addr);
        send(&mut socket_1, FromClient::Dequeue, server_addr);
        send(&mut socket_2, FromClient::Queue, server_addr);

        let peers = expect_msg(&mut socket_2, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peers) = peers {
            assert_eq!(
                peers,
                HashSet::new(),
                "second to queue receives empty peer set"
            );
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

        send(&mut socket_2, FromClient::Queue, server_addr);
        let peers = expect_msg(&mut socket_2, ToClient::Peers(HashSet::new())).unwrap();
        if let ToClient::Peers(peers) = peers {
            assert_eq!(
                peers,
                HashSet::new(),
                "first client should have timed out of the queue"
            );
        }
    }
}
