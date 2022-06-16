use std::net::SocketAddr;

use std::{io::ErrorKind, sync::Arc};

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;

use futures::SinkExt;

use tracing::error;

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedGenerateResult, MessageFramedGenerator, MessageFramedRead, MessageFramedReader,
    MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult,
    PayloadEncryptionTypeSelector, PayloadType, PpaassError, ProxyMessagePayloadTypeValue, ReadMessageFramedError, ReadMessageFramedRequest,
    ReadMessageFramedResult, ReadMessageServiceResultContent, RsaCryptoFetcher, TcpConnectRequest, TcpConnectResult, TcpConnector, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use crate::{
    config::AgentConfig,
    message::socks5::Socks5Addr,
    service::common::{DEFAULT_BUFFER_SIZE, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS},
};

pub(crate) struct Socks5TcpConnectService;

pub(crate) struct Socks5TcpConnectServiceRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
    pub dest_address: Socks5Addr,
}

pub(crate) struct Socks5TcpConnectServiceResponse<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub proxy_address: Option<SocketAddr>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub connect_response_message_id: String,
}

impl Socks5TcpConnectService {
    pub async fn exec<T>(
        request: Socks5TcpConnectServiceRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>,
    ) -> Result<Socks5TcpConnectServiceResponse<T>>
    where
        T: RsaCryptoFetcher,
    {
        let Socks5TcpConnectServiceRequest {
            connection_id,
            proxy_addresses,
            dest_address,
            ..
        } = request;
        let client_address = request.client_address;
        let target_stream_so_linger = configuration.proxy_stream_so_linger().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS);

        let TcpConnectResult { target_stream: proxy_stream } = TcpConnector::connect(TcpConnectRequest {
            target_addresses: proxy_addresses.to_vec(),
            target_stream_so_linger,
        })
        .await?;

        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);

        let MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
            framed_address,
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
            connection_id,
            message_framed_read,
            read_from_address: framed_address,
        })
        .await
        {
            Err(ReadMessageFramedError { source, .. }) => Err(anyhow!(source)),
            Ok(ReadMessageFramedResult { content: None, .. }) => Err(anyhow!(PpaassError::CodecError)),
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageServiceResultContent {
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
            }) => Ok(Socks5TcpConnectServiceResponse {
                client_address,
                message_framed_read,
                message_framed_write,
                source_address,
                target_address,
                connect_response_message_id: message_id,
                proxy_address: framed_address,
            }),
            Ok(ReadMessageFramedResult {
                content:
                    Some(ReadMessageServiceResultContent {
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                                ..
                            }),
                        ..
                    }),
                ..
            }) => Err(anyhow!(PpaassError::IoError {
                source: std::io::Error::new(ErrorKind::ConnectionReset, "Fail connect to target from proxy",),
            })),
            Ok(ReadMessageFramedResult { .. }) => {
                error!("Invalid payload type read from proxy.");
                Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                }))
            },
        }
    }
}
