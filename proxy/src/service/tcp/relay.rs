use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::Service;

use common::CommonError;

use crate::service::tcp::close::{TcpCloseService, TcpCloseServiceRequest, TcpCloseServiceResult};

pub(crate) struct TcpRelayServiceRequest {
    pub agent_stream: TcpStream,
    pub agent_address: SocketAddr,
    pub target_stream: TcpStream,
}
pub(crate) struct TcpRelayServiceResult {
    pub agent_stream: TcpStream,
    pub agent_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct TcpRelayService {
    tcp_close_service: BoxCloneService<TcpCloseServiceRequest, TcpCloseServiceResult, CommonError>,
}

impl TcpRelayService {
    pub(crate) fn new() -> Self {
        Self {
            tcp_close_service: BoxCloneService::new(TcpCloseService),
        }
    }
}

impl Service<TcpRelayServiceRequest> for TcpRelayService {
    type Response = TcpRelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpRelayServiceRequest) -> Self::Future {
        Box::pin(async move {
            Ok(TcpRelayServiceResult {
                agent_address: req.agent_address,
                agent_stream: req.agent_stream,
            })
        })
    }
}
