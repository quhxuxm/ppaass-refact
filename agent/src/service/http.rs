use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::Service;

use common::CommonError;

mod connect;
mod relay;

#[derive(Debug)]
pub(crate) struct HttpFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
#[derive(Debug)]
pub(crate) struct HttpFlowResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

#[derive(Clone)]
pub(crate) struct HttpFlowService;

impl HttpFlowService {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Service<HttpFlowRequest> for HttpFlowService {
    type Response = HttpFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: HttpFlowRequest) -> Self::Future {
        todo!()
    }
}
