use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::Service;
use tracing::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType,
    PayloadType, ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, WriteMessageService, WriteMessageServiceRequest,
    WriteMessageServiceResult,
};

use crate::SERVER_CONFIG;

const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;

pub(crate) struct TcpRelayServiceRequest {
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
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
    read_agent_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
    write_proxy_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
}

impl Default for TcpRelayService {
    fn default() -> Self {
        Self {
            read_agent_message_service: BoxCloneService::new(ReadMessageService),
            write_proxy_message_service: BoxCloneService::new(WriteMessageService),
        }
    }
}

impl Service<TcpRelayServiceRequest> for TcpRelayService {
    type Response = TcpRelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let read_agent_message_service_ready = self.read_agent_message_service.poll_ready(cx);
        let write_proxy_message_service_ready = self.write_proxy_message_service.poll_ready(cx);
        if read_agent_message_service_ready.is_ready()
            && write_proxy_message_service_ready.is_ready()
        {
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }

    fn call(&mut self, req: TcpRelayServiceRequest) -> Self::Future {
        let mut read_agent_message_service = self.read_agent_message_service.clone();
        let mut write_proxy_message_service = self.write_proxy_message_service.clone();
        let mut message_framed_read = req.message_framed_read;
        let mut message_framed_write = req.message_framed_write;
        let target_stream = req.target_stream;
        let agent_tcp_connect_message_id = req.agent_tcp_connect_message_id;
        let user_token = req.user_token;
        let agent_connect_message_source_address = req.source_address;
        let agent_connect_message_target_address = req.target_address;
        let (mut target_stream_read, mut target_stream_write) = target_stream.into_split();
        Box::pin(async move {
            tokio::spawn(async move {
                loop {
                    let read_agent_message_result = ready_and_call_service(
                        &mut read_agent_message_service,
                        ReadMessageServiceRequest {
                            message_framed_read,
                        },
                    )
                    .await;
                    let ReadMessageServiceResult {
                        message_payload: MessagePayload { data, .. },
                        message_framed_read: message_framed_read_from_read_agent_result,
                        ..
                    } = match read_agent_message_result {
                        Ok(None) => return,
                        Ok(Some(
                            v @ ReadMessageServiceResult {
                                message_payload:
                                    MessagePayload {
                                        payload_type:
                                            PayloadType::AgentPayload(
                                                AgentMessagePayloadTypeValue::TcpData,
                                            ),
                                        ..
                                    },
                                ..
                            },
                        )) => v,
                        Ok(_) => {
                            error!("Invalid payload type.");
                            return;
                        }
                        Err(e) => {
                            error!("Fail to read from agent because of error: {:#?}", e);
                            return;
                        }
                    };
                    message_framed_read = message_framed_read_from_read_agent_result;
                    if let Err(e) = target_stream_write.write(data.as_ref()).await {
                        error!(
                            "Fail to write from agent to target because of error: {:#?}",
                            e
                        );
                        return;
                    };
                    if let Err(e) = target_stream_write.flush().await {
                        error!(
                            "Fail to flush from agent to target because of error: {:#?}",
                            e
                        );
                        return;
                    };
                }
            });
            tokio::spawn(async move {
                loop {
                    let mut buf = BytesMut::with_capacity(
                        SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                    );
                    match target_stream_read.read_buf(&mut buf).await {
                        Err(e) => {
                            error!("Fail to read data from target because of error: {:#?}", e);
                            return;
                        }
                        Ok(0) => {
                            debug!("Read all data from target.");
                            return;
                        }
                        Ok(size) => {
                            debug!("Read {} bytes from target.", size)
                        }
                    };
                    let proxy_message_payload = MessagePayload::new(
                        agent_connect_message_source_address.clone(),
                        agent_connect_message_target_address.clone(),
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                        buf.freeze(),
                    );
                    let write_proxy_message_result = ready_and_call_service(
                        &mut write_proxy_message_service,
                        WriteMessageServiceRequest {
                            message_framed_write,
                            ref_id: Some(agent_tcp_connect_message_id.clone()),
                            user_token: user_token.clone(),
                            payload_encryption_type: PayloadEncryptionType::Blowfish(
                                generate_uuid().into(),
                            ),
                            message_payload: Some(proxy_message_payload),
                        },
                    )
                    .await;
                    match write_proxy_message_result {
                        Err(e) => {
                            error!("Fail to read from target because of error(ready): {:#?}", e);
                            return;
                        }
                        Ok(proxy_message_write_result) => {
                            message_framed_write = proxy_message_write_result.message_framed_write;
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
