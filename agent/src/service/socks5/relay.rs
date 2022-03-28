use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::{BufMut, BytesMut};
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
pub(crate) struct Socks5RelayServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub proxy_address_string: String,
    pub message_framed_write: MessageFramedWrite,
    pub message_framed_read: MessageFramedRead,
    pub connect_response_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}

pub(crate) struct Socks5RelayServiceResult {
    pub client_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Socks5RelayService {
    write_agent_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
    read_proxy_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
}

impl Default for Socks5RelayService {
    fn default() -> Self {
        Self {
            write_agent_message_service: BoxCloneService::new(WriteMessageService),
            read_proxy_message_service: BoxCloneService::new(ReadMessageService),
        }
    }
}

impl Service<Socks5RelayServiceRequest> for Socks5RelayService {
    type Response = Socks5RelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, ctx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let write_agent_message_service_ready = self.write_agent_message_service.poll_ready(ctx)?;
        let read_proxy_message_service_ready = self.read_proxy_message_service.poll_ready(ctx)?;
        if write_agent_message_service_ready.is_ready()
            && read_proxy_message_service_ready.is_ready()
        {
            debug!("Ready to do socks5 relay.");
            return Poll::Ready(Ok(()));
        }
        debug!("Not ready to do socks5 relay.");
        Poll::Pending
    }

    fn call(&mut self, request: Socks5RelayServiceRequest) -> Self::Future {
        let mut write_agent_message_service = self.write_agent_message_service.clone();
        let mut read_proxy_message_service = self.read_proxy_message_service.clone();
        Box::pin(async move {
            let client_stream = request.client_stream;
            let mut message_framed_read = request.message_framed_read;
            let mut message_framed_write = request.message_framed_write;
            let (mut client_stream_read_half, mut client_stream_write_half) =
                client_stream.into_split();
            let source_address_a2t = request.source_address.clone();
            let target_address_a2t = request.target_address.clone();
            let target_address_t2a = request.target_address.clone();
            let (relay_status_sender, mut relay_status_receiver) = tokio::sync::mpsc::channel::<bool>(1);
            tokio::spawn(async move {
                loop {
                    match relay_status_receiver.try_recv(){
                        Err(e)=>{
                            debug!("Proxy data reader goes well: {:#?}", e);
                        }
                        Ok(_)=>{
                            error!("Proxy data reader error happen.");
                            return;
                        }
                    }
                    let mut buf = BytesMut::with_capacity(
                        SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                    );
                    match client_stream_read_half.read_buf(&mut buf).await {
                        Err(e) => {
                            error!(
                                "Fail to read client data from {:#?} because of error: {:#?}",
                                target_address_a2t, e
                            );
                            return;
                        }
                        Ok(0) if buf.remaining_mut() > 0 => {
                            debug!("Read all data from agent.");
                            return;
                        }
                        Ok(size) => {
                            debug!("Read {} bytes from client", size);
                        }
                    }
                    let write_agent_message_result = ready_and_call_service(
                        &mut write_agent_message_service,
                        WriteMessageServiceRequest {
                            message_framed_write,
                            ref_id: Some(request.connect_response_message_id.clone()),
                            user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                            payload_encryption_type: PayloadEncryptionType::Blowfish(
                                generate_uuid().into(),
                            ),
                            message_payload: Some(MessagePayload::new(
                                source_address_a2t.clone(),
                                target_address_a2t.clone(),
                                PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                                buf.freeze(),
                            )),
                        },
                    )
                    .await;
                    let write_agent_message_result = match write_agent_message_result {
                        Err(_) => return,
                        Ok(v) => v,
                    };
                    message_framed_write = write_agent_message_result.message_framed_write;
                }
            });
            tokio::spawn(async move {
                loop {
                    let read_proxy_message_result = ready_and_call_service(
                        &mut read_proxy_message_service,
                        ReadMessageServiceRequest {
                            message_framed_read,
                        },
                    )
                    .await;
                    let ReadMessageServiceResult {
                        message_framed_read: message_framed_read_in_result,
                        message_payload:
                            MessagePayload {
                                data: proxy_raw_data,
                                ..
                            },
                        ..
                    } = match read_proxy_message_result {
                        Err(e) => {
                            error!("Fail to read proxy data because of error: {:#?}", e);
                            match relay_status_sender.try_send(true) {
                                Err(e) => {
                                    error!(
                                        "Fail to notice proxy data reader because of error: {:#?}.",
                                        e
                                    );
                                }
                                Ok(_) => {
                                    debug!("Success notice proxy data reader to close.");
                                }
                            };
                            return;
                        },
                        Ok(Some(
                            value @ ReadMessageServiceResult {
                                message_payload:
                                    MessagePayload {
                                        payload_type:
                                            PayloadType::ProxyPayload(
                                                ProxyMessagePayloadTypeValue::TcpData,
                                            ),
                                        ..
                                    },
                                ..
                            },
                        )) => value,
                        Ok(_) => return,
                    };
                    message_framed_read = message_framed_read_in_result;
                    if let Err(e) = client_stream_write_half
                        .write_all(proxy_raw_data.as_ref())
                        .await
                    {
                        error!(
                            "Fail to write proxy data from {:#?} to client because of error: {:#?}",
                            target_address_t2a, e
                        );
                        return;
                    };
                    if let Err(e) = client_stream_write_half.flush().await {
                        error!(
                            "Fail to flush proxy data from {:#?} to client because of error: {:#?}",
                            target_address_t2a, e
                        );
                        return;
                    };
                }
            });
            Ok(Socks5RelayServiceResult {
                client_address: request.client_address,
            })
        })
    }
}
