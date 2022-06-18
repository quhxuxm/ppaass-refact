use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use common::{MessageFramedRead, MessageFramedWrite, NetAddress, RsaCryptoFetcher};

use tracing::info;

use crate::config::ProxyConfig;

#[allow(unused)]
pub(crate) struct UdpAssociateFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub agent_address: SocketAddr,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
}

#[allow(unused)]
pub(crate) struct UdpAssociateFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
}

pub(crate) struct UdpAssociateFlow;

impl UdpAssociateFlow {
    pub async fn exec<T>(request: UdpAssociateFlowRequest<T>, _configuration: Arc<ProxyConfig>) -> Result<UdpAssociateFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let UdpAssociateFlowRequest {
            connection_id,
            message_id,
            user_token,
            message_framed_read,
            message_framed_write,
            ..
        } = request;
        info!("Connection [{}] associate udp success.", connection_id);
        Ok(UdpAssociateFlowResult {
            connection_id,
            message_framed_read,
            message_framed_write,
            message_id,
            user_token,
        })
    }
}
