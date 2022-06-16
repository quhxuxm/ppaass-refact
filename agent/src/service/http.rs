use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures::future::BoxFuture;
use tokio::net::TcpStream;
use tower::{Service, ServiceBuilder};

use common::{ready_and_call_service, RsaCryptoFetcher};

use crate::service::http::connect::{HttpConnectService, HttpConnectServiceRequest};
use crate::{
    config::AgentConfig,
    service::common::{TcpRelayService, TcpRelayServiceRequest},
};

mod connect;

#[derive(Debug)]
pub(crate) struct HttpFlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
    pub configuration: Arc<AgentConfig>,
}
#[derive(Debug)]
#[allow(unused)]
pub(crate) struct HttpFlowResult {
    pub client_address: SocketAddr,
}

#[derive(Clone, Debug)]
pub(crate) struct HttpFlowService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}

impl<T> HttpFlowService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }
}

impl<T> Service<HttpFlowRequest> for HttpFlowService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = HttpFlowResult;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpFlowRequest) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
        Box::pin(async move {
            let mut connect_service = ServiceBuilder::new().service(HttpConnectService::new(rsa_crypto_fetcher.clone()));
            let mut relay_service = ServiceBuilder::new().service(TcpRelayService::new());
            let connect_result = ready_and_call_service(
                &mut connect_service,
                HttpConnectServiceRequest {
                    connection_id: req.connection_id.clone(),
                    proxy_addresses: req.proxy_addresses,
                    client_address: req.client_address,
                    client_stream: req.client_stream,
                    initial_buf: req.buffer,
                    configuration: req.configuration.clone(),
                },
            )
            .await?;
            let relay_result = ready_and_call_service(
                &mut relay_service,
                TcpRelayServiceRequest {
                    connection_id: req.connection_id,
                    client_address: connect_result.client_address,
                    client_stream: connect_result.client_stream,
                    message_framed_write: connect_result.message_framed_write,
                    message_framed_read: connect_result.message_framed_read,
                    target_address: connect_result.target_address,
                    source_address: connect_result.source_address,
                    init_data: connect_result.init_data,
                    connect_response_message_id: connect_result.message_id,
                    proxy_address: connect_result.proxy_address,
                    configuration: req.configuration,
                },
            )
            .await?;
            Ok(HttpFlowResult {
                client_address: relay_result.client_address,
            })
        })
    }
}
