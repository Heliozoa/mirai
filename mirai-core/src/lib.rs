pub mod v1 {
    // types used by the client and the server
    pub use serde::{Deserialize, Serialize};
    use std::{collections::HashSet, net::SocketAddr};

    pub const SERVER_PORT: u16 = 44444;
    pub const CLIENT_PORT: u16 = 44445;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
    pub enum ClientToServer {
        StatusCheck,
        Queue,
        Dequeue,
        Heartbeat,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
    pub enum ServerToClient {
        Alive,
        Peers(HashSet<SocketAddr>),
        Queued(SocketAddr),
        Dequeued(SocketAddr),
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
    pub enum Input<T> {
        Confirmed(T),
        Unconfirmed(T),
    }

    pub mod client {
        pub use super::ClientToServer as ToServer;
        pub use super::ServerToClient as FromServer;
    }

    pub mod server {
        pub use super::ClientToServer as FromClient;
        pub use super::ServerToClient as ToClient;
    }
}
