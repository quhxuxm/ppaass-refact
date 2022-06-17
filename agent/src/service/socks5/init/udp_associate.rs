use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use common::{MessageFramedRead, MessageFramedWrite, NetAddress, RsaCryptoFetcher};

use crate::config::AgentConfig;
pub struct Socks5UdpAssociateFlowRequest;
pub struct Socks5UdpAssociateFlowResponse<T>
where
    T: RsaCryptoFetcher,
{
    pub port: u16,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub proxy_address: Option<SocketAddr>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}
pub struct Socks5UdpAssociateFlow;

impl Socks5UdpAssociateFlow {
    pub async fn exec<T>(
        request: Socks5UdpAssociateFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>,
    ) -> Result<Socks5UdpAssociateFlowResponse<T>>
    where
        T: RsaCryptoFetcher,
    {
        todo!()
    }
}
