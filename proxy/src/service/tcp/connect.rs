use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{future::BoxFuture, StreamExt};
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::error;
use tracing::log::debug;

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType,
    PayloadType, ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, WriteMessageService, WriteMessageServiceRequest,
    WriteMessageServiceResult,
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
pub(crate) struct TcpConnectServiceResult {
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub agent_tcp_connect_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
}

#[derive(Clone)]
pub(crate) struct TcpConnectService {
    read_agent_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
    write_proxy_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
    connect_to_target_service:
        BoxCloneService<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, CommonError>,
}

impl Default for TcpConnectService {
    fn default() -> Self {
        Self {
            read_agent_message_service: BoxCloneService::new(ReadMessageService),
            write_proxy_message_service: BoxCloneService::new(WriteMessageService),
            connect_to_target_service: BoxCloneService::new(ConnectToTargetService::new(
                SERVER_CONFIG.target_connection_retry().unwrap_or(3),
            )),
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
                            "Agent {}, fail connect to target {:#?} because of error: {:#?}",
                            req.agent_address, target_address, e
                        );
                        let connect_fail_payload = MessagePayload::new(
                            source_address,
                            target_address,
                            PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                            Bytes::new(),
                        );
                        write_proxy_message_service
                            .ready()
                            .await?
                            .call(WriteMessageServiceRequest {
                                message_framed_write: req.message_framed_write,
                                message_payload: Some(connect_fail_payload),
                                payload_encryption_type: PayloadEncryptionType::Blowfish(
                                    generate_uuid().into(),
                                ),
                                user_token,
                                ref_id: Some(message_id),
                            })
                            .await?;
                        return Err(e);
                    }
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
                        payload_encryption_type: PayloadEncryptionType::Blowfish(
                            generate_uuid().into(),
                        ),
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
