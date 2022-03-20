use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::StreamExt;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::error;
use tracing::log::debug;

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, CommonError, MessageFrameRead, MessageFrameWrite,
    MessagePayload, NetAddress, PayloadEncryptionType, PayloadType, ProxyMessagePayloadTypeValue,
};

use crate::service::{
    ConnectToTargetService, ConnectToTargetServiceRequest, ConnectToTargetServiceResult,
    ReadAgentMessageService, ReadAgentMessageServiceRequest, ReadAgentMessageServiceResult,
    WriteProxyMessageService, WriteProxyMessageServiceRequest, WriteProxyMessageServiceResult,
};

pub(crate) struct TcpConnectServiceRequest {
    pub message_frame_read: MessageFrameRead,
    pub message_frame_write: MessageFrameWrite,
    pub agent_address: SocketAddr,
}
pub(crate) struct TcpConnectServiceResult {
    pub target_stream: TcpStream,
    pub message_frame_read: MessageFrameRead,
    pub message_frame_write: MessageFrameWrite,
    pub agent_tcp_connect_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
}

#[derive(Clone)]
pub(crate) struct TcpConnectService {
    read_agent_message_service: BoxCloneService<
        ReadAgentMessageServiceRequest,
        Option<ReadAgentMessageServiceResult>,
        CommonError,
    >,
    write_proxy_message_service: BoxCloneService<
        WriteProxyMessageServiceRequest,
        WriteProxyMessageServiceResult,
        CommonError,
    >,
    connect_to_target_service:
        BoxCloneService<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, CommonError>,
}

impl TcpConnectService {
    pub(crate) fn new() -> Self {
        Self {
            read_agent_message_service: BoxCloneService::new(ReadAgentMessageService),
            write_proxy_message_service: BoxCloneService::new(WriteProxyMessageService),
            connect_to_target_service: BoxCloneService::new(ConnectToTargetService::new(3)),
        }
    }
}

impl Service<TcpConnectServiceRequest> for TcpConnectService {
    type Response = TcpConnectServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpConnectServiceRequest) -> Self::Future {
        let mut read_agent_message_service = self.read_agent_message_service.clone();
        let mut connect_to_target_service = self.connect_to_target_service.clone();
        let mut write_proxy_message_service = self.write_proxy_message_service.clone();
        Box::pin(async move {
            let read_result = match read_agent_message_service
                .ready()
                .await?
                .call(ReadAgentMessageServiceRequest {
                    agent_address: req.agent_address,
                    message_frame_read: req.message_frame_read,
                })
                .await?
            {
                None => return Err(CommonError::CodecError),
                Some(r) => r,
            };
            let agent_message_payload = read_result.agent_message_payload;
            if let PayloadType::ProxyPayload(_) = agent_message_payload.payload_type {
                return Err(CommonError::CodecError);
            }
            return match agent_message_payload.payload_type {
                PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect) => {
                    let connect_result = match connect_to_target_service
                        .ready()
                        .await?
                        .call(ConnectToTargetServiceRequest {
                            target_address: agent_message_payload.target_address.to_string(),
                            agent_address: req.agent_address,
                        })
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            error!(
                                "Agent {}, fail connect to target {:#?} because of error: {:#?}",
                                req.agent_address, agent_message_payload.target_address, e
                            );
                            let connect_fail_payload = MessagePayload::new(
                                agent_message_payload.source_address,
                                agent_message_payload.target_address,
                                PayloadType::ProxyPayload(
                                    ProxyMessagePayloadTypeValue::TcpConnectFail,
                                ),
                                Bytes::new(),
                            );
                            write_proxy_message_service
                                .ready()
                                .await?
                                .call(WriteProxyMessageServiceRequest {
                                    message_frame_write: req.message_frame_write,
                                    proxy_message_payload: Some(connect_fail_payload),
                                    payload_encryption_type: PayloadEncryptionType::Blowfish(
                                        generate_uuid().into(),
                                    ),
                                    user_token: read_result.user_token,
                                    ref_id: Some(read_result.message_id),
                                })
                                .await?;

                            return Err(e);
                        }
                    };
                    debug!(
                        "Agent {}, success connect to target {:#?}",
                        req.agent_address, agent_message_payload.target_address
                    );
                    let connect_success_payload = MessagePayload::new(
                        agent_message_payload.source_address.clone(),
                        agent_message_payload.target_address.clone(),
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                        Bytes::new(),
                    );
                    let write_proxy_message_result = write_proxy_message_service
                        .ready()
                        .await?
                        .call(WriteProxyMessageServiceRequest {
                            message_frame_write: req.message_frame_write,
                            proxy_message_payload: Some(connect_success_payload),
                            payload_encryption_type: PayloadEncryptionType::Blowfish(
                                generate_uuid().into(),
                            ),
                            user_token: read_result.user_token.clone(),
                            ref_id: Some(read_result.message_id.clone()),
                        })
                        .await?;
                    return Ok(TcpConnectServiceResult {
                        message_frame_write: write_proxy_message_result.message_frame_write,
                        message_frame_read: read_result.message_frame_read,
                        target_stream: connect_result.target_stream,
                        agent_tcp_connect_message_id: read_result.message_id,
                        source_address: agent_message_payload.source_address,
                        target_address: agent_message_payload.target_address,
                        user_token: read_result.user_token,
                    });
                }
                _ => Err(CommonError::CodecError),
            };
        })
    }
}
