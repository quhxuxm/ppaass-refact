use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::Service;
use tower::ServiceBuilder;
use tracing::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest,
    PayloadEncryptionTypeSelectServiceResult, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageService, ReadMessageServiceRequest, ReadMessageServiceResult, WriteMessageService,
    WriteMessageServiceRequest,
};

use crate::service::common::{
    DEFAULT_BUFFER_SIZE, DEFAULT_READ_CLIENT_TIMEOUT_SECONDS, DEFAULT_READ_PROXY_TIMEOUT_SECONDS,
};
use crate::SERVER_CONFIG;

#[allow(unused)]
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

#[allow(unused)]
pub(crate) struct Socks5RelayServiceResult {
    pub client_address: SocketAddr,
}

#[derive(Clone, Default)]
pub(crate) struct Socks5RelayService;

impl Service<Socks5RelayServiceRequest> for Socks5RelayService {
    type Response = Socks5RelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5RelayServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut write_agent_message_service =
                ServiceBuilder::new().service(WriteMessageService::default());
            let mut read_proxy_message_service =
                ServiceBuilder::new().service(ReadMessageService::new(
                    SERVER_CONFIG
                        .read_proxy_timeout_seconds()
                        .unwrap_or(DEFAULT_READ_PROXY_TIMEOUT_SECONDS),
                ));
            let mut payload_encryption_type_select_service =
                ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
            let client_stream = request.client_stream;
            let mut message_framed_read = request.message_framed_read;
            let mut message_framed_write = request.message_framed_write;
            let (mut client_stream_read_half, mut client_stream_write_half) =
                client_stream.into_split();
            let source_address_a2t = request.source_address.clone();
            let target_address_a2t = request.target_address.clone();
            let target_address_t2a = request.target_address.clone();
            tokio::spawn(async move {
                loop {
                    let mut buf = BytesMut::with_capacity(
                        SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                    );
                    let read_client_timeout_seconds = SERVER_CONFIG
                        .read_client_timeout_seconds()
                        .unwrap_or(DEFAULT_READ_CLIENT_TIMEOUT_SECONDS);
                    tokio::select! {
                        client_read_result=client_stream_read_half.read_buf(&mut buf)=>{
                            match client_read_result {
                                Err(e) => {
                                    error!(
                                        "Fail to read client data from {:#?} because of error: {:#?}",
                                        target_address_a2t, e
                                    );
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
                        }
                        _=tokio::time::sleep(Duration::from_secs(read_client_timeout_seconds))=>{
                            error!("The read client data timeout in {} seconds.", read_client_timeout_seconds);
                            return;
                        }
                    }

                    let PayloadEncryptionTypeSelectServiceResult {
                        payload_encryption_type,
                        ..
                    } = match ready_and_call_service(
                        &mut payload_encryption_type_select_service,
                        PayloadEncryptionTypeSelectServiceRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                        },
                    )
                    .await
                    {
                        Err(e) => {
                            error!(
                                "Fail to select payload encryption type because of error: {:#?}",
                                e
                            );
                            return;
                        }
                        Ok(v) => v,
                    };

                    let write_agent_message_result = ready_and_call_service(
                        &mut write_agent_message_service,
                        WriteMessageServiceRequest {
                            message_framed_write,
                            ref_id: Some(request.connect_response_message_id.clone()),
                            user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                            payload_encryption_type,
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
                            return;
                        }
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
                        .write(proxy_raw_data.as_ref())
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
