use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::Service;

use common::CommonError;

pub(crate) struct Socks5RelayFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub proxy_stream: TcpStream,
}

pub(crate) struct Socks5RelayFlowResult {
    pub client_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Socks5RelayService;

impl Service<Socks5RelayFlowRequest> for Socks5RelayService {
    type Response = Socks5RelayFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5RelayFlowRequest) -> Self::Future {
        Box::pin(async move {
            let client_stream = request.client_stream;
            let proxy_stream = request.proxy_stream;
            let (mut client_stream_read_half, mut client_stream_write_half) =
                client_stream.into_split();
            let (mut proxy_stream_read_half, mut proxy_stream_write_half) =
                proxy_stream.into_split();
            tokio::spawn(async move {
                tokio::io::copy(&mut client_stream_read_half, &mut proxy_stream_write_half).await;
            });
            tokio::spawn(async move {
                tokio::io::copy(&mut proxy_stream_read_half, &mut client_stream_write_half).await;
            });
            Ok(Socks5RelayFlowResult {
                client_address: request.client_address,
            })
        })
    }
}
