use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_tower::multiplex::Server;
use tokio_util::codec::Framed;
use tower::timeout::Timeout;
use tower::Service;
use tracing::error;

use crate::codec::socks::{Socks5AuthCodec, Socks5ConnectCodec};
use crate::protocol::socks::command::{Socks5AuthMethod, Socks5AuthRequest, Socks5AuthResponse};
use crate::protocol::socks::service::{Socks5AuthCommandService, Socks5ConnectCommandService};

pub(crate) mod protocol {
    pub(crate) mod http;
    pub(crate) mod socks;
}

pub(crate) mod codec {
    pub(crate) mod socks;
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:10081").await.unwrap();
    loop {
        let (mut client_stream, client_address) = match listener.accept().await {
            Err(e) => {
                error!(
                    "Fail to accept client connection because of error: {:#?}",
                    e
                );
                continue;
            }
            Ok((client_stream, client_address)) => (client_stream, client_address),
        };
        let auth_command_framed = Framed::new(&mut client_stream, Socks5AuthCodec);
        let auth_command_server = Server::new(auth_command_framed, Socks5AuthCommandService);
        auth_command_server.await;

        let connect_command_framed = Framed::new(&mut client_stream, Socks5ConnectCodec);
        let connect_command_server =
            Server::new(connect_command_framed, Socks5ConnectCommandService);
        connect_command_server.await;

        let relay_framed = Framed::new(&mut client_stream, Socks5AuthCodec);
        let relay_server = Server::new(relay_framed, Socks5AuthCommandService);
        relay_server.await;
    }
}
