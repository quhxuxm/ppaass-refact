use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::{Service, ServiceExt};
use tower::util::BoxCloneService;
use tracing::{debug, error, trace};

use common::{
    AgentMessagePayloadTypeValue, CommonError, generate_uuid, MessageFramedRead,
    MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType, PayloadType,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, WriteMessageService, WriteMessageServiceRequest,
    WriteMessageServiceResult,
};

use crate::SERVER_CONFIG;

const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;

pub(crate) struct TcpRelayServiceRequest {
    pub message_frame_read: MessageFramedRead,
    pub message_frame_write: MessageFramedWrite,
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
    read_agent_message_service: BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
    write_proxy_message_service: BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
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
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpRelayServiceRequest) -> Self::Future {
        let mut read_agent_message_service = self.read_agent_message_service.clone();
        let mut write_proxy_message_service = self.write_proxy_message_service.clone();
        let mut message_frame_read = req.message_frame_read;
        let mut message_frame_write = req.message_frame_write;
        let target_stream = req.target_stream;
        let agent_tcp_connect_message_id = req.agent_tcp_connect_message_id;
        let user_token = req.user_token;
        let agent_connect_message_source_address = req.source_address;
        let agent_connect_message_target_address = req.target_address;
        let (mut target_stream_read, mut target_stream_write) = target_stream.into_split();
        Box::pin(async move {
            tokio::spawn(async move {
                loop {
                    let service_obj = match read_agent_message_service.ready().await {
                        Err(e) => {
                            error!("Fail to read from agent because of error: {:#?}", e);
                            return;
                        }
                        Ok(v) => v,
                    };
                    match service_obj.call(ReadMessageServiceRequest {
                        message_framed_read: message_frame_read,
                    }).await {
                        Err(e) => {
                            error!("Fail to read from agent because of error: {:#?}", e);
                            return;
                        }
                        Ok(None) => {
                            debug!("Nothing read from agent.");
                            return;
                        }
                        Ok(Some(agent_message_read_result)) => {
                            trace!(
                                "Success read message from agent, agent message payload:\n{:#?}\n",
                                agent_message_read_result.message_payload
                            );
                            message_frame_read = agent_message_read_result.message_framed_read;
                            let agent_message_payload = agent_message_read_result.message_payload;
                            match agent_message_payload.payload_type {
                                PayloadType::AgentPayload(
                                    AgentMessagePayloadTypeValue::TcpData,
                                ) => {
                                    if let Err(e) = target_stream_write.write(agent_message_payload.data.as_ref()).await {
                                        error!("Fail to write from agent to target because of error: {:#?}",e);
                                        return;
                                    };
                                    if let Err(e) = target_stream_write.flush().await {
                                        error!( "Fail to flush from agent to target because of error: {:#?}",e);
                                        return;
                                    };
                                }
                                _ => {
                                    error!("Invalid payload type.");
                                    return;
                                }
                            }
                        }
                    }
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
                    let service_obj = match write_proxy_message_service.ready().await {
                        Err(e) => {
                            error!("Fail to read from target because of error(ready): {:#?}", e);
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
                    match service_obj.call(WriteMessageServiceRequest {
                        message_framed_write: message_frame_write,
                        ref_id: Some(agent_tcp_connect_message_id.clone()),
                        user_token: user_token.clone(),
                        payload_encryption_type: PayloadEncryptionType::Blowfish(
                            generate_uuid().into(),
                        ),
                        message_payload: Some(proxy_message_payload),
                    }).await {
                        Err(e) => {
                            error!("Fail to read from target because of error(ready): {:#?}", e);
                            return;
                        }
                        Ok(proxy_message_write_result) => {
                            message_frame_write = proxy_message_write_result.message_framed_write;
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
