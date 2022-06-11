use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::future::BoxFuture;
use tower::{Service, ServiceBuilder};
use tracing::error;

use common::{
    AgentMessagePayloadTypeValue, generate_uuid, MessageFramedRead, MessageFramedWrite,
    MessagePayload, NetAddress, PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest,
    PayloadEncryptionTypeSelectServiceResult, PayloadType,
    PpaassError, ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, ready_and_call_service, RsaCryptoFetcher, WriteMessageService,
    WriteMessageServiceRequest,
};

use crate::SERVER_CONFIG;
use crate::service::common::{
    ConnectToProxyService, ConnectToProxyServiceRequest, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS,
    DEFAULT_READ_PROXY_TIMEOUT_SECONDS, DEFAULT_RETRY_TIMES, generate_prepare_message_framed_service,
};

#[derive(Debug)]
pub(crate) struct Socks5UdpAssociateServiceRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
}

#[allow(unused)]
pub(crate) struct Socks5UdpAssociateServiceResponse<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub connect_response_message_id: String,
    pub proxy_address: Option<SocketAddr>,
}
pub(crate) struct Socks5UdpAssociateService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}

impl<T> Socks5UdpAssociateService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }
}

impl<T> Service<Socks5UdpAssociateServiceRequest> for Socks5UdpAssociateService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = Socks5UdpAssociateServiceResponse<T>;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5UdpAssociateServiceRequest) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
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
                    proxy_addresses: request.proxy_addresses,
                    client_address: request.client_address,
                },
            )
                .await?;
            let mut prepare_message_framed_service =
                generate_prepare_message_framed_service(rsa_crypto_fetcher);
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
                    read_from_address: framed_result.framed_address,
                },
            )
                .await?
            {
                None => {
                    error!("Nothing read from proxy.");
                    Err(PpaassError::CodecError)
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
                    connect_response_message_id: message_id,
                    proxy_address: framed_result.framed_address,
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
                    Err(PpaassError::UnknownError)
                },
                Some(_) => {
                    error!("Invalid payload type read from proxy.");
                    Err(PpaassError::UnknownError)
                },
            }
        })
    }
}
