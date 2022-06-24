use std::net::SocketAddr;

use std::{io::ErrorKind, sync::Arc};

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;

use futures::SinkExt;

use tracing::{debug, error};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedGenerateResult, MessageFramedGenerator, MessageFramedRead, MessageFramedReader,
    MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult,
    PayloadEncryptionTypeSelector, PayloadType, PpaassError, ProxyMessagePayloadTypeValue, ReadMessageFramedError, ReadMessageFramedRequest,
    ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};

use crate::{
    config::AgentConfig,
    message::socks5::Socks5Addr,
    service::{
        common::DEFAULT_BUFFER_SIZE,
        pool::{ProxyConnection, ProxyConnectionPool},
    },
};

pub(crate) struct Socks5TcpConnectFlow;

pub(crate) struct Socks5TcpConnectFlowRequest {
    pub client_connection_id: String,
    pub client_address: SocketAddr,
    pub dest_address: Socks5Addr,
}

pub(crate) struct Socks5TcpConnectFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub proxy_address: SocketAddr,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub proxy_connection_id: String,
}

impl Socks5TcpConnectFlow {
    pub async fn exec<T>(
        request: Socks5TcpConnectFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<ProxyConnectionPool>,
    ) -> Result<Socks5TcpConnectFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let Socks5TcpConnectFlowRequest {
            client_connection_id,
            dest_address,
            ..
        } = request;
        let client_address = request.client_address;
        let ProxyConnection {
            id: proxy_connection_id,
            stream: proxy_stream,
        } = proxy_connection_pool.fetch_connection().await?;
        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);
        let connected_proxy_address = proxy_stream.peer_addr()?;
        let MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
        } = MessageFramedGenerator::generate(proxy_stream, message_framed_buffer_size, compress, rsa_crypto_fetcher).await;

        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: configuration.user_token().clone().unwrap(),
        })
        .await?;
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            connection_id: Some(proxy_connection_id.clone()),
            message_framed_write,
            payload_encryption_type,
            user_token: configuration.user_token().clone().unwrap(),
            ref_id: None,
            message_payload: Some(MessagePayload {
                source_address: Some(client_address.into()),
                target_address: Some(dest_address.clone().into()),
                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                data: Bytes::new(),
            }),
        })
        .await
        {
            Err(WriteMessageFramedError { source, .. }) => {
                return Err(anyhow!(source));
            },
            Ok(WriteMessageFramedResult { mut message_framed_write }) => {
                if let Err(e) = message_framed_write.flush().await {
                    return Err(anyhow!(e));
                }
                message_framed_write
            },
        };
        match MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: proxy_connection_id.clone(),
            message_framed_read,
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
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                                source_address: Some(source_address),
                                target_address: Some(target_address),
                                ..
                            }),
                        ..
                    }),
            }) => {
                debug!(
                    "Proxy connection [{}] tcp connect process success, response message id: {}, client connection id: {}",
                    proxy_connection_id, message_id, client_connection_id
                );
                Ok(Socks5TcpConnectFlowResult {
                    client_address,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    target_address,
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
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
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
