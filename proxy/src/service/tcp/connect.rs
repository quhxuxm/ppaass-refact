use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageServiceResultContent, RsaCryptoFetcher, TcpConnectRequest,
    TcpConnectResult, TcpConnector, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};

use futures::SinkExt;
use std::net::SocketAddr;

use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};
use tokio::net::TcpStream;

use tracing::debug;
use tracing::error;

use crate::config::{ProxyConfig, DEFAULT_TARGET_STREAM_SO_LINGER};

pub(crate) struct TcpConnectProcessRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub agent_address: SocketAddr,
}

pub(crate) struct TcpConnectProcessResult<T>
where
    T: RsaCryptoFetcher,
{
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub agent_tcp_connect_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
}

#[derive(Clone, Default)]
pub(crate) struct TcpConnectProcess;

impl TcpConnectProcess {
    pub async fn exec<T>(&self, request: TcpConnectProcessRequest<T>, configuration: Arc<ProxyConfig>) -> Result<TcpConnectProcessResult<T>>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let TcpConnectProcessRequest {
            connection_id,
            message_framed_read,
            message_framed_write,
            agent_address,
        } = request;
        let target_stream_so_linger = configuration.target_stream_so_linger().unwrap_or(DEFAULT_TARGET_STREAM_SO_LINGER);
        let read_agent_message_result = MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: connection_id.clone(),
            message_framed_read,
            read_from_address: Some(agent_address),
        })
        .await;
        if let Ok(ReadMessageFramedResult {
            message_framed_read,
            content:
                Some(ReadMessageServiceResultContent {
                    message_id,
                    user_token,
                    message_payload:
                        Some(MessagePayload {
                            payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                            target_address,
                            source_address,
                            ..
                        }),
                    ..
                }),
            ..
        }) = read_agent_message_result
        {
            let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                encryption_token: generate_uuid().into(),
                user_token: user_token.clone(),
            })
            .await?;
            let connect_to_target_result = TcpConnector::connect(TcpConnectRequest {
                target_addresses: vec![target_address.clone().try_into()?],
                target_stream_so_linger,
            })
            .await;
            let TcpConnectResult { target_stream } = match connect_to_target_result {
                Err(e) => {
                    error!(
                        "Connection [{}] fail connect to target {:#?} because of error: {:#?}",
                        connection_id, target_address, e
                    );
                    let connect_fail_payload = MessagePayload::new(
                        source_address,
                        target_address,
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                        Bytes::new(),
                    );
                    match MessageFramedWriter::write(WriteMessageFramedRequest {
                        message_framed_write,
                        message_payload: Some(connect_fail_payload),
                        payload_encryption_type,
                        user_token: user_token.clone(),
                        ref_id: Some(message_id),
                        connection_id: Some(connection_id),
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
                        },
                    };
                    return Err(anyhow!(e));
                },
                Ok(v) => v,
            };
            debug!(
                "Connection [{}] agent address: {}, success connect to target {:#?}",
                connection_id, agent_address, target_address
            );
            let connect_success_payload = MessagePayload::new(
                source_address.clone(),
                target_address.clone(),
                PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                Bytes::new(),
            );
            let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                message_framed_write,
                message_payload: Some(connect_success_payload),
                payload_encryption_type,
                user_token: user_token.clone(),
                ref_id: Some(message_id.clone()),
                connection_id: Some(connection_id),
            })
            .await
            {
                Err(WriteMessageFramedError { source, .. }) => return Err(anyhow!(source)),
                Ok(WriteMessageFramedResult { mut message_framed_write }) => {
                    if let Err(e) = message_framed_write.flush().await {
                        return Err(anyhow!(e));
                    }
                    message_framed_write
                },
            };
            return Ok(TcpConnectProcessResult {
                message_framed_write,
                message_framed_read,
                target_stream,
                agent_tcp_connect_message_id: message_id,
                source_address,
                target_address,
                user_token,
            });
        };
        Err(anyhow!(PpaassError::CodecError))
    }
}
