use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::Result;
use common::{MessageFramedRead, MessageFramedWrite, NetAddress, RsaCryptoFetcher, TcpConnectRequest, TcpConnectResult, TcpConnector};
use tokio::net::UdpSocket;

use crate::service::common::DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS;
use crate::{config::AgentConfig, message::socks5::Socks5Addr};
pub(crate) struct Socks5UdpAssociateFlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
    pub dest_address: Socks5Addr,
}
pub(crate) struct Socks5UdpAssociateFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub associated_udp_socket: UdpSocket,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub proxy_address: Option<SocketAddr>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}
pub(crate) struct Socks5UdpAssociateFlow;

impl Socks5UdpAssociateFlow {
    pub(crate) async fn exec<T>(
        request: Socks5UdpAssociateFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>,
    ) -> Result<Socks5UdpAssociateFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let Socks5UdpAssociateFlowRequest { proxy_addresses, .. } = request;
        let target_stream_so_linger = configuration.proxy_stream_so_linger().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS);
        let initial_local_ip = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
        let initial_udp_address = SocketAddr::new(initial_local_ip, 0);
        let associated_udp_socket = UdpSocket::bind(initial_udp_address).await?;
        let associated_udp_address = associated_udp_socket.local_addr()?;
        let associated_udp_port = associated_udp_address.port();

        let TcpConnectResult {
            connected_stream: proxy_stream,
        } = TcpConnector::connect(TcpConnectRequest {
            connect_addresses: proxy_addresses.to_vec(),
            connected_stream_so_linger: target_stream_so_linger,
        })
        .await?;
        todo!()
    }
}
