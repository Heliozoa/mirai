mod game;
mod inputs;

use game::Game;
use ggez::event::KeyCode;
use ggez::*;
use inputs::*;
use mirai_game_client::Client as GameClient;
use mirai_matchmaking_client::{Client, PeerStatus};
use std::env;
use std::io::Result;

const LOCAL_IP: &str = "127.0.0.1";

fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    let server_ip = &args[1];
    let p1_input = InputSourceKind::local(KeyCode::A, KeyCode::D, KeyCode::S);
    let p2_input;

    let single_player;
    let mut s = String::new();
    println!("input 1 for single player, 2 for multi");
    std::io::stdin().read_line(&mut s).unwrap();
    match s.trim().parse() {
        Ok(1) => single_player = true,
        Ok(2) => single_player = false,
        _ => panic!("asd"),
    }
    println!("{}", single_player);

    if single_player {
        p2_input = InputSourceKind::local(KeyCode::Left, KeyCode::Right, KeyCode::Down);
    } else {
        // matchmaking

        let mut client =
            Client::new(LOCAL_IP.parse().unwrap(), server_ip.parse().unwrap()).unwrap();
        // matchmaking
        client.dequeue().unwrap();
        client.queue().unwrap();
        let opp = 'ret: loop {
            let matches = client.peers().unwrap();
            for mut peer in matches {
                if let Some(l) = peer.latency() {
                    match peer.status() {
                        PeerStatus::Confirmed => {
                            println!("confirmed");
                            break 'ret peer;
                        }
                        PeerStatus::IncomingChallenge => {
                            println!("accepting");
                            client.accept(&mut peer).unwrap();
                        }
                        PeerStatus::None => {
                            println!("challenging");
                            client.challenge(&mut peer).unwrap();
                        }
                        _ => {}
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        };

        client.dequeue().unwrap();
        let (receiver, sender) = client.close().unwrap();
        let client = GameClient::new(opp.addr(), receiver, sender);
        while let Some(_) = client.check_time_until_start() {}
        p2_input = InputSourceKind::remote(client);
    }

    let input_source = InputSource::new(p1_input, p2_input);
    let mut my_game = Game::new(input_source);

    let (mut ctx, mut event_loop) = ContextBuilder::new("gemu", "Heliozoa")
        .window_mode(conf::WindowMode::default().dimensions(640.0, 480.0))
        .build()
        .unwrap();

    match event::run(&mut ctx, &mut event_loop, &mut my_game) {
        Ok(_) => println!("Exited cleanly."),
        Err(e) => println!("Error occured: {}", e),
    }
    Ok(())
}
