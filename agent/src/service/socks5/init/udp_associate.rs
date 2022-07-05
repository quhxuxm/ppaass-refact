use std::{
    fmt::Debug,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::anyhow;
use anyhow::Result;

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedGenerateResult, MessageFramedGenerator, MessageFramedRead, MessageFramedReader,
    MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult,
    PayloadEncryptionTypeSelector, PayloadType, PpaassError, ProxyMessagePayloadTypeValue, ReadMessageFramedError, ReadMessageFramedRequest,
    ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};

use tokio::net::UdpSocket;
use tracing::{debug, error, instrument};

use crate::service::{
    common::DEFAULT_BUFFER_SIZE,
    pool::{ProxyConnection, ProxyConnectionPool},
};
use crate::{config::AgentConfig, message::socks5::Socks5Addr};

#[derive(Debug)]
pub(crate) struct Socks5UdpAssociateFlowRequest<'a> {
    pub client_connection_id: &'a str,
    pub client_address: Socks5Addr,
}

#[derive(Debug)]
pub(crate) struct Socks5UdpAssociateFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub associated_udp_socket: UdpSocket,
    pub associated_udp_address: Socks5Addr,
    pub message_framed_read: MessageFramedRead<T, ProxyConnection>,
    pub message_framed_write: MessageFramedWrite<T, ProxyConnection>,
    pub proxy_address: SocketAddr,
    pub client_address: NetAddress,
    pub proxy_connection_id: String,
}
pub(crate) struct Socks5UdpAssociateFlow;

impl Socks5UdpAssociateFlow {
    #[instrument(skip_all, fields(request.client_connection_id))]
    pub(crate) async fn exec<'a, T>(
        request: Socks5UdpAssociateFlowRequest<'a>, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>,
        proxy_connection_pool: Arc<ProxyConnectionPool>,
    ) -> Result<Socks5UdpAssociateFlowResult<T>>
    where
        T: RsaCryptoFetcher + Debug,
    {
        let Socks5UdpAssociateFlowRequest {
            client_address,
            client_connection_id,
        } = request;
        let initial_local_ip = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
        let initial_udp_address = SocketAddr::new(initial_local_ip, 0);
        let associated_udp_socket = UdpSocket::bind(initial_udp_address).await?;
        let associated_udp_address = associated_udp_socket.local_addr()?;
        let proxy_connection = proxy_connection_pool.fetch_connection().await?;
        let proxy_connection_id = proxy_connection.id.clone();
        let connected_proxy_address = proxy_connection.peer_addr()?;
        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);

        let MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
        } = MessageFramedGenerator::generate(proxy_connection, message_framed_buffer_size, compress, rsa_crypto_fetcher).await;

        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
        })
        .await?;
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            connection_id: Some(proxy_connection_id.as_str()),
            message_framed_write,
            payload_encryption_type,
            user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
            ref_id: None,
            message_payloads: Some(vec![MessagePayload {
                // The source address is the udp address that the
                // client side receiving the udp packets
                source_address: Some(client_address.clone().into()),
                target_address: None,
                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate),
                data: None,
            }]),
        })
        .await
        {
            Err(WriteMessageFramedError { source, .. }) => {
                return Err(anyhow!(source));
            },
            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
        };

        match MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: proxy_connection_id.as_str(),
            message_framed_read,
            timeout: None,
        })
        .await
        {
            Err(ReadMessageFramedError { source, .. }) => Err(anyhow!(source)),
            Ok(ReadMessageFramedResult { content: None, .. }) => Err(anyhow!(PpaassError::CodecError)),
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpAssociateSuccess),
                                source_address: Some(client_address),
                                ..
                            }),
                        ..
                    }),
            }) => {
                debug!(
                    "Proxy connection [{}] udp associate process success, response message id: {}, client connection id: {}",
                    proxy_connection_id, message_id, client_connection_id
                );
                Ok(Socks5UdpAssociateFlowResult {
                    associated_udp_socket,
                    associated_udp_address: associated_udp_address.into(),
                    client_address,
                    message_framed_read,
                    message_framed_write,
                    proxy_address: connected_proxy_address,
                    proxy_connection_id,
                })
            },
            Ok(ReadMessageFramedResult {
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpAssociateFail),
                                ..
                            }),
                        ..
                    }),
                ..
            }) => {
                error!(
                    "Proxy connection [{}] fail connect to target from proxy, response message id: {}, client connection id: {}",
                    proxy_connection_id, message_id, client_connection_id
                );
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(
                        ErrorKind::ConnectionReset,
                        format!("Proxy connection [{}] fail connect to target from proxy", proxy_connection_id)
                    ),
                }))
            },
            Ok(ReadMessageFramedResult {
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        message_payload: Some(MessagePayload { payload_type, .. }),
                        ..
                    }),
                ..
            }) => {
                error!(
                    "Proxy connection [{}] fail connect to target from proxy because of invalid payload type:{:?}, response message id: {}, client connection id: {}",
                    proxy_connection_id, payload_type, message_id, client_connection_id
                );
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                }))
            },
            Ok(ReadMessageFramedResult { .. }) => {
                error!(
                    "Proxy connection [{}] fail connect to target from proxy because of unknown response content.",
                    proxy_connection_id
                );
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid response content read from proxy.",),
                }))
            },
        }
    }
}
