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
    ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, TcpConnectRequest, TcpConnectResult, TcpConnector, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use crate::{
    config::AgentConfig,
    message::socks5::Socks5Addr,
    service::common::{DEFAULT_BUFFER_SIZE, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS},
};

pub(crate) struct Socks5TcpConnectFlow;

pub(crate) struct Socks5TcpConnectFlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
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
}

impl Socks5TcpConnectFlow {
    pub async fn exec<T>(
        request: Socks5TcpConnectFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>,
    ) -> Result<Socks5TcpConnectFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let Socks5TcpConnectFlowRequest {
            connection_id,
            proxy_addresses,
            dest_address,
            ..
        } = request;
        let client_address = request.client_address;
        let connected_stream_so_linger = configuration.proxy_stream_so_linger().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS);

        let TcpConnectResult {
            connected_stream: proxy_stream,
        } = TcpConnector::connect(TcpConnectRequest {
            connect_addresses: proxy_addresses.to_vec(),
            connected_stream_so_linger,
        })
        .await?;

        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);
        let connected_proxy_address = proxy_stream.peer_addr()?;
        let MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
        } = MessageFramedGenerator::generate(proxy_stream, message_framed_buffer_size, compress, rsa_crypto_fetcher).await?;

        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: configuration.user_token().clone().unwrap(),
        })
        .await?;
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            connection_id: Some(connection_id.clone()),
            message_framed_write,
            payload_encryption_type,
            user_token: configuration.user_token().clone().unwrap(),
            ref_id: None,
            message_payload: Some(MessagePayload::new(
                client_address.into(),
                dest_address.clone().into(),
                PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                Bytes::new(),
            )),
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
            connection_id: connection_id.clone(),
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
                                source_address,
                                target_address,
                                ..
                            }),
                        ..
                    }),
            }) => {
                debug!(
                    "Connection [{}] tcp connect process success, response message id: {}",
                    connection_id, message_id
                );
                Ok(Socks5TcpConnectFlowResult {
                    client_address,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    target_address,
                    proxy_address: connected_proxy_address,
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
                    "Connection [{}] fail connect to target from proxy, response message id: {}",
                    connection_id, message_id
                );
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(
                        ErrorKind::ConnectionReset,
                        format!("Connection [{}] fail connect to target from proxy", connection_id)
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
                    "Connection [{}] fail connect to target from proxy because of invalid payload type:{:?}, response message id: {}",
                    connection_id, payload_type, message_id
                );
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                }))
            },
            Ok(ReadMessageFramedResult { .. }) => {
                error!(
                    "Connection [{}] fail connect to target from proxy because of unknown response content.",
                    connection_id
                );
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid response content read from proxy.",),
                }))
            },
        }
    }
}
