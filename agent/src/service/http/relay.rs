use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::{BufMut, BytesMut};
use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::error::TryRecvError,
};
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
pub(crate) struct HttpRelayServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite,
    pub message_framed_read: MessageFramedRead,
    pub connect_response_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
}
pub(crate) struct HttpRelayServiceResult {
    pub client_address: SocketAddr,
}

#[derive(Clone, Debug)]
pub(crate) struct HttpRelayService {
    write_agent_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
    read_proxy_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
}
impl Default for HttpRelayService {
    fn default() -> Self {
        Self {
            write_agent_message_service: BoxCloneService::new(WriteMessageService),
            read_proxy_message_service: BoxCloneService::new(ReadMessageService),
        }
    }
}
impl Service<HttpRelayServiceRequest> for HttpRelayService {
    type Response = HttpRelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let write_agent_message_service_ready = self.write_agent_message_service.poll_ready(cx)?;
        let read_proxy_message_service_ready = self.read_proxy_message_service.poll_ready(cx)?;
        if write_agent_message_service_ready.is_ready()
            && read_proxy_message_service_ready.is_ready()
        {
            debug!("Ready to do http relay.");
            return Poll::Ready(Ok(()));
        }
        debug!("Not ready to do http relay.");
        Poll::Pending
    }

    fn call(&mut self, request: HttpRelayServiceRequest) -> Self::Future {
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

            let (proxy_reader_error_sender, mut proxy_reader_error_receiver) =
                tokio::sync::mpsc::channel::<bool>(1);
            let (proxy_writer_error_sender, mut proxy_writer_error_receiver) =
                tokio::sync::mpsc::channel::<bool>(1);

            let (client_reader_error_sender, mut client_reader_error_receiver) =
                tokio::sync::mpsc::channel::<bool>(1);
            let (client_writer_error_sender, mut client_writer_error_receiver) =
                tokio::sync::mpsc::channel::<bool>(1);

            tokio::spawn(async move {
                if let Some(init_data) = request.init_data {
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
                                init_data.into(),
                            )),
                        },
                    )
                    .await;
                    let _write_agent_message_result = match write_agent_message_result {
                        Err(e) => {
                            error!(
                                "Fail to write agent message to proxy because of error: {:#?}",
                                e
                            );
                            if let Err(e) = proxy_writer_error_sender.try_send(true) {
                                error!("Fail to notice proxy writer error happen because of error: {:#?}", e);
                            }
                            return;
                        }
                        Ok(v) => message_framed_write = v.message_framed_write,
                    };
                    match proxy_reader_error_receiver.try_recv() {
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                debug!("Proxy data reader goes well: {:#?}", e);
                            }
                            TryRecvError::Disconnected => return,
                        },
                        Ok(_) => {
                            error!("Proxy data reader error happen.");
                            return;
                        }
                    }
                    match client_writer_error_receiver.try_recv() {
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                debug!("Client data writer goes well: {:#?}", e);
                            }
                            TryRecvError::Disconnected => return,
                        },
                        Ok(_) => {
                            error!("Client data writer error happen.");
                            return;
                        }
                    }
                }
                loop {
                    let mut buf = BytesMut::with_capacity(
                        SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                    );
                    match client_stream_read_half.read_buf(&mut buf).await {
                        Err(e) => {
                            error!(
                                "Fail to read client data from {:#?} because of error: {:#?}",
                                target_address_a2t, e
                            );
                            if let Err(e) = client_reader_error_sender.try_send(true) {
                                error!(
                                    "Fail to notice client reader error because of error: {:#?}",
                                    e
                                );
                            }
                            return;
                        }
                        Ok(0) if buf.remaining_mut() > 0 => {
                            debug!("Read all data from agent");
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
                        Err(e) => {
                            error!(
                                "Fail to write agent message to proxy because of error: {:#?}",
                                e
                            );
                            if let Err(e) = proxy_writer_error_sender.try_send(true) {
                                error!("Fail to notice proxy writer error happen because of error: {:#?}", e);
                            }
                            return;
                        }
                        Ok(v) => v,
                    };
                    message_framed_write = write_agent_message_result.message_framed_write;
                    match proxy_reader_error_receiver.try_recv() {
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                debug!("Proxy data reader goes well: {:#?}", e);
                            }
                            TryRecvError::Disconnected => return,
                        },
                        Ok(_) => {
                            error!("Proxy data reader error happen.");
                            return;
                        }
                    }
                    match client_writer_error_receiver.try_recv() {
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                debug!("Client data writer goes well: {:#?}", e);
                            }
                            TryRecvError::Disconnected => return,
                        },
                        Ok(_) => {
                            error!("Client data writer error happen.");
                            return;
                        }
                    }
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
                            match proxy_reader_error_sender.try_send(true) {
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
                        }
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
                        if let Err(e) = client_writer_error_sender.try_send(true) {
                            error!(
                                "Fail to notice client writer error because of error: {:#?}",
                                e
                            );
                        }
                        return;
                    };
                    if let Err(e) = client_stream_write_half.flush().await {
                        error!(
                            "Fail to flush proxy data from {:#?} to client because of error: {:#?}",
                            target_address_t2a, e
                        );
                        return;
                    };
                    match proxy_writer_error_receiver.try_recv() {
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                debug!("Proxy data writer goes well: {:#?}", e);
                            }
                            TryRecvError::Disconnected => return,
                        },
                        Ok(_) => {
                            error!("Proxy data writer error happen.");
                            return;
                        }
                    }
                    match client_reader_error_receiver.try_recv() {
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                debug!("Client reader goes well: {:#?}", e);
                            }
                            TryRecvError::Disconnected => return,
                        },
                        Ok(_) => {
                            error!("Client reader error happen.");
                            return;
                        }
                    }
                }
            });
            Ok(HttpRelayServiceResult {
                client_address: request.client_address,
            })
        })
    }
}
