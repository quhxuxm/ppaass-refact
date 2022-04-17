use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::future::BoxFuture;
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

use crate::service::common::{
    generate_prepare_message_framed_service, ConnectToProxyService, ConnectToProxyServiceRequest,
    DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS, DEFAULT_READ_PROXY_TIMEOUT_SECONDS, DEFAULT_RETRY_TIMES,
};
use crate::SERVER_CONFIG;

pub(crate) struct Socks5UdpAssociateServiceRequest {
    pub client_address: SocketAddr,
}

#[allow(unused)]
pub(crate) struct Socks5UdpAssociateServiceResponse {
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub proxy_address_string: String,
    pub connect_response_message_id: String,
}
pub(crate) struct Socks5UdpAssociateService;

impl Default for Socks5UdpAssociateService {
    fn default() -> Self {
        Self {}
    }
}

impl Service<Socks5UdpAssociateServiceRequest> for Socks5UdpAssociateService {
    type Response = Socks5UdpAssociateServiceResponse;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5UdpAssociateServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut connect_to_proxy_service =
                ServiceBuilder::new().service(ConnectToProxyService::new(
                    SERVER_CONFIG
                        .proxy_connection_retry()
                        .unwrap_or(DEFAULT_RETRY_TIMES),
                    SERVER_CONFIG
                        .connect_proxy_timeout_seconds()
                        .unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS),
                ));
            let connect_to_proxy_service_result = ready_and_call_service(
                &mut connect_to_proxy_service,
                ConnectToProxyServiceRequest {
                    proxy_address: None,
                    client_address: request.client_address,
                },
            )
            .await?;
            let mut prepare_message_framed_service = generate_prepare_message_framed_service();
            let framed_result = ready_and_call_service(
                &mut prepare_message_framed_service,
                connect_to_proxy_service_result.proxy_stream,
            )
            .await?;

            let mut payload_encryption_type_select_service =
                ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
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

            let mut write_agent_message_service =
                ServiceBuilder::new().service(WriteMessageService::default());
            let write_message_result = ready_and_call_service(
                &mut write_agent_message_service,
                WriteMessageServiceRequest {
                    message_framed_write: framed_result.message_framed_write,
                    payload_encryption_type,
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                    ref_id: None,
                    message_payload: Some(MessagePayload::new(
                        request.client_address.into(),
                        NetAddress::default(),
                        PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate),
                        Bytes::new(),
                    )),
                },
            )
            .await?;
            let mut read_proxy_message_service =
                ServiceBuilder::new().service(ReadMessageService::new(
                    SERVER_CONFIG
                        .read_proxy_timeout_seconds()
                        .unwrap_or(DEFAULT_READ_PROXY_TIMEOUT_SECONDS),
                ));
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
                                    ProxyMessagePayloadTypeValue::UdpAssociateSuccess,
                                ),
                            source_address,
                            target_address,
                            ..
                        },
                    message_framed_read,
                    message_id,
                    ..
                }) => Ok(Socks5UdpAssociateServiceResponse {
                    message_framed_read,
                    message_framed_write: write_message_result.message_framed_write,
                    source_address,
                    target_address,
                    proxy_address_string: connect_to_proxy_service_result.connected_proxy_address,
                    connect_response_message_id: message_id,
                }),
                Some(ReadMessageServiceResult {
                    message_payload:
                        MessagePayload {
                            payload_type:
                                PayloadType::ProxyPayload(
                                    ProxyMessagePayloadTypeValue::UdpAssociateFail,
                                ),
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
