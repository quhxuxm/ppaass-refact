use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{
    fmt::{Debug, Formatter},
    io::ErrorKind,
};

use anyhow::anyhow;
use anyhow::Result;
use bytecodec::bytes::BytesEncoder;
use bytecodec::EncodeExt;
use bytes::{Bytes, BytesMut};
use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest, PayloadEncryptionTypeSelectServiceResult, PayloadType, PpaassError,
    PrepareMessageFramedService, ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceError, ReadMessageServiceRequest,
    ReadMessageServiceResult, ReadMessageServiceResultContent, RsaCryptoFetcher, WriteMessageService, WriteMessageServiceError, WriteMessageServiceRequest,
    WriteMessageServiceResult,
};
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use httpcodec::{BodyEncoder, HttpVersion, ReasonPhrase, RequestEncoder, Response, StatusCode};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};
use tower::Service;
use tower::ServiceBuilder;
use tracing::error;
use url::Url;

use crate::service::common::{ConnectToProxyService, ConnectToProxyServiceRequest, ConnectToProxyServiceResult};

use crate::{codec::http::HttpCodec, config::AgentConfig};
const DEFAULT_BUFFER_SIZE: usize = 65536;
const HTTPS_SCHEMA: &str = "https";
const SCHEMA_SEP: &str = "://";
const CONNECT_METHOD: &str = "connect";
const HTTPS_DEFAULT_PORT: u16 = 443;
const HTTP_DEFAULT_PORT: u16 = 80;
const OK_CODE: u16 = 200;
const ERROR_CODE: u16 = 400;
const ERROR_REASON: &str = " ";
const CONNECTION_ESTABLISHED: &str = "Connection Established";
const DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS: u64 = 20;

type HttpFramed<'a> = Framed<&'a mut TcpStream, HttpCodec>;

#[allow(unused)]
pub(crate) struct HttpConnectServiceRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub initial_buf: BytesMut,
    pub configuration: Arc<AgentConfig>,
}

impl Debug for HttpConnectServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HttpConnectServiceRequest: proxy_addresses={:#?}, client_address={}",
            self.proxy_addresses, self.client_address,
        )
    }
}

#[allow(unused)]
pub(crate) struct HttpConnectServiceResult<T>
where
    T: RsaCryptoFetcher,
{
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub proxy_address: Option<SocketAddr>,
    pub init_data: Option<Vec<u8>>,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct HttpConnectService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}

impl<T> HttpConnectService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }

    async fn send_error_to_client(mut client_http_framed: HttpFramed<'_>) -> Result<()> {
        let bad_request_status_code = StatusCode::new(ERROR_CODE).unwrap();
        let error_response_reason = ReasonPhrase::new(ERROR_REASON).unwrap();
        let connect_error_response = Response::new(HttpVersion::V1_1, bad_request_status_code, error_response_reason, vec![]);
        client_http_framed.send(connect_error_response).await?;
        client_http_framed.flush().await?;
        Ok(())
    }
}

impl<T> Service<HttpConnectServiceRequest> for HttpConnectService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = HttpConnectServiceResult<T>;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: HttpConnectServiceRequest) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
        Box::pin(async move {
            let mut prepare_message_framed_service = PrepareMessageFramedService::new(
                request.configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                request.configuration.compress().unwrap_or(true),
                rsa_crypto_fetcher,
            );
            let mut write_agent_message_service: WriteMessageService = Default::default();

            let mut read_proxy_message_service: ReadMessageService = Default::default();

            let mut connect_to_proxy_service =
                ConnectToProxyService::new(request.configuration.proxy_stream_so_linger().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS));
            let mut payload_encryption_type_select_service = ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
            let mut framed_parts = FramedParts::new(&mut request.client_stream, HttpCodec::default());
            framed_parts.read_buf = request.initial_buf;
            let mut http_client_framed = Framed::from_parts(framed_parts);
            let http_message = match http_client_framed.next().await {
                Some(Ok(v)) => v,
                _ => {
                    return {
                        Self::send_error_to_client(http_client_framed).await?;
                        Err(anyhow!(PpaassError::CodecError))
                    }
                },
            };
            let (request_url, init_data) = if http_message.method().to_string().to_lowercase() == CONNECT_METHOD {
                (format!("{}{}{}", HTTPS_SCHEMA, SCHEMA_SEP, http_message.request_target().to_string()), None)
            } else {
                let request_url = http_message.request_target().to_string();
                let mut http_data_encoder = RequestEncoder::<BodyEncoder<BytesEncoder>>::default();
                let encode_result = match http_data_encoder.encode_into_bytes(http_message) {
                    Err(e) => {
                        error!("Fail to encode http data because of error: {:#?} ", e);
                        Self::send_error_to_client(http_client_framed).await?;
                        return Err(anyhow!(e));
                    },
                    Ok(v) => v,
                };
                (request_url, Some(encode_result))
            };
            let parsed_request_url = Url::parse(request_url.as_str()).map_err(|e| {
                error!("Fail to parse request url because of error: {:#?}", e);
                e
            })?;
            let target_port = match parsed_request_url.port() {
                None => match parsed_request_url.scheme() {
                    HTTPS_SCHEMA => HTTPS_DEFAULT_PORT,
                    _ => HTTP_DEFAULT_PORT,
                },
                Some(v) => v,
            };
            let target_host = match parsed_request_url.host() {
                None => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(PpaassError::CodecError));
                },
                Some(v) => v.to_string(),
            };
            let target_address = NetAddress::Domain(target_host, target_port);
            let ConnectToProxyServiceResult { proxy_stream } = match ready_and_call_service(
                &mut connect_to_proxy_service,
                ConnectToProxyServiceRequest {
                    proxy_addresses: request.proxy_addresses,
                    client_address: request.client_address,
                },
            )
            .await
            {
                Err(e) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(e));
                },
                Ok(v) => v,
            };
            let framed_result = match ready_and_call_service(&mut prepare_message_framed_service, proxy_stream).await {
                Err(e) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(e));
                },
                Ok(v) => v,
            };
            let PayloadEncryptionTypeSelectServiceResult { payload_encryption_type, .. } = ready_and_call_service(
                &mut payload_encryption_type_select_service,
                PayloadEncryptionTypeSelectServiceRequest {
                    encryption_token: generate_uuid().into(),
                    user_token: request.configuration.user_token().clone().unwrap(),
                },
            )
            .await?;
            let message_framed_write = match ready_and_call_service(
                &mut write_agent_message_service,
                WriteMessageServiceRequest {
                    message_framed_write: framed_result.message_framed_write,
                    payload_encryption_type,
                    user_token: request.configuration.user_token().clone().unwrap(),
                    ref_id: None,
                    message_payload: Some(MessagePayload::new(
                        request.client_address.into(),
                        target_address,
                        PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                        Bytes::new(),
                    )),
                },
            )
            .await
            {
                Err(WriteMessageServiceError { source, .. }) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(source));
                },
                Ok(WriteMessageServiceResult { mut message_framed_write }) => {
                    if let Err(e) = message_framed_write.flush().await {
                        Self::send_error_to_client(http_client_framed).await?;
                        return Err(anyhow!(e));
                    }
                    message_framed_write
                },
            };
            match ready_and_call_service(
                &mut read_proxy_message_service,
                ReadMessageServiceRequest {
                    connection_id: request.connection_id,
                    message_framed_read: framed_result.message_framed_read,
                    read_from_address: framed_result.framed_address,
                },
            )
            .await
            {
                Err(ReadMessageServiceError { source, .. }) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(source));
                },
                Ok(ReadMessageServiceResult { content: None, .. }) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(PpaassError::IoError {
                        source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                    }));
                },
                Ok(ReadMessageServiceResult {
                    message_framed_read,
                    content:
                        Some(ReadMessageServiceResultContent {
                            message_id,
                            message_payload:
                                Some(MessagePayload {
                                    target_address,
                                    source_address,
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                                    ..
                                }),
                            ..
                        }),
                }) => {
                    if let None = init_data {
                        let http_connect_success_response = Response::new(
                            HttpVersion::V1_1,
                            StatusCode::new(OK_CODE).unwrap(),
                            ReasonPhrase::new(CONNECTION_ESTABLISHED).unwrap(),
                            vec![],
                        );
                        http_client_framed.send(http_connect_success_response).await?;
                        http_client_framed.flush().await?;
                        return Ok(HttpConnectServiceResult {
                            client_stream: request.client_stream,
                            client_address: request.client_address,
                            proxy_address: framed_result.framed_address,
                            init_data: None,
                            message_framed_read,
                            message_framed_write,
                            message_id,
                            target_address,
                            source_address,
                        });
                    }
                    return Ok(HttpConnectServiceResult {
                        client_stream: request.client_stream,
                        client_address: request.client_address,
                        proxy_address: framed_result.framed_address,
                        init_data,
                        message_framed_read,
                        message_framed_write,
                        message_id,
                        target_address,
                        source_address,
                    });
                },
                Ok(ReadMessageServiceResult { .. }) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(anyhow!(PpaassError::IoError {
                        source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                    }));
                },
            };
        })
    }
}
