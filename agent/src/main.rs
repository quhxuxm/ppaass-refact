use std::time::Duration;

use tokio::net::TcpListener;
use tokio_tower::multiplex::Server;
use tower::timeout::Timeout;
use tracing::error;

pub(crate) mod protocol {
    pub(crate) mod http;
    pub(crate) mod socks;
}

pub(crate) mod codec {
    pub(crate) mod socks;
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:10080").await.unwrap();
    loop {
        match listener.accept().await {
            Err(e) => {
                error!(
                    "Fail to accept client connection because of error: {:#?}",
                    e
                );
            }
            Ok((client_stream, client_address)) => {
                let server = Server::new(client_stream, Timeout::new(Duration::from_secs(10)));
            }
        }
    }
}
