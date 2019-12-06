use crossbeam_channel::{Receiver, Sender};
use laminar::{Packet, SocketEvent};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

enum Message {}

enum Input {}

pub struct Client {
    opp_addr: SocketAddr,
}

impl Client {
    pub fn new(
        opp_addr: SocketAddr,
        receiver: Receiver<SocketEvent>,
        sender: Sender<Packet>,
    ) -> Self {
        Self { opp_addr }
    }

    fn handle_packets(
        opp_addr: SocketAddr,
        packet_sender: Sender<Packet>,
        event_receiver: Receiver<SocketEvent>,
        receiver: Receiver<Message>,
        inputs: Arc<Mutex<BTreeMap<u32, Input>>>,
        latest_fully_confirmed: Arc<Mutex<u32>>,
    ) {
        loop {
            /*
            while let Ok(event) = event_receiver.try_recv() {
                match event {
                    SocketEvent::Packet(packet) => {
                        // try to deserialize incoming packet
                        match bincode::deserialize::<NetworkInput>(&packet.payload()) {
                            Ok(input) => {
                                let mut frame = input.frame;
                                let mut inputs =
                                    inputs.lock().expect("failed to get lock for inputs in evr");
                                println!("received {} inputs for {}", input.inputs.len(), frame);
                                frame += 1;
                                for input in input.inputs {
                                    // reverse inputs, the opponent is playing as p1 but on our side they are p2
                                    let input = Input {
                                        left: input.right,
                                        right: input.left,
                                        ..input
                                    };
                                    frame -= 1;
                                    inputs.insert(frame, input);
                                }
                                // update latest fully confirmed
                                let mut latest_fully_confirmed = latest_fully_confirmed
                                    .lock()
                                    .expect("failed to get lock for confirm in evr");
                                for f in *latest_fully_confirmed + 1.. {
                                    if !inputs.contains_key(&f) {
                                        *latest_fully_confirmed = f - 1;
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            while let Ok(msg) = receiver.try_recv() {
                match msg {
                    Message::Inputs(frame, inputs) => {
                        let msg = bincode::serialize(&NetworkInput { frame, inputs })
                            .expect("failed to serialize ni");
                        packet_sender
                            .send(Packet::unreliable(opp_addr, msg))
                            .expect("failed to send packet");
                    }
                }
            }
            */
        }
    }

    pub fn check_time_until_start(&self) -> Option<u8> {
        None
    }

    pub fn addr(&self) -> SocketAddr {
        self.opp_addr
    }
}
