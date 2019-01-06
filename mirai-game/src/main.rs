use std::io::Result;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{Duration, SystemTime};

fn main() -> Result<()> {
    const FRAMES_PER_SECOND: u64 = 1;
    const FRAME_DURATION: Duration = Duration::from_nanos(1000000000 / FRAMES_PER_SECOND);
    const LOCAL_ADDRESS: &str = "127.0.0.1:41926";
    let server_ip: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
    let server_socket: SocketAddrV4 = SocketAddrV4::new(server_ip, 41925);

    let mut frame_counter: u32 = 0;
    let mut previous_frame = SystemTime::UNIX_EPOCH;

    let udp = UdpSocket::bind(LOCAL_ADDRESS)?;
    loop {
        //execute every FRAME_DURATION
        let elapsed = loop {
            if let Ok(x) = previous_frame.elapsed() {
                break x;
            }
        };
        if elapsed < FRAME_DURATION {
            continue;
        }
        frame_counter += 1;
        previous_frame = SystemTime::now();

        if frame_counter % 2 == 0 {
            udp.send_to(&[], server_socket)?;
        } else {
            udp.send_to(&[0], server_socket)?;
        }
    }
}
