use std::net::SocketAddr;
use std::task::{Context, Poll};

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

use crate::service::common::DEFAULT_BUFFER_SIZE;
use crate::SERVER_CONFIG;

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

#[derive(Clone, Debug, Default)]
pub(crate) struct HttpRelayService;

impl Service<HttpRelayServiceRequest> for HttpRelayService {
    type Response = HttpRelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: HttpRelayServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut write_agent_message_service =
                ServiceBuilder::new().service(WriteMessageService::default());
            let mut read_proxy_message_service =
                ServiceBuilder::new().service(ReadMessageService::default());
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
                if let Some(init_data) = request.init_data {
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
                            return;
                        }
                        Ok(v) => message_framed_write = v.message_framed_write,
                    };
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
            Ok(HttpRelayServiceResult {
                client_address: request.client_address,
            })
        })
    }
}
