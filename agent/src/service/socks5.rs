use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::Service;

use common::{ready_and_call_service, CommonError};

use crate::service::socks5::authenticate::{
    Socks5AuthCommandService, Socks5AuthenticateFlowRequest, Socks5AuthenticateFlowResult,
};
use crate::service::socks5::connect::{
    Socks5ConnectCommandService, Socks5ConnectCommandServiceRequest,
    Socks5ConnectCommandServiceResult,
};
use crate::service::socks5::relay::{
    Socks5RelayService, Socks5RelayServiceRequest, Socks5RelayServiceResult,
};

mod authenticate;
mod connect;
mod relay;

pub(crate) struct Socks5FlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
pub(crate) struct Socks5FlowResult {
    pub client_address: SocketAddr,
}
#[derive(Clone, Debug)]
pub(crate) struct Socks5FlowService {
    authenticate_service:
        BoxCloneService<Socks5AuthenticateFlowRequest, Socks5AuthenticateFlowResult, CommonError>,
    connect_service: BoxCloneService<
        Socks5ConnectCommandServiceRequest,
        Socks5ConnectCommandServiceResult,
        CommonError,
    >,
    relay_service:
        BoxCloneService<Socks5RelayServiceRequest, Socks5RelayServiceResult, CommonError>,
}

impl Default for Socks5FlowService {
    fn default() -> Self {
        Self {
            authenticate_service: BoxCloneService::new::<Socks5AuthCommandService>(
                Default::default(),
            ),
            connect_service: BoxCloneService::new::<Socks5ConnectCommandService>(Default::default()),
            relay_service: BoxCloneService::new::<Socks5RelayService>(Default::default()),
        }
    }
}

impl Service<Socks5FlowRequest> for Socks5FlowService {
    type Response = Socks5FlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let authenticate_service_ready = self.authenticate_service.poll_ready(cx);
        let connect_service_ready = self.connect_service.poll_ready(cx);
        let relay_service_ready = self.relay_service.poll_ready(cx);
        if authenticate_service_ready.is_ready()
            && connect_service_ready.is_ready()
            && relay_service_ready.is_ready()
        {
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }

    fn call(&mut self, req: Socks5FlowRequest) -> Self::Future {
        let mut authenticate_service = self.authenticate_service.clone();
        let mut connect_service = self.connect_service.clone();
        let mut relay_service = self.relay_service.clone();
        Box::pin(async move {
            let authenticate_result = ready_and_call_service(
                &mut authenticate_service,
                Socks5AuthenticateFlowRequest {
                    client_stream: req.client_stream,
                    client_address: req.client_address,
                },
            )
            .await?;
            let connect_flow_result = ready_and_call_service(
                &mut connect_service,
                Socks5ConnectCommandServiceRequest {
                    client_stream: authenticate_result.client_stream,
                    client_address: authenticate_result.client_address,
                },
            )
            .await?;
            let relay_flow_result = ready_and_call_service(
                &mut relay_service,
                Socks5RelayServiceRequest {
                    client_address: connect_flow_result.client_address,
                    client_stream: connect_flow_result.client_stream,
                    message_framed_write: connect_flow_result.message_framed_write,
                    message_framed_read: connect_flow_result.message_framed_read,
                    connect_response_message_id: connect_flow_result.connect_response_message_id,
                    proxy_address_string: connect_flow_result.proxy_address_string,
                    source_address: connect_flow_result.source_address,
                    target_address: connect_flow_result.target_address,
                },
            )
            .await?;
            Ok(Socks5FlowResult {
                client_address: relay_flow_result.client_address,
            })
        })
    }
}
