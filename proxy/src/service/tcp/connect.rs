use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{future::BoxFuture, StreamExt};
use tokio::net::TcpStream;
use tower::{Service, ServiceExt};
use tower::util::BoxCloneService;
use tracing::error;
use tracing::log::debug;

use common::{
    AgentMessagePayloadTypeValue, CallServiceResult, CommonError, general_call_service,
    generate_uuid, MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionType, PayloadType, ProxyMessagePayloadTypeValue, ReadMessageService,
    ReadMessageServiceRequest, ReadMessageServiceResult, WriteMessageService,
    WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::SERVER_CONFIG;
use crate::service::{
    ConnectToTargetService, ConnectToTargetServiceRequest, ConnectToTargetServiceResult,
};

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
            let read_agent_message_result = general_call_service(
                read_agent_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: req.message_framed_read,
                },
            ).await?;
            let CallServiceResult {
                service,
                result:
                Some(ReadMessageServiceResult {
                    message_payload,
                    message_framed_read,
                    user_token,
                    message_id,
                }),
                ..
            } = match read_agent_message_result {
                CallServiceResult {
                    result: None,
                    ..
                } => {
                    return Err(CommonError::CodecError);
                }
                v => v
            };
            if let PayloadType::ProxyPayload(_) = message_payload.payload_type {
                return Err(CommonError::CodecError);
            }
            return match message_payload.payload_type {
                PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect) => {
                    let connect_result = match connect_to_target_service
                        .ready()
                        .await?
                        .call(ConnectToTargetServiceRequest {
                            target_address: message_payload.target_address.to_string(),
                            agent_address: req.agent_address,
                        })
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            error!(
                                "Agent {}, fail connect to target {:#?} because of error: {:#?}",
                                req.agent_address, message_payload.target_address, e
                            );
                            let connect_fail_payload = MessagePayload::new(
                                message_payload.source_address,
                                message_payload.target_address,
                                PayloadType::ProxyPayload(
                                    ProxyMessagePayloadTypeValue::TcpConnectFail,
                                ),
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
                    };
                    debug!(
                        "Agent {}, success connect to target {:#?}",
                        req.agent_address, message_payload.target_address
                    );
                    let connect_success_payload = MessagePayload::new(
                        message_payload.source_address.clone(),
                        message_payload.target_address.clone(),
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                        Bytes::new(),
                    );
                    let write_proxy_message_result = write_proxy_message_service
                        .ready()
                        .await?
                        .call(WriteMessageServiceRequest {
                            message_framed_write: req.message_framed_write,
                            message_payload: Some(connect_success_payload),
                            payload_encryption_type: PayloadEncryptionType::Blowfish(
                                generate_uuid().into(),
                            ),
                            user_token: user_token.clone(),
                            ref_id: Some(message_id.clone()),
                        })
                        .await?;
                    return Ok(TcpConnectServiceResult {
                        message_framed_write: write_proxy_message_result.message_framed_write,
                        message_framed_read,
                        target_stream: connect_result.target_stream,
                        agent_tcp_connect_message_id: message_id,
                        source_address: message_payload.source_address,
                        target_address: message_payload.target_address,
                        user_token,
                    });
                }
                _ => Err(CommonError::CodecError),
            };
        })
    }
}
