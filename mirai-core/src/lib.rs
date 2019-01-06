#[macro_use]
extern crate serde_derive;
extern crate bincode;

pub mod player {
    use bincode::serialize;
    use std::fmt::{Display, Formatter, Result};
    use std::net::SocketAddr;

    #[derive(Serialize, Deserialize, PartialEq, Eq, Hash)]
    pub struct Player {
        addr: SocketAddr,
    }

    impl Player {
        pub fn new(addr: SocketAddr) -> Player {
            Player { addr }
        }

        pub fn addr(&self) -> SocketAddr {
            self.addr
        }

        pub fn serialize(&self) -> [u8; 8] {
            let mut ser = [0; 8];
            for (i,u) in serialize(&self).unwrap().iter().enumerate() {
                ser[i] = *u;
            }
            ser
        }
    }

    impl Display for Player {
        fn fmt(&self, f: &mut Formatter) -> Result {
            self.addr.fmt(f)
        }
    }
}
