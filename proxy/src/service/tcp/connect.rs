use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::StreamExt;
use tokio::net::windows::named_pipe::PipeMode::Byte;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::util::{BoxCloneService, BoxService};
use tower::{Service, ServiceExt};
use tracing::error;
use tracing::log::debug;

use common::{AgentMessagePayloadTypeValue, CommonError, generate_uuid, MessageCodec, MessageFrameRead, MessageFrameWrite, MessagePayload, PayloadEncryptionType, PayloadType, ProxyMessagePayloadTypeValue};

use crate::config::{AGENT_PUBLIC_KEY, PROXY_PRIVATE_KEY};
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
}

#[derive(Clone)]
pub(crate) struct TcpConnectService {
    read_agent_message_service:
        BoxCloneService<ReadAgentMessageServiceRequest, ReadAgentMessageServiceResult, CommonError>,
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
        let mut write_proxy_message_service =self.write_proxy_message_service.clone();
        Box::pin(async move {
            let read_result = read_agent_message_service
                .ready()
                .await?
                .call(ReadAgentMessageServiceRequest {
                    agent_address: req.agent_address,
                    message_frame_read: req.message_frame_read,
                })
                .await?;
            let agent_message_payload = match read_result.agent_message_payload {
                Some(r) => r,
                None => return Err(CommonError::CodecError),
            };
            match agent_message_payload.payload_type {
                PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect) => {
                    let connect_result = connect_to_target_service
                        .ready()
                        .await?
                        .call(ConnectToTargetServiceRequest {
                            target_address: agent_message_payload.target_address.to_string(),
                            agent_address: req.agent_address,
                        })
                        .await?;
                    let connect_success_payload = MessagePayload::new(
                        agent_message_payload.source_address,
                        agent_message_payload.target_address,
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                        Bytes::new(),
                    );
                    write_proxy_message_service.ready().await?.call(WriteProxyMessageServiceRequest{
                        message_frame_write: req.message_frame_write,
                        proxy_message_payload: Some(connect_success_payload),
                        payload_encryption_type: PayloadEncryptionType::Blowfish(generate_uuid().into()),
                        user_token: agent_message_payload.
                    }).await?;
                    Ok(TcpConnectServiceResult {
                        target_stream: connect_result.target_stream,
                        message_frame_read: read_result.message_frame_read,
                        message_frame_write: req.message_frame_write,
                    })
                }
                _ => Err(CommonError::CodecError),
            }
        })
    }
}
