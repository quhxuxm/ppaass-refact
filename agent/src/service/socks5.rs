use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use bytes::BytesMut;
use tokio::net::TcpStream;
use tracing::{debug, instrument};

use common::RsaCryptoFetcher;

use crate::service::socks5::auth::{Socks5AuthenticateFlow, Socks5AuthenticateFlowRequest};
use crate::service::socks5::init::{Socks5InitFlow, Socks5InitFlowRequest};
use crate::{
    config::AgentConfig,
    service::common::{TcpRelayFlow, TcpRelayFlowRequest},
};

use super::{common::TcpRelayFlowResult, pool::ProxyConnectionPool};

use self::{
    auth::Socks5AuthenticateFlowResult,
    init::{Socks5InitFlowResult, Socks5UdpRelayFlow, Socks5UdpRelayFlowRequest, Socks5UdpRelayFlowResult},
};

mod auth;
mod init;

#[derive(Debug)]
pub(crate) struct Socks5FlowRequest {
    pub client_connection_id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

pub(crate) struct Socks5FlowResult;

pub(crate) struct Socks5FlowProcessor;
impl Socks5FlowProcessor {
    #[instrument(skip_all, fields(request.client_connection_id))]
    pub async fn exec<T>(
        request: Socks5FlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<ProxyConnectionPool>,
    ) -> Result<Socks5FlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let Socks5FlowRequest {
            client_connection_id,
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
            client_connection_id: client_connection_id.as_str(),
            client_stream,
            client_address,
            buffer,
        })
        .await?;
        debug!("Client connection [{}] success to do authenticate, begin to init.", client_connection_id);
        let init_flow_result = Socks5InitFlow::exec(
            Socks5InitFlowRequest {
                client_connection_id: client_connection_id.as_str(),
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
                proxy_connection_id,
            } => {
                let TcpRelayFlowResult { client_address } = TcpRelayFlow::exec(
                    TcpRelayFlowRequest {
                        client_connection_id: client_connection_id.as_str(),
                        proxy_connection_id: proxy_connection_id.as_str(),
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
                debug!(
                    "Client connection [{}] start socks5 tcp relay for client: {:?}",
                    client_connection_id, client_address
                );
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
                proxy_connection_id,
            } => {
                let udp_address = associated_udp_socket.local_addr()?;
                let target_address = common::NetAddress::IpV4([0, 0, 0, 0], 0);
                let Socks5UdpRelayFlowResult { .. } = Socks5UdpRelayFlow::exec(
                    Socks5UdpRelayFlowRequest {
                        associated_udp_socket,
                        associated_udp_address,
                        client_connection_id: client_connection_id.as_str(),
                        proxy_connection_id: proxy_connection_id.as_str(),
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
                debug!(
                    "Client connection [{}] complete socks5 udp relay for client: {:?} on udp address: {:?}",
                    client_connection_id, client_address, udp_address
                );
                Ok(Socks5FlowResult)
            },
        }
    }
}
