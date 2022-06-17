use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use common::{MessageFramedRead, MessageFramedWrite, NetAddress, RsaCryptoFetcher};
use tokio::net::TcpStream;

use crate::config::AgentConfig;
pub struct Socks5UdpRelayFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub port: u16,
    pub connection_id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: Option<SocketAddr>,
}
pub struct Socks5UdpRelayFlowResponse {
    port: u16,
}
pub struct Socks5UdpRelayFlow;

impl Socks5UdpRelayFlow {
    pub async fn exec<T>(
        request: Socks5UdpRelayFlowRequest<T>, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>,
    ) -> Result<Socks5UdpRelayFlowResponse>
    where
        T: RsaCryptoFetcher,
    {
        todo!()
    }
}
