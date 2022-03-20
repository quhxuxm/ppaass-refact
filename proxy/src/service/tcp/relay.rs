use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::{debug, error, trace};

use common::{
    generate_uuid, CommonError, MessageFrameRead, MessageFrameWrite, MessagePayload, NetAddress,
    PayloadEncryptionType, PayloadType, ProxyMessagePayloadTypeValue,
};

use crate::service::tcp::close::{TcpCloseService, TcpCloseServiceRequest, TcpCloseServiceResult};
use crate::service::{
    ReadAgentMessageService, ReadAgentMessageServiceRequest, ReadAgentMessageServiceResult,
    WriteProxyMessageService, WriteProxyMessageServiceRequest, WriteProxyMessageServiceResult,
};

pub(crate) struct TcpRelayServiceRequest {
    pub message_frame_read: MessageFrameRead,
    pub message_frame_write: MessageFrameWrite,
    pub agent_address: SocketAddr,
    pub target_stream: TcpStream,
    pub agent_tcp_connect_message_id: String,
    pub user_token: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}
pub(crate) struct TcpRelayServiceResult {
    pub agent_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct TcpRelayService {
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
    tcp_close_service: BoxCloneService<TcpCloseServiceRequest, TcpCloseServiceResult, CommonError>,
}

impl TcpRelayService {
    pub(crate) fn new() -> Self {
        Self {
            tcp_close_service: BoxCloneService::new(TcpCloseService),
            read_agent_message_service: BoxCloneService::new(ReadAgentMessageService),
            write_proxy_message_service: BoxCloneService::new(WriteProxyMessageService),
        }
    }
}

impl Service<TcpRelayServiceRequest> for TcpRelayService {
    type Response = TcpRelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpRelayServiceRequest) -> Self::Future {
        let mut read_agent_message_service = self.read_agent_message_service.clone();
        let mut write_proxy_message_service = self.write_proxy_message_service.clone();
        let mut message_frame_read = req.message_frame_read;
        let mut message_frame_write = req.message_frame_write;
        let agent_address_for_a2t = req.agent_address.clone();
        let agent_address_for_t2a = req.agent_address.clone();
        let target_stream = req.target_stream;
        let agent_tcp_connect_message_id = req.agent_tcp_connect_message_id;
        let user_token = req.user_token;
        let agent_connect_message_source_address = req.source_address;
        let agent_connect_message_target_address = req.target_address;
        let (mut target_stream_read, mut target_stream_write) = target_stream.into_split();
        Box::pin(async move {
            tokio::spawn(async move {
                loop {
                    let check_ready_result = read_agent_message_service.ready().await;
                    let service_obj = match check_ready_result {
                        Err(e) => {
                            error!(
                                "Agent: {}, fail to read from agent because of error(ready): {:#?}",
                                agent_address_for_a2t, e
                            );
                            return;
                        }
                        Ok(v) => v,
                    };
                    match service_obj
                        .call(ReadAgentMessageServiceRequest {
                            message_frame_read,
                            agent_address: agent_address_for_a2t,
                        })
                        .await
                    {
                        Err(e) => {
                            error!(
                                "Agent: {}, fail to read from agent because of error: {:#?}",
                                agent_address_for_a2t, e
                            );
                            return;
                        }
                        Ok(None) => {
                            debug!("Agent: {}, nothing read from agent.", agent_address_for_a2t);
                            return;
                        }
                        Ok(Some(agent_message_read_result)) => {
                            trace!(
                                "Agent: {}, success read message from agent, agent message payload:\n{:#?}\n",
                                agent_address_for_a2t,
                                agent_message_read_result.agent_message_payload
                            );
                            message_frame_read = agent_message_read_result.message_frame_read;
                            let agent_message_payload =
                                agent_message_read_result.agent_message_payload;
                            target_stream_write
                                .write(agent_message_payload.data.as_ref())
                                .await;
                            target_stream_write.flush().await;
                        }
                    }
                }
            });
            tokio::spawn(async move {
                loop {
                    let mut buf = BytesMut::with_capacity(1024 * 64);
                    target_stream_read.read_buf(&mut buf).await;
                    let check_ready_result = write_proxy_message_service.ready().await;
                    let service_obj = match check_ready_result {
                        Err(e) => {
                            error!(
                                "Agent: {}, fail to read from target because of error(ready): {:#?}",
                                agent_address_for_a2t, e
                            );
                            return;
                        }
                        Ok(v) => v,
                    };
                    let proxy_message_payload = MessagePayload::new(
                        agent_connect_message_source_address.clone(),
                        agent_connect_message_target_address.clone(),
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                        buf.freeze(),
                    );
                    match service_obj
                        .call(WriteProxyMessageServiceRequest {
                            message_frame_write,
                            ref_id: Some(agent_tcp_connect_message_id.clone()),
                            user_token: user_token.clone(),
                            payload_encryption_type: PayloadEncryptionType::Blowfish(
                                generate_uuid().into(),
                            ),
                            proxy_message_payload: Some(proxy_message_payload),
                        })
                        .await
                    {
                        Err(e) => {
                            error!(
                            "Agent: {}, fail to read from target because of error(ready): {:#?}",
                            agent_address_for_t2a, e
                            );
                            return;
                        }
                        Ok(proxy_message_write_result) => {
                            message_frame_write = proxy_message_write_result.message_frame_write;
                        }
                    }
                }
            });
            Ok(TcpRelayServiceResult {
                agent_address: req.agent_address,
            })
        })
    }
}
