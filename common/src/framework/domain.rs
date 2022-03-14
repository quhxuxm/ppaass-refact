use std::net::SocketAddr;

use tokio::net::TcpStream;

pub struct Channel {
    pub id: String,
    pub stream: TcpStream,
    pub remote_address: SocketAddr,
    pub local_address: SocketAddr,
}
