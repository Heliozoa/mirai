use bincode;
use crossbeam_channel::{Receiver as CrossReceiver, Sender as CrossSender};
use ggez::{event::KeyCode, input, Context};
use laminar::{Packet, SocketEvent};
use mirai_game_client::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    iter::DoubleEndedIterator,
    net::SocketAddr,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

use InputSourceKind::*;

// top level abstraction for dealing with inputs
pub struct InputSource {
    p1: InputSourceKind,
    p2: InputSourceKind,
}

impl InputSource {
    pub fn new(p1: InputSourceKind, p2: InputSourceKind) -> Self {
        Self { p1, p2 }
    }

    // returns the latest fully confirmed frame = the largest frame f where 0..=f are all confirmed
    pub fn latest_fully_confirmed(&self) -> u32 {
        std::cmp::min(
            self.p1.latest_fully_confirmed(),
            self.p2.latest_fully_confirmed(),
        )
    }

    // locks the current local inputs for the target frame and sends them to each remote source
    pub fn progress_frame(&mut self, ctx: &mut Context, frame: u32) {
        if let Local(local_source) = &mut self.p1 {
            local_source.progress_frame(ctx);
            if let Remote(remote_source) = &self.p2 {
                let lower_bound = if frame > 8 { frame as usize - 7 } else { 0 };
                let mut inputs = local_source.inputs[lower_bound..=frame as usize].to_vec();
                inputs.reverse();
                remote_source.send(frame, inputs);
            }
        }
        if let Local(local_source) = &mut self.p2 {
            local_source.progress_frame(ctx);
            if let Remote(remote_source) = &self.p1 {
                let lower_bound = if frame > 8 { frame as usize - 8 } else { 0 };
                let mut inputs = local_source.inputs[lower_bound..=frame as usize].to_vec();
                inputs.reverse();
                remote_source.send(frame, inputs);
            }
        }
    }

    // fetch input for the given frame for p1
    pub fn p1_input_for(&self, frame: u32) -> Input {
        match &self.p1 {
            Local(is) => is.input_for(frame),
            Remote(is) => is.input_for(frame),
        }
    }

    // fetch input for the given frame for p2
    pub fn p2_input_for(&self, frame: u32) -> Input {
        match &self.p2 {
            Local(is) => is.input_for(frame),
            Remote(is) => is.input_for(frame),
        }
    }
}

// differentiation between local and remote input sources, i.e. keyboard vs socket
pub enum InputSourceKind {
    Local(LocalInputSource),
    Remote(RemoteInputSource),
}

impl InputSourceKind {
    pub fn local(left_keycode: KeyCode, right_keycode: KeyCode, attack_keycode: KeyCode) -> Self {
        Self::Local(LocalInputSource::new(
            left_keycode,
            right_keycode,
            attack_keycode,
        ))
    }

    pub fn remote(client: Client) -> Self {
        Self::Remote(RemoteInputSource::new(client))
    }

    pub fn latest_fully_confirmed(&self) -> u32 {
        match self {
            Local(is) => is.target_frame,
            Remote(is) => is.latest_fully_confirmed(),
        }
    }
}

struct LocalInputSource {
    left_keycode: KeyCode,
    right_keycode: KeyCode,
    attack_keycode: KeyCode,
    inputs: Vec<Input>,
    target_frame: u32,
}

impl LocalInputSource {
    fn new(left_keycode: KeyCode, right_keycode: KeyCode, attack_keycode: KeyCode) -> Self {
        let inputs = vec![Input::default()];
        LocalInputSource {
            left_keycode,
            right_keycode,
            attack_keycode,
            inputs,
            target_frame: 0,
        }
    }
    fn input_for(&self, frame: u32) -> Input {
        self.inputs[frame as usize]
    }
    // progress target_frame, set the current input for that frame
    fn progress_frame(&mut self, ctx: &mut Context) {
        self.target_frame += 1;
        let current_input = Input {
            left: input::keyboard::is_key_pressed(ctx, self.left_keycode),
            right: input::keyboard::is_key_pressed(ctx, self.right_keycode),
            attack: input::keyboard::is_key_pressed(ctx, self.attack_keycode),
        };
        self.inputs.push(current_input);
    }
}

enum Message {
    Inputs(u32, Vec<Input>),
}

struct RemoteInputSource {
    client: Client,
    sender: Sender<Message>,
    inputs: Arc<Mutex<BTreeMap<u32, Input>>>,
    latest_fully_confirmed: Arc<Mutex<u32>>,
}

impl RemoteInputSource {
    fn new(client: Client) -> Self {
        // start thread
        let mut inputs = BTreeMap::new();
        inputs.insert(0, Input::default());
        let inputs = Arc::new(Mutex::new(inputs));
        let thread_inputs = Arc::clone(&inputs);
        let latest_fully_confirmed = Arc::new(Mutex::new(0));
        let thread_latest_fully_confirmed = Arc::clone(&latest_fully_confirmed);
        let (sender, receiver) = channel();
        Self {
            client,
            sender,
            inputs,
            latest_fully_confirmed,
        }
    }

    // fetch input for the given frame, if not available then fetch latest input we have before that frame
    fn input_for(&self, frame: u32) -> Input {
        let inputs = self.inputs.lock().expect("failed to get lock for inputs");
        if let Some(&input) = inputs.get(&frame) {
            input
        } else {
            *inputs.range(0..frame).next_back().expect("empty range").1
        }
    }

    fn send(&self, frame: u32, inputs: Vec<Input>) {
        self.sender
            .send(Message::Inputs(frame, inputs))
            .expect("failed to send inputs");
    }

    pub fn latest_fully_confirmed(&self) -> u32 {
        *self
            .latest_fully_confirmed
            .lock()
            .expect("failed to get lock for confirm")
    }
}

#[derive(Serialize, Deserialize)]
struct NetworkInput {
    frame: u32,
    inputs: Vec<Input>, // stored in reverse order: [input for frame, input for frame - 1, ...]
}

#[derive(Copy, Clone, Default, Serialize, Deserialize)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub attack: bool,
}
