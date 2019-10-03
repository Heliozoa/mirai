use crossbeam_channel::{Receiver, Sender};
use laminar::{Packet, SocketEvent};

struct Client {}

impl Client {
    pub fn new(receiver: Receiver<SocketEvent>, sender: Sender<Packet>) -> Self {
        Self {}
    }

    fn handle_packets(receiver: Receiver<SocketEvent>, sender: Sender<Packet>) {
        loop {
            match receiver.try_recv() {
                _ => {}
            }
        }
    }
}
