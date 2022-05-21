use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::future::BoxFuture;
use tokio::net::TcpStream;
use tower::Service;
use tower::ServiceBuilder;
use tracing::error;
use tracing::log::debug;

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest,
    PayloadEncryptionTypeSelectServiceResult, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageService, ReadMessageServiceRequest, ReadMessageServiceResult, WriteMessageService,
    WriteMessageServiceRequest,
};

use crate::config::{
    DEFAULT_CONNECT_TARGET_RETRY, DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS,
    DEFAULT_READ_AGENT_TIMEOUT_SECONDS,
};
use crate::service::{
    ConnectToTargetService, ConnectToTargetServiceRequest, ConnectToTargetServiceResult,
};
use crate::SERVER_CONFIG;

pub(crate) struct TcpConnectServiceRequest {
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub agent_address: SocketAddr,
}

impl Debug for TcpConnectServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "TcpConnectServiceRequest: agent_address={}",
            self.agent_address
        )
    }
}

pub(crate) struct TcpConnectServiceResult {
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub agent_tcp_connect_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
}

#[derive(Clone, Default)]
pub(crate) struct TcpConnectService;

impl Service<TcpConnectServiceRequest> for TcpConnectService {
    type Response = TcpConnectServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpConnectServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut read_agent_message_service =
                ServiceBuilder::new().service(ReadMessageService::new(
                    SERVER_CONFIG
                        .read_agent_timeout_seconds()
                        .unwrap_or(DEFAULT_READ_AGENT_TIMEOUT_SECONDS),
                ));
            let mut write_proxy_message_service =
                ServiceBuilder::new().service(WriteMessageService::default());
            let mut connect_to_target_service =
                ServiceBuilder::new().service(ConnectToTargetService::new(
                    SERVER_CONFIG
                        .target_connection_retry()
                        .unwrap_or(DEFAULT_CONNECT_TARGET_RETRY),
                    SERVER_CONFIG
                        .connect_target_timeout_seconds()
                        .unwrap_or(DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS),
                ));
            let mut payload_encryption_type_select_service =
                ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
            let read_agent_message_result = ready_and_call_service(
                &mut read_agent_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: req.message_framed_read,
                },
            )
            .await?;
            if let Some(ReadMessageServiceResult {
                message_payload:
                    MessagePayload {
                        payload_type:
                            PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                        target_address,
                        source_address,
                        ..
                    },
                message_framed_read,
                user_token,
                message_id,
            }) = read_agent_message_result
            {
                let PayloadEncryptionTypeSelectServiceResult {
                    payload_encryption_type,
                    ..
                } = ready_and_call_service(
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
                let ConnectToTargetServiceResult { target_stream } = match connect_to_target_result
                {
                    Err(e) => {
                        error!(
                            "Fail connect to target {:#?} because of error: {:#?}",
                            target_address, e
                        );
                        let connect_fail_payload = MessagePayload::new(
                            source_address,
                            target_address,
                            PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                            Bytes::new(),
                        );
                        ready_and_call_service(
                            &mut write_proxy_message_service,
                            WriteMessageServiceRequest {
                                message_framed_write: req.message_framed_write,
                                message_payload: Some(connect_fail_payload),
                                payload_encryption_type,
                                user_token,
                                ref_id: Some(message_id),
                            },
                        )
                        .await?;
                        return Err(e);
                    },
                    Ok(v) => v,
                };
                debug!(
                    "Agent {}, success connect to target {:#?}",
                    req.agent_address, target_address
                );
                let connect_success_payload = MessagePayload::new(
                    source_address.clone(),
                    target_address.clone(),
                    PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                    Bytes::new(),
                );
                let write_proxy_message_result = ready_and_call_service(
                    &mut write_proxy_message_service,
                    WriteMessageServiceRequest {
                        message_framed_write: req.message_framed_write,
                        message_payload: Some(connect_success_payload),
                        payload_encryption_type,
                        user_token: user_token.clone(),
                        ref_id: Some(message_id.clone()),
                    },
                )
                .await?;
                return Ok(TcpConnectServiceResult {
                    message_framed_write: write_proxy_message_result.message_framed_write,
                    message_framed_read,
                    target_stream,
                    agent_tcp_connect_message_id: message_id,
                    source_address,
                    target_address,
                    user_token,
                });
            };
            Err(CommonError::CodecError)
        })
    }
}
