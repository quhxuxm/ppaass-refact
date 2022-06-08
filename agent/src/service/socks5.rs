use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures::future::BoxFuture;
use tokio::net::TcpStream;
use tower::Service;
use tower::ServiceBuilder;

use common::{ready_and_call_service, PpaassError, RsaCryptoFetcher};

use crate::service::common::{TcpRelayService, TcpRelayServiceRequest};
use crate::service::socks5::auth::{Socks5AuthCommandService, Socks5AuthenticateFlowRequest};
use crate::service::socks5::init::{Socks5InitCommandService, Socks5InitCommandServiceRequest};

mod auth;
mod init;

pub(crate) struct Socks5FlowRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

impl Debug for Socks5FlowRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Socks5FlowRequest: proxy_addresses={:#?}, client_address={}",
            self.proxy_addresses, self.client_address
        )
    }
}

pub(crate) struct Socks5FlowResult {
    pub client_address: SocketAddr,
}
#[derive(Clone, Debug)]
pub(crate) struct Socks5FlowService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}

impl<T> Socks5FlowService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }
}

impl<T> Service<Socks5FlowRequest> for Socks5FlowService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = Socks5FlowResult;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Socks5FlowRequest) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
        Box::pin(async move {
            let mut authenticate_service =
                ServiceBuilder::new().service(Socks5AuthCommandService::default());
            let mut connect_service = ServiceBuilder::new()
                .service(Socks5InitCommandService::new(rsa_crypto_fetcher.clone()));
            let mut relay_service = ServiceBuilder::new().service(TcpRelayService::new());
            let authenticate_result = ready_and_call_service(
                &mut authenticate_service,
                Socks5AuthenticateFlowRequest {
                    client_stream: req.client_stream,
                    client_address: req.client_address,
                    buffer: req.buffer,
                },
            )
            .await?;
            let connect_flow_result = ready_and_call_service(
                &mut connect_service,
                Socks5InitCommandServiceRequest {
                    proxy_addresses: req.proxy_addresses,
                    client_stream: authenticate_result.client_stream,
                    client_address: authenticate_result.client_address,
                    buffer: authenticate_result.buffer,
                },
            )
            .await?;
            let relay_flow_result = ready_and_call_service(
                &mut relay_service,
                TcpRelayServiceRequest {
                    client_address: connect_flow_result.client_address,
                    client_stream: connect_flow_result.client_stream,
                    message_framed_write: connect_flow_result.message_framed_write,
                    message_framed_read: connect_flow_result.message_framed_read,
                    connect_response_message_id: connect_flow_result.connect_response_message_id,
                    source_address: connect_flow_result.source_address,
                    target_address: connect_flow_result.target_address,
                    init_data: None,
                    proxy_address: connect_flow_result.proxy_address,
                },
            )
            .await?;
            Ok(Socks5FlowResult {
                client_address: relay_flow_result.client_address,
            })
        })
    }
}
