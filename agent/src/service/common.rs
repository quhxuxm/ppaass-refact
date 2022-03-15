use std::task::{Context, Poll};
use std::{future::Future, pin::Pin};
use std::{net::SocketAddr, process::Output};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::Service;
use tracing::error;

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
    // type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ClientConnection) -> Self::Future {
        let ClientConnection {
            stream,
            client_address,
        } = req;
        Box::pin(async move {
            let mut protocol_buf: [u8; 1] = [0];
            let peek_result = stream.peek(&mut protocol_buf).await;
            match peek_result {
                Err(e) => {
                    error!(
                        "Fail to peek protocol from client stream because of error: {:#?}",
                        e
                    );
                    return Err(e);
                }
                Ok(1) => {}
                Ok(_) => {}
            }
            Ok(())
        })
    }
}
