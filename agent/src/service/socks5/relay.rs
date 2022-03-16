use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::Service;

use common::CommonError;

pub(crate) struct Socks5RelayFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

pub(crate) struct Socks5RelayFlowResult {
    pub client_stream: TcpStream,
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
        todo!()
    }
}
