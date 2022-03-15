use std::task::{Context, Poll};
use std::{future::Future, pin::Pin};
use std::{net::SocketAddr, process::Output};

use tokio::net::TcpStream;
use tower::Service;

use common::CommonError;

pub(crate) struct ClientConnection {
    stream: TcpStream,
    client_address: SocketAddr,
}

impl ClientConnection {
    pub(crate) fn new(stream: TcpStream, client_address: SocketAddr) -> Self {
        Self {
            stream,
            client_address,
        }
    }
}

pub(crate) struct HandleClientConnectionService;

impl Service<ClientConnection> for HandleClientConnectionService {
    type Response = ();
    type Error = CommonError;
    type Future = futures_util::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: ClientConnection) -> Self::Future {
        let ClientConnection {
            stream,
            client_address,
        } = req;
        let mut protocol_buf: [u8; 1] = [0];
        let peek_result = stream.poll_peek(protocol_buf).await;
    }
}
