use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::TcpStream;

use anyhow::Result;
use common::RsaCryptoFetcher;
use tracing::info;

use crate::service::socks5::auth::{Socks5AuthenticateFlow, Socks5AuthenticateFlowRequest};
use crate::service::socks5::init::{Socks5InitFlow, Socks5InitFlowRequest};
use crate::{
    config::AgentConfig,
    service::common::{TcpRelayFlow, TcpRelayFlowRequest},
};

use self::{
    auth::Socks5AuthenticateFlowResult,
    init::{Socks5InitFlowResult, Socks5InitFlowResultRelayType, Socks5UdpRelayFlow, Socks5UdpRelayFlowRequest, Socks5UdpRelayFlowResponse},
};

use super::common::TcpRelayFlowResult;

mod auth;
mod init;

pub(crate) struct Socks5FlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

pub(crate) struct Socks5FlowResult {
    pub client_address: SocketAddr,
}

pub(crate) struct Socks5FlowProcessor;
impl Socks5FlowProcessor {
    pub async fn exec<T>(request: Socks5FlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>) -> Result<Socks5FlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let Socks5FlowRequest {
            connection_id,
            proxy_addresses,
            client_stream,
            client_address,
            buffer,
        } = request;
        let Socks5AuthenticateFlowResult {
            client_stream,
            client_address,
            buffer,
            ..
        } = Socks5AuthenticateFlow::exec(Socks5AuthenticateFlowRequest {
            client_stream,
            client_address,
            buffer,
        })
        .await?;
        let Socks5InitFlowResult {
            client_stream,
            message_framed_read,
            message_framed_write,
            client_address,
            source_address,
            target_address,
            proxy_address,
            relay_type,
        } = Socks5InitFlow::exec(
            Socks5InitFlowRequest {
                connection_id: connection_id.clone(),
                proxy_addresses,
                client_stream,
                client_address,
                buffer,
            },
            rsa_crypto_fetcher.clone(),
            configuration.clone(),
        )
        .await?;
        match relay_type {
            Socks5InitFlowResultRelayType::Tcp => {
                let TcpRelayFlowResult { client_address } = TcpRelayFlow::exec(
                    TcpRelayFlowRequest {
                        connection_id,
                        client_address,
                        client_stream,
                        message_framed_write,
                        message_framed_read,
                        source_address,
                        target_address,
                        init_data: None,
                        proxy_address,
                    },
                    configuration,
                )
                .await?;
                info!("Start socks5 tcp relay for client: {:?}", client_address);
                Ok(Socks5FlowResult { client_address })
            },
            Socks5InitFlowResultRelayType::Udp(port) => {
                let Socks5UdpRelayFlowResponse { .. } = Socks5UdpRelayFlow::exec(
                    Socks5UdpRelayFlowRequest {
                        port,
                        connection_id,
                        client_address,
                        client_stream,
                        message_framed_write,
                        message_framed_read,
                        source_address,
                        target_address,
                        init_data: None,
                        proxy_address,
                    },
                    rsa_crypto_fetcher,
                    configuration,
                )
                .await?;
                info!("Start socks5 udp relay for client: {:?}", client_address);
                Ok(Socks5FlowResult { client_address })
            },
        }
    }
}
