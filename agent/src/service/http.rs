use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use bytes::BytesMut;
use common::RsaCryptoFetcher;
use tokio::net::TcpStream;

use crate::service::http::connect::{HttpConnectFlow, HttpConnectFlowRequest};
use crate::{
    config::AgentConfig,
    service::common::{TcpRelayFlow, TcpRelayFlowRequest},
};

use self::connect::HttpConnectFlowResult;

use super::{common::TcpRelayFlowResult, pool::ProxyConnectionPool};

mod connect;

#[derive(Debug)]
pub(crate) struct HttpFlowRequest {
    pub connection_id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}
#[derive(Debug)]
#[allow(unused)]
pub(crate) struct HttpFlowResult;

pub(crate) struct HttpFlow;

impl HttpFlow {
    pub async fn exec<T>(
        request: HttpFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<ProxyConnectionPool>,
    ) -> Result<HttpFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let HttpFlowRequest {
            connection_id,
            client_stream,
            client_address,
            buffer,
        } = request;
        let HttpConnectFlowResult {
            client_stream,
            client_address,
            proxy_address,
            init_data,
            message_framed_read,
            message_framed_write,
            source_address,
            target_address,
        } = HttpConnectFlow::exec(
            HttpConnectFlowRequest {
                connection_id: connection_id.clone(),
                client_address,
                client_stream,
                initial_buf: buffer,
            },
            rsa_crypto_fetcher.clone(),
            configuration.clone(),
            proxy_connection_pool,
        )
        .await?;
        let TcpRelayFlowResult { .. } = TcpRelayFlow::exec(
            TcpRelayFlowRequest {
                connection_id,
                client_address,
                client_stream,
                message_framed_write,
                message_framed_read,
                target_address,
                source_address,
                init_data,
                proxy_address,
            },
            configuration,
        )
        .await?;
        Ok(HttpFlowResult)
    }
}
