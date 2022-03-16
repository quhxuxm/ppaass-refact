use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};

use common::CommonError;

use crate::service::socks5::authenticate::{
    Socks5AuthCommandService, Socks5AuthenticateFlowRequest, Socks5AuthenticateFlowResult,
};
use crate::service::socks5::connect::{
    Socks5ConnectCommandService, Socks5ConnectFlowRequest, Socks5ConnectFlowResult,
};
use crate::service::socks5::relay::{
    Socks5RelayFlowRequest, Socks5RelayFlowResult, Socks5RelayService,
};

pub(crate) struct Socks5FlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
pub(crate) struct Socks5FlowResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Socks5FlowService {
    authenticate_service:
        BoxCloneService<Socks5AuthenticateFlowRequest, Socks5AuthenticateFlowResult, CommonError>,
    connect_service:
        BoxCloneService<Socks5ConnectFlowRequest, Socks5ConnectFlowResult, CommonError>,
    relay_service: BoxCloneService<Socks5RelayFlowRequest, Socks5RelayFlowResult, CommonError>,
}

impl Socks5FlowService {
    pub(crate) fn new() -> Self {
        Self {
            authenticate_service: BoxCloneService::new(Socks5AuthCommandService),
            connect_service: BoxCloneService::new(Socks5ConnectCommandService),
            relay_service: BoxCloneService::new(Socks5RelayService),
        }
    }
}

impl Service<Socks5FlowRequest> for Socks5FlowService {
    type Response = Socks5FlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Socks5FlowRequest) -> Self::Future {
        let mut authenticate_service = self.authenticate_service.clone();
        let mut connect_service = self.connect_service.clone();
        let mut relay_service = self.relay_service.clone();
        Box::pin(async move {
            let authenticate_result = authenticate_service
                .ready()
                .await?
                .call(Socks5AuthenticateFlowRequest {
                    client_stream: req.client_stream,
                    client_address: req.client_address,
                })
                .await?;
            if let None = authenticate_result.auth_method {
                return Ok(Socks5FlowResult {
                    client_stream: authenticate_result.client_stream,
                    client_address: authenticate_result.client_address,
                });
            }
            let connect_flow_result = connect_service
                .ready()
                .await?
                .call(Socks5ConnectFlowRequest {
                    client_stream: authenticate_result.client_stream,
                    client_address: authenticate_result.client_address,
                })
                .await?;
            let relay_flow_result = relay_service
                .ready()
                .await?
                .call(Socks5RelayFlowRequest {
                    client_address: connect_flow_result.client_address,
                    client_stream: connect_flow_result.client_stream,
                })
                .await?;
            Ok(Socks5FlowResult {
                client_stream: relay_flow_result.client_stream,
                client_address: relay_flow_result.client_address,
            })
        })
    }
}
