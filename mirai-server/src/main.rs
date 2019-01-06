use crate::matchmaker::Matchmaker;
use mirai_core::player::Player;
use std::io::Result;
use std::net::UdpSocket;

fn main() -> Result<()> {
    const LOCAL_ADDRESS: &str = "127.0.0.1:41925";

    println!("starting server");
    let udp = UdpSocket::bind(LOCAL_ADDRESS)?;
    println!("started server at {}", LOCAL_ADDRESS);

    println!("starting matchmaker");
    let matchmaker = Matchmaker::new(udp.try_clone().unwrap());
    println!("started matchmaker");

    println!("waiting for requests...");
    loop {
        let mut buf = [0; 32];
        let (amt, src) = udp.recv_from(&mut buf)?;
        println!("received request from {}", src);

        match amt {
            0 => {
                let p = Player::new(src);
                println!("queuing {}", p);
                matchmaker.queue(p);
            }
            1 => {
                let p = Player::new(src);
                println!("dequeuing {}", p);
                matchmaker.dequeue(p);
            }
            _ => println!("invalid request"),
        }
    }
}

mod matchmaker {
    use mirai_core::player::Player;
    use std::collections::HashSet;
    use std::net::UdpSocket;
    use std::ops::Drop;
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::spawn;
    use std::net::SocketAddr;

    enum Command {
        Queue,
        Dequeue,
    }

    use self::Command::Dequeue;
    use self::Command::Queue;

    pub struct Matchmaker {
        queue_sender: Sender<(Player, Command)>,
        shutdown_sender: Sender<()>,
    }

    impl Matchmaker {
        pub fn new(udp: UdpSocket) -> Matchmaker {
            let (queue_sender, queue_receiver) = channel();
            let (shutdown_sender, shutdown_receiver) = channel();
            let players: Arc<Mutex<HashSet<Player>>> = Arc::new(Mutex::new(HashSet::new()));
            let players_clone = players.clone();

            //queuing thread
            spawn(move || {
                while let Ok((player, cmd)) = queue_receiver.recv() {
                    let mut players = players_clone.lock().unwrap();
                    match cmd {
                        Queue => players.insert(player),
                        Dequeue => players.remove(&player),
                    };
                }
            });

            //matching thread
            spawn(move || {
                while let Err(_) = shutdown_receiver.try_recv() {
                    let players = players.lock().unwrap();
                    for player in players.iter() {
                        let players: Vec<u8> = players.iter().flat_map(|p| p.serialize()).collect();
                        udp.send_to(players.as_slice(), player.addr()).unwrap();
                    }
                }
            });

            Matchmaker {
                queue_sender,
                shutdown_sender,
            }
        }

        pub fn queue(&self, p: Player) {
            self.queue_sender.send((p, Queue)).unwrap();
        }

        pub fn dequeue(&self, p: Player) {
            self.queue_sender.send((p, Dequeue)).unwrap();
        }
    }

    impl Drop for Matchmaker {
        fn drop(&mut self) {
            self.shutdown_sender.send(()).unwrap();
        }
    }
}
