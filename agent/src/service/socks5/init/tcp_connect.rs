use std::{
    fmt::{Debug, Formatter},
    task::Context,
};
use std::{io::ErrorKind, sync::Arc};
use std::{net::SocketAddr, task::Poll};

use anyhow::anyhow;
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::SinkExt;
use tower::{Service, ServiceBuilder};
use tracing::error;

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest, PayloadEncryptionTypeSelectServiceResult, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceError, ReadMessageServiceRequest, ReadMessageServiceResult,
    ReadMessageServiceResultContent, RsaCryptoFetcher, WriteMessageService, WriteMessageServiceError, WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::{
    config::SERVER_CONFIG,
    message::socks5::Socks5Addr,
    service::common::{
        generate_prepare_message_framed_service, ConnectToProxyService, ConnectToProxyServiceRequest, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS,
        DEFAULT_RETRY_TIMES,
    },
};

pub(crate) struct Socks5TcpConnectService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}

pub(crate) struct Socks5TcpConnectServiceRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
    pub dest_address: Socks5Addr,
}

impl Debug for Socks5TcpConnectServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Socks5TcpConnectServiceRequest: proxy_addresses={:#?}, client_address={}, dest_address={}",
            self.proxy_addresses,
            self.client_address,
            self.dest_address.to_string()
        )
    }
}

pub(crate) struct Socks5TcpConnectServiceResponse<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub proxy_address: Option<SocketAddr>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub connect_response_message_id: String,
}

impl<T> Socks5TcpConnectService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }
}

impl<T> Service<Socks5TcpConnectServiceRequest> for Socks5TcpConnectService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = Socks5TcpConnectServiceResponse<T>;

    type Error = anyhow::Error;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5TcpConnectServiceRequest) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
        Box::pin(async move {
            let client_address = request.client_address;
            let mut write_agent_message_service: WriteMessageService = Default::default();
            let mut read_proxy_message_service: ReadMessageService = Default::default();
            let mut connect_to_proxy_service = ServiceBuilder::new().service(ConnectToProxyService::new(
                SERVER_CONFIG.proxy_connection_retry().unwrap_or(DEFAULT_RETRY_TIMES),
                SERVER_CONFIG.connect_proxy_timeout_seconds().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS),
            ));
            let mut payload_encryption_type_select_service = ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
            let mut prepare_message_framed_service = generate_prepare_message_framed_service(rsa_crypto_fetcher);
            let connect_to_proxy_service_result = ready_and_call_service(
                &mut connect_to_proxy_service,
                ConnectToProxyServiceRequest {
                    proxy_addresses: request.proxy_addresses,
                    client_address: request.client_address,
                },
            )
            .await?;
            let framed_result = ready_and_call_service(&mut prepare_message_framed_service, connect_to_proxy_service_result.proxy_stream).await?;
            let PayloadEncryptionTypeSelectServiceResult { payload_encryption_type, .. } = ready_and_call_service(
                &mut payload_encryption_type_select_service,
                PayloadEncryptionTypeSelectServiceRequest {
                    encryption_token: generate_uuid().into(),
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                },
            )
            .await?;
            let message_framed_write = match ready_and_call_service(
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
            .await
            {
                Err(WriteMessageServiceError { source, .. }) => {
                    return Err(anyhow!(source));
                },
                Ok(WriteMessageServiceResult { mut message_framed_write }) => {
                    if let Err(e) = message_framed_write.flush().await {
                        return Err(anyhow!(e));
                    }
                    message_framed_write
                },
            };
            match ready_and_call_service(
                &mut read_proxy_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                    read_from_address: framed_result.framed_address,
                },
            )
            .await
            {
                Err(ReadMessageServiceError { source, .. }) => Err(anyhow!(source)),
                Ok(ReadMessageServiceResult { content: None, .. }) => Err(anyhow!(PpaassError::CodecError)),
                Ok(ReadMessageServiceResult {
                    message_framed_read,
                    content:
                        Some(ReadMessageServiceResultContent {
                            message_id,
                            message_payload:
                                Some(MessagePayload {
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                                    source_address,
                                    target_address,
                                    ..
                                }),
                            ..
                        }),
                }) => Ok(Socks5TcpConnectServiceResponse {
                    client_address,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    target_address,
                    connect_response_message_id: message_id,
                    proxy_address: framed_result.framed_address,
                }),
                Ok(ReadMessageServiceResult {
                    content:
                        Some(ReadMessageServiceResultContent {
                            message_payload:
                                Some(MessagePayload {
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                                    ..
                                }),
                            ..
                        }),
                    ..
                }) => Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::ConnectionReset, "Fail connect to target from proxy",),
                })),
                Ok(ReadMessageServiceResult { .. }) => {
                    error!("Invalid payload type read from proxy.");
                    Err(anyhow!(PpaassError::IoError {
                        source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                    }))
                },
            }
        })
    }
}
