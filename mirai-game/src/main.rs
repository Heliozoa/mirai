mod game;
mod inputs;

use game::Game;
use ggez::event::KeyCode;
use ggez::*;
use inputs::*;
use mirai_matchmaking_client::Client;
use std::env;
use std::io::Result;
use std::net::SocketAddr;

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
        let challenge;
        let mut s = String::new();
        println!("input 1 for challenge, 2 for wait");
        std::io::stdin().read_line(&mut s).unwrap();
        match s.trim().parse() {
            Ok(1) => challenge = true,
            Ok(2) => challenge = false,
            _ => panic!("asd"),
        }
        println!("{}", challenge);

        let client = Client::new(LOCAL_IP.parse().unwrap(), server_ip.parse().unwrap());
        // matchmaking
        client.dequeue();
        client.queue();
        'ret: loop {
            if challenge {
                let matches = client.check_matches();
                for (k, v) in matches {
                    if let Some(l) = v {
                        println!("challenging");
                        client.challenge(k);
                        break 'ret;
                    }
                }
            } else {
                match client.check_challenge_status() {
                    ChallengeStatus::Incoming(_) => {
                        println!("accepting");
                        client.accept_challenge();
                        break 'ret;
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let opp_addr = loop {
            match client.check_challenge_status() {
                ChallengeStatus::Confirmed(opp) => {
                    println!("confirmed");
                    client.dequeue();
                    break opp;
                }
                _ => {}
            }
        };
        while let Some(_) = client.check_time_until_start() {}
        let (sender, receiver) = client.to_sender_receiver();
        p2_input = InputSourceKind::remote(opp_addr, sender, receiver);
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
