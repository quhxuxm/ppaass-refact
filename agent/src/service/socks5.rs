use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use tokio::{net::TcpStream, sync::Mutex};

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
    init::{Socks5InitFlowResult, Socks5UdpRelayFlow, Socks5UdpRelayFlowRequest, Socks5UdpRelayFlowResult},
};

use super::common::{ProxyConnectionPool, TcpRelayFlowResult};

mod auth;
mod init;

pub(crate) struct Socks5FlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

pub(crate) struct Socks5FlowResult;

pub(crate) struct Socks5FlowProcessor;
impl Socks5FlowProcessor {
    pub async fn exec<T>(
        request: Socks5FlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<Mutex<ProxyConnectionPool>>,
    ) -> Result<Socks5FlowResult>
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
            connection_id: connection_id.clone(),
            client_stream,
            client_address,
            buffer,
        })
        .await?;
        info!("Connection [{}] success to do authenticate, begin to init.", connection_id);
        let init_flow_result = Socks5InitFlow::exec(
            Socks5InitFlowRequest {
                connection_id: connection_id.clone(),
                proxy_addresses,
                client_stream,
                client_address,
                buffer,
            },
            rsa_crypto_fetcher.clone(),
            configuration.clone(),
            proxy_connection_pool,
        )
        .await?;
        match init_flow_result {
            Socks5InitFlowResult::Tcp {
                client_stream,
                message_framed_read,
                message_framed_write,
                client_address,
                source_address,
                target_address,
                proxy_address,
            } => {
                let TcpRelayFlowResult { client_address } = TcpRelayFlow::exec(
                    TcpRelayFlowRequest {
                        connection_id: connection_id.clone(),
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
                info!("Connection [{}] start socks5 tcp relay for client: {:?}", connection_id, client_address);
                Ok(Socks5FlowResult)
            },
            Socks5InitFlowResult::Udp {
                associated_udp_socket,
                associated_udp_address,
                client_stream,
                message_framed_read,
                message_framed_write,
                client_address,
                proxy_address,
            } => {
                let udp_address = associated_udp_socket.local_addr()?;
                let target_address = common::NetAddress::IpV4([0, 0, 0, 0], 0);
                let Socks5UdpRelayFlowResult { .. } = Socks5UdpRelayFlow::exec(
                    Socks5UdpRelayFlowRequest {
                        associated_udp_socket,
                        associated_udp_address,
                        connection_id: connection_id.clone(),
                        client_address: client_address.clone(),
                        client_stream,
                        message_framed_write,
                        message_framed_read,
                        target_address,
                        init_data: None,
                        proxy_address,
                    },
                    configuration,
                )
                .await?;
                info!(
                    "Connection [{}] complete socks5 udp relay for client: {:?} on udp address: {:?}",
                    connection_id, client_address, udp_address
                );
                Ok(Socks5FlowResult)
            },
        }
    }
}
