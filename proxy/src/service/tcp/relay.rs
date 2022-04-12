use std::task::{Context, Poll};
use std::{net::SocketAddr, time::Duration};

use bytes::BytesMut;
use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::sleep,
};
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

use crate::config::DEFAULT_READ_AGENT_TIMEOUT_SECONDS;
use crate::SERVER_CONFIG;

const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;
const DEFAULT_READ_TARGET_TIMEOUT_SECONDS: u64 = 20;
#[allow(unused)]
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

#[allow(unused)]
pub(crate) struct TcpRelayServiceResult {
    pub agent_address: SocketAddr,
    pub target_address: NetAddress,
}

#[derive(Clone, Default)]
pub(crate) struct TcpRelayService;

impl Service<TcpRelayServiceRequest> for TcpRelayService {
    type Response = TcpRelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpRelayServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut message_framed_read = req.message_framed_read;
            let mut message_framed_write = req.message_framed_write;
            let target_stream = req.target_stream;
            let agent_tcp_connect_message_id = req.agent_tcp_connect_message_id;
            let user_token = req.user_token;
            let agent_connect_message_source_address = req.source_address;
            let agent_connect_message_target_address = req.target_address;
            let target_address_for_return = agent_connect_message_target_address.clone();
            let (mut target_stream_read, mut target_stream_write) = target_stream.into_split();
            tokio::spawn(async move {
                let mut read_agent_message_service =
                    ServiceBuilder::new().service(ReadMessageService::new(
                        SERVER_CONFIG
                            .read_agent_timeout_seconds()
                            .unwrap_or(DEFAULT_READ_AGENT_TIMEOUT_SECONDS),
                    ));
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
                        Ok(None) => {
                            debug!("Read all data from agent: {:#?}", req.agent_address);
                            return;
                        },
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
                            error!("Invalid payload type from agent: {:#?}", req.agent_address);
                            return;
                        },
                        Err(e) => {
                            error!("Fail to read from agent because of error: {:#?}", e);
                            return;
                        },
                    };
                    if let Err(e) = target_stream_write.write(data.as_ref()).await {
                        error!(
                            "Fail to write from agent to target because of error: {:#?}",
                            e
                        );
                        let _ = target_stream_write.shutdown().await;
                        return;
                    };
                    if let Err(e) = target_stream_write.flush().await {
                        error!(
                            "Fail to flush from agent to target because of error: {:#?}",
                            e
                        );
                        let _ = target_stream_write.shutdown().await;
                        return;
                    };
                    message_framed_read = message_framed_read_from_read_agent_result;
                }
            });
            tokio::spawn(async move {
                let mut write_proxy_message_service =
                    ServiceBuilder::new().service(WriteMessageService::default());
                let mut payload_encryption_type_select_service =
                    ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
                loop {
                    let read_target_data_future = async move {
                        let mut buf = BytesMut::with_capacity(
                            SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                        );
                        let read_size = match target_stream_read.read_buf(&mut buf).await {
                            Err(e) => {
                                error!("Fail to read data from target because of error: {:#?}", e);
                                return Err(CommonError::IoError { source: e });
                            },
                            Ok(0) => {
                                return Ok(None);
                            },
                            Ok(size) => {
                                debug!("Read {} bytes from target.", size);
                                size
                            },
                        };
                        Ok(Some((buf.freeze(), target_stream_read, read_size)))
                    };
                    let read_target_data_future_result = tokio::select! {
                        future_result = read_target_data_future => {
                            future_result
                        }
                        _ =  sleep(Duration::from_secs(SERVER_CONFIG.read_target_timeout_seconds().unwrap_or(DEFAULT_READ_TARGET_TIMEOUT_SECONDS))) => {
                            error!("The read target data timeout.");
                            Err(CommonError::TimeoutError)
                        }
                    };
                    let (buf, inner_target_stream_read, _read_size) =
                        match read_target_data_future_result {
                            Ok(None) => {
                                debug!(
                                    "Nothing to read from target, return from read target future."
                                );
                                return;
                            },
                            Ok(Some(v)) => v,
                            Err(e) => {
                                error!("Fail to read target data because of error: {:#?}", e);
                                return;
                            },
                        };
                    let proxy_message_payload = MessagePayload::new(
                        agent_connect_message_source_address.clone(),
                        agent_connect_message_target_address.clone(),
                        PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                        buf,
                    );
                    let PayloadEncryptionTypeSelectServiceResult {
                        payload_encryption_type,
                        ..
                    } = match ready_and_call_service(
                        &mut payload_encryption_type_select_service,
                        PayloadEncryptionTypeSelectServiceRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token.clone(),
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
                        },
                        Ok(v) => v,
                    };
                    let write_proxy_message_result = ready_and_call_service(
                        &mut write_proxy_message_service,
                        WriteMessageServiceRequest {
                            message_framed_write,
                            ref_id: Some(agent_tcp_connect_message_id.clone()),
                            user_token: user_token.clone(),
                            payload_encryption_type,
                            message_payload: Some(proxy_message_payload),
                        },
                    )
                    .await;
                    match write_proxy_message_result {
                        Err(e) => {
                            error!("Fail to read from target because of error(ready): {:#?}", e);
                            return;
                        },
                        Ok(proxy_message_write_result) => {
                            message_framed_write = proxy_message_write_result.message_framed_write;
                            target_stream_read = inner_target_stream_read;
                        },
                    }
                }
            });
            Ok(TcpRelayServiceResult {
                target_address: target_address_for_return,
                agent_address: req.agent_address,
            })
        })
    }
}
