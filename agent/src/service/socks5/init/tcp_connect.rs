use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use std::{net::SocketAddr, task::Poll};

use bytes::Bytes;
use futures::future::BoxFuture;
use tower::{Service, ServiceBuilder};
use tracing::error;

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest,
    PayloadEncryptionTypeSelectServiceResult, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageService, ReadMessageServiceRequest, ReadMessageServiceResult, WriteMessageService,
    WriteMessageServiceRequest,
};

use crate::{
    config::SERVER_CONFIG,
    message::socks5::Socks5Addr,
    service::common::{
        generate_prepare_message_framed_service, ConnectToProxyService,
        ConnectToProxyServiceRequest, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS,
        DEFAULT_READ_PROXY_TIMEOUT_SECONDS, DEFAULT_RETRY_TIMES,
    },
};

pub(crate) struct Socks5TcpConnectService;

pub(crate) struct Socks5TcpConnectServiceRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
    pub dest_address: Socks5Addr,
}

impl Debug for Socks5TcpConnectServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Socks5TcpConnectServiceRequest: proxy_addresses={:#?}, client_address={}, dest_address={}", 
        self.proxy_addresses, self.client_address, self.dest_address.to_string())
    }
}

pub(crate) struct Socks5TcpConnectServiceResponse {
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub client_address: SocketAddr,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub connect_response_message_id: String,
}

impl Default for Socks5TcpConnectService {
    fn default() -> Self {
        Self {}
    }
}

impl Service<Socks5TcpConnectServiceRequest> for Socks5TcpConnectService {
    type Response = Socks5TcpConnectServiceResponse;

    type Error = CommonError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5TcpConnectServiceRequest) -> Self::Future {
        Box::pin(async move {
            let client_address = request.client_address;
            let mut write_agent_message_service =
                ServiceBuilder::new().service(WriteMessageService::default());
            let mut read_proxy_message_service =
                ServiceBuilder::new().service(ReadMessageService::new(
                    SERVER_CONFIG
                        .read_proxy_timeout_seconds()
                        .unwrap_or(DEFAULT_READ_PROXY_TIMEOUT_SECONDS),
                ));
            let mut connect_to_proxy_service =
                ServiceBuilder::new().service(ConnectToProxyService::new(
                    SERVER_CONFIG
                        .proxy_connection_retry()
                        .unwrap_or(DEFAULT_RETRY_TIMES),
                    SERVER_CONFIG
                        .connect_proxy_timeout_seconds()
                        .unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS),
                ));
            let mut payload_encryption_type_select_service =
                ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
            let mut prepare_message_framed_service = generate_prepare_message_framed_service();
            let connect_to_proxy_service_result = ready_and_call_service(
                &mut connect_to_proxy_service,
                ConnectToProxyServiceRequest {
                    proxy_addresses: request.proxy_addresses,
                    client_address: request.client_address,
                },
            )
            .await?;
            let framed_result = ready_and_call_service(
                &mut prepare_message_framed_service,
                connect_to_proxy_service_result.proxy_stream,
            )
            .await?;
            let PayloadEncryptionTypeSelectServiceResult {
                payload_encryption_type,
                ..
            } = ready_and_call_service(
                &mut payload_encryption_type_select_service,
                PayloadEncryptionTypeSelectServiceRequest {
                    encryption_token: generate_uuid().into(),
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                },
            )
            .await?;
            let write_message_result = ready_and_call_service(
                &mut write_agent_message_service,
                WriteMessageServiceRequest {
                    message_framed_write: framed_result.message_framed_write,
                    payload_encryption_type,
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                    ref_id: None,
                    message_payload: Some(MessagePayload::new(
                        request.client_address.into(),
                        request.dest_address.clone().into(),
                        PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                        Bytes::new(),
                    )),
                },
            )
            .await?;
            match ready_and_call_service(
                &mut read_proxy_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                },
            )
            .await?
            {
                None => {
                    error!("Nothing read from proxy.");
                    Err(CommonError::CodecError)
                },
                Some(ReadMessageServiceResult {
                    message_payload:
                        MessagePayload {
                            payload_type:
                                PayloadType::ProxyPayload(
                                    ProxyMessagePayloadTypeValue::TcpConnectSuccess,
                                ),
                            source_address,
                            target_address,
                            ..
                        },
                    message_framed_read,
                    message_id,
                    ..
                }) => Ok(Socks5TcpConnectServiceResponse {
                    client_address,
                    message_framed_read,
                    message_framed_write: write_message_result.message_framed_write,
                    source_address,
                    target_address,
                    connect_response_message_id: message_id,
                }),
                Some(ReadMessageServiceResult {
                    message_payload:
                        MessagePayload {
                            payload_type:
                                PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                            ..
                        },
                    ..
                }) => {
                    error!("Fail connect to target from proxy.");
                    Err(CommonError::UnknownError)
                },
                Some(_) => {
                    error!("Invalid payload type read from proxy.");
                    Err(CommonError::UnknownError)
                },
            }
        })
    }
}
