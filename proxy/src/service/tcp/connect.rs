use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use anyhow::anyhow;
use bytes::Bytes;
use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest, PayloadEncryptionTypeSelectServiceResult, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest, ReadMessageServiceResult, ReadMessageServiceResultContent, RsaCryptoFetcher,
    WriteMessageService, WriteMessageServiceError, WriteMessageServiceRequest, WriteMessageServiceResult,
};
use futures::future::BoxFuture;
use futures::SinkExt;
use tokio::net::TcpStream;
use tower::Service;
use tower::ServiceBuilder;
use tracing::error;
use tracing::log::debug;

use crate::config::{ProxyConfig, DEFAULT_CONNECT_TARGET_RETRY, DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS, DEFAULT_TARGET_STREAM_SO_LINGER};
use crate::service::{ConnectToTargetService, ConnectToTargetServiceRequest, ConnectToTargetServiceResult};

pub(crate) struct TcpConnectServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub agent_address: SocketAddr,
    pub configuration: Arc<ProxyConfig>,
}

impl<T> Debug for TcpConnectServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "TcpConnectServiceRequest: agent_address={}", self.agent_address)
    }
}

pub(crate) struct TcpConnectServiceResult<T>
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
pub(crate) struct TcpConnectService;

impl<T> Service<TcpConnectServiceRequest<T>> for TcpConnectService
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = TcpConnectServiceResult<T>;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpConnectServiceRequest<T>) -> Self::Future {
        Box::pin(async move {
            let mut read_agent_message_service: ReadMessageService = Default::default();
            let mut write_proxy_message_service = WriteMessageService::default();
            let mut connect_to_target_service = ServiceBuilder::new().service(ConnectToTargetService::new(
                req.configuration.target_connection_retry().unwrap_or(DEFAULT_CONNECT_TARGET_RETRY),
                req.configuration
                    .connect_target_timeout_seconds()
                    .unwrap_or(DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS),
                req.configuration.target_stream_so_linger().unwrap_or(DEFAULT_TARGET_STREAM_SO_LINGER),
            ));

            let mut payload_encryption_type_select_service: PayloadEncryptionTypeSelectService = Default::default();
            let read_agent_message_result = ready_and_call_service(
                &mut read_agent_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: req.message_framed_read,
                    read_from_address: Some(req.agent_address),
                },
            )
            .await;
            if let Ok(ReadMessageServiceResult {
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
                let PayloadEncryptionTypeSelectServiceResult { payload_encryption_type, .. } = ready_and_call_service(
                    &mut payload_encryption_type_select_service,
                    PayloadEncryptionTypeSelectServiceRequest {
                        encryption_token: generate_uuid().into(),
                        user_token: user_token.clone(),
                    },
                )
                .await?;
                let connect_to_target_result = ready_and_call_service(
                    &mut connect_to_target_service,
                    ConnectToTargetServiceRequest {
                        target_address: target_address.to_string(),
                        agent_address: req.agent_address,
                    },
                )
                .await;
                let ConnectToTargetServiceResult { target_stream } = match connect_to_target_result {
                    Err(e) => {
                        error!("Fail connect to target {:#?} because of error: {:#?}", target_address, e);
                        let connect_fail_payload = MessagePayload::new(
                            source_address,
                            target_address,
                            PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                            Bytes::new(),
                        );
                        match ready_and_call_service(
                            &mut write_proxy_message_service,
                            WriteMessageServiceRequest {
                                message_framed_write: req.message_framed_write,
                                message_payload: Some(connect_fail_payload),
                                payload_encryption_type,
                                user_token: user_token.clone(),
                                ref_id: Some(message_id),
                            },
                        )
                        .await
                        {
                            Err(WriteMessageServiceError { source, .. }) => {
                                return Err(anyhow!(source));
                            },
                            Ok(WriteMessageServiceResult { mut message_framed_write }) => {
                                if let Err(e) = message_framed_write.flush().await {
                                    return Err(anyhow!(e));
                                }
                            },
                        };
                        return Err(anyhow!(e));
                    },
                    Ok(v) => v,
                };
                debug!("Agent {}, success connect to target {:#?}", req.agent_address, target_address);
                let connect_success_payload = MessagePayload::new(
                    source_address.clone(),
                    target_address.clone(),
                    PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                    Bytes::new(),
                );
                let message_framed_write = match ready_and_call_service(
                    &mut write_proxy_message_service,
                    WriteMessageServiceRequest {
                        message_framed_write: req.message_framed_write,
                        message_payload: Some(connect_success_payload),
                        payload_encryption_type,
                        user_token: user_token.clone(),
                        ref_id: Some(message_id.clone()),
                    },
                )
                .await
                {
                    Err(WriteMessageServiceError { source, .. }) => return Err(anyhow!(source)),
                    Ok(WriteMessageServiceResult { mut message_framed_write }) => {
                        if let Err(e) = message_framed_write.flush().await {
                            return Err(anyhow!(e));
                        }
                        message_framed_write
                    },
                };
                return Ok(TcpConnectServiceResult {
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
        })
    }
}
