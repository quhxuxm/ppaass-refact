use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;
use tower::ServiceBuilder;

use common::{ready_and_call_service, CommonError};

use crate::service::socks5::init::{Socks5InitCommandService, Socks5InitCommandServiceRequest};
use crate::service::socks5::relay::{Socks5RelayService, Socks5RelayServiceRequest};
use crate::{
    codec::socks5::Socks5InitCodec,
    service::socks5::auth::{Socks5AuthCommandService, Socks5AuthenticateFlowRequest},
};

mod auth;
mod init;
mod relay;

pub(crate) type Socks5Framed<'a> = Framed<&'a mut TcpStream, Socks5InitCodec>;

pub(crate) struct Socks5FlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
pub(crate) struct Socks5FlowResult {
    pub client_address: SocketAddr,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Socks5FlowService;

impl Service<Socks5FlowRequest> for Socks5FlowService {
    type Response = Socks5FlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Socks5FlowRequest) -> Self::Future {
        Box::pin(async move {
            let mut authenticate_service =
                ServiceBuilder::new().service(Socks5AuthCommandService::default());
            let mut connect_service =
                ServiceBuilder::new().service(Socks5InitCommandService::default());
            let mut relay_service = ServiceBuilder::new().service(Socks5RelayService::default());
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
                Socks5InitCommandServiceRequest {
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
