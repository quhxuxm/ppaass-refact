use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytecodec::bytes::BytesEncoder;
use bytecodec::EncodeExt;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use httpcodec::{BodyEncoder, HttpVersion, ReasonPhrase, RequestEncoder, Response, StatusCode};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::util::BoxCloneService;
use tower::Service;
use tracing::error;
use url::Url;

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType,
    PayloadType, PrepareMessageFramedResult, PrepareMessageFramedService,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, WriteMessageService, WriteMessageServiceRequest,
    WriteMessageServiceResult,
};

use crate::codec::http::HttpCodec;
use crate::config::{AGENT_PRIVATE_KEY, PROXY_PUBLIC_KEY};
use crate::service::common::{
    ConnectToProxyService, ConnectToProxyServiceRequest, ConnectToProxyServiceResult,
};
use crate::SERVER_CONFIG;

const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
const DEFAULT_MAX_FRAME_SIZE: usize = DEFAULT_BUFFER_SIZE * 2;
const DEFAULT_RETRY_TIMES: u16 = 3;

const HTTPS_SCHEMA: &str = "https";
const SCHEMA_SEP: &str = "://";
const CONNECT_METHOD: &str = "connect";
const HTTPS_DEFAULT_PORT: u16 = 443;
const HTTP_DEFAULT_PORT: u16 = 80;
const OK_CODE: u16 = 200;
const ERROR_CODE: u16 = 400;
const ERROR_REASON: &str = " ";
const CONNECTION_ESTABLISHED: &str = "Connection Established";

type HttpFramed<'a> = Framed<&'a mut TcpStream, HttpCodec>;

pub(crate) struct HttpConnectServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
pub(crate) struct HttpConnectServiceResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub init_data: Option<Vec<u8>>,
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub proxy_address_string: String,
}
#[derive(Clone, Debug)]
pub(crate) struct HttpConnectService {
    connect_to_proxy_service:
        BoxCloneService<ConnectToProxyServiceRequest, ConnectToProxyServiceResult, CommonError>,
    prepare_message_framed_service:
        BoxCloneService<TcpStream, PrepareMessageFramedResult, CommonError>,
    write_agent_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
    read_proxy_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
}

impl Default for HttpConnectService {
    fn default() -> Self {
        Self {
            prepare_message_framed_service: BoxCloneService::new(PrepareMessageFramedService::new(
                &(*PROXY_PUBLIC_KEY),
                &(*AGENT_PRIVATE_KEY),
                SERVER_CONFIG
                    .max_frame_size()
                    .unwrap_or(DEFAULT_MAX_FRAME_SIZE),
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                SERVER_CONFIG.compress().unwrap_or(true),
            )),
            write_agent_message_service: BoxCloneService::new::<WriteMessageService>(
                Default::default(),
            ),
            read_proxy_message_service: BoxCloneService::new::<ReadMessageService>(
                Default::default(),
            ),
            connect_to_proxy_service: BoxCloneService::new(ConnectToProxyService::new(
                SERVER_CONFIG
                    .proxy_connection_retry()
                    .unwrap_or(DEFAULT_RETRY_TIMES),
            )),
        }
    }
}

impl HttpConnectService {
    async fn send_error_to_client(
        mut client_http_framed: HttpFramed<'_>,
    ) -> Result<(), CommonError> {
        let bad_request_status_code = StatusCode::new(ERROR_CODE).unwrap();
        let error_response_reason = ReasonPhrase::new(ERROR_REASON).unwrap();
        let connect_error_response = Response::new(
            HttpVersion::V1_1,
            bad_request_status_code,
            error_response_reason,
            vec![],
        );
        client_http_framed.send(connect_error_response).await?;
        client_http_framed.flush().await?;
        Ok(())
    }
}

impl Service<HttpConnectServiceRequest> for HttpConnectService {
    type Response = HttpConnectServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let prepare_message_framed_service_ready =
            self.prepare_message_framed_service.poll_ready(cx);
        let write_agent_message_service_ready = self.write_agent_message_service.poll_ready(cx);
        let read_proxy_message_service_ready = self.read_proxy_message_service.poll_ready(cx);
        let connect_to_proxy_service_ready = self.connect_to_proxy_service.poll_ready(cx);
        if prepare_message_framed_service_ready.is_ready()
            && write_agent_message_service_ready.is_ready()
            && read_proxy_message_service_ready.is_ready()
            && connect_to_proxy_service_ready.is_ready()
        {
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }

    fn call(&mut self, mut request: HttpConnectServiceRequest) -> Self::Future {
        let mut prepare_message_framed_service = self.prepare_message_framed_service.clone();
        let mut write_agent_message_service = self.write_agent_message_service.clone();
        let mut read_proxy_message_service = self.read_proxy_message_service.clone();
        let mut connect_to_proxy_service = self.connect_to_proxy_service.clone();
        Box::pin(async move {
            let mut http_client_framed = Framed::with_capacity(
                &mut request.client_stream,
                HttpCodec::default(),
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );
            let http_message = match http_client_framed.next().await {
                Some(Ok(v)) => v,
                _ => {
                    return {
                        Self::send_error_to_client(http_client_framed).await?;
                        Err(CommonError::CodecError)
                    }
                }
            };
            let (request_url, init_data) = if http_message.method().to_string().to_lowercase()
                == CONNECT_METHOD
            {
                (
                    format!(
                        "{}{}{}",
                        HTTPS_SCHEMA,
                        SCHEMA_SEP,
                        http_message.request_target().to_string()
                    ),
                    None,
                )
            } else {
                let request_url = http_message.request_target().to_string();
                let mut http_data_encoder = RequestEncoder::<BodyEncoder<BytesEncoder>>::default();
                let encode_result = match http_data_encoder.encode_into_bytes(http_message) {
                    Err(e) => {
                        error!("Fail to encode http data because of error: {:#?} ", e);
                        Self::send_error_to_client(http_client_framed).await?;
                        return Err(CommonError::CodecError);
                    }
                    Ok(v) => v,
                };
                (request_url, Some(encode_result))
            };
            let parsed_request_url = Url::parse(request_url.as_str()).map_err(|e| {
                error!("Fail to parse request url because of error: {:#?}", e);
                CommonError::CodecError
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
                    return Err(CommonError::CodecError);
                }
                Some(v) => v.to_string(),
            };
            let target_address = NetAddress::Domain(target_host, target_port);
            let ConnectToProxyServiceResult {
                connected_proxy_address,
                proxy_stream,
            } = match ready_and_call_service(
                &mut connect_to_proxy_service,
                ConnectToProxyServiceRequest {
                    proxy_address: None,
                    client_address: request.client_address,
                },
            )
            .await
            {
                Err(e) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(e);
                }
                Ok(v) => v,
            };
            let framed_result =
                match ready_and_call_service(&mut prepare_message_framed_service, proxy_stream)
                    .await
                {
                    Err(e) => {
                        Self::send_error_to_client(http_client_framed).await?;
                        return Err(e);
                    }
                    Ok(v) => v,
                };
            let WriteMessageServiceResult {
                message_framed_write,
            } = match ready_and_call_service(
                &mut write_agent_message_service,
                WriteMessageServiceRequest {
                    message_framed_write: framed_result.message_framed_write,
                    payload_encryption_type: PayloadEncryptionType::Blowfish(
                        generate_uuid().into(),
                    ),
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
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
                Err(e) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(e);
                }
                Ok(v) => v,
            };
            let read_tcp_connect_response_result = match ready_and_call_service(
                &mut read_proxy_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                },
            )
            .await
            {
                Err(e) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(e);
                }
                Ok(Some(v)) => v,
                Ok(_) => {
                    Self::send_error_to_client(http_client_framed).await?;
                    return Err(CommonError::OtherError);
                }
            };
            if let ReadMessageServiceResult {
                message_payload:
                    MessagePayload {
                        target_address,
                        source_address,
                        payload_type:
                            PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                        ..
                    },
                message_framed_read,
                message_id,
                ..
            } = read_tcp_connect_response_result
            {
                if let None = init_data {
                    let http_connect_success_response = Response::new(
                        HttpVersion::V1_1,
                        StatusCode::new(OK_CODE).unwrap(),
                        ReasonPhrase::new(CONNECTION_ESTABLISHED).unwrap(),
                        vec![],
                    );
                    http_client_framed
                        .send(http_connect_success_response)
                        .await?;
                    http_client_framed.flush().await?;
                    return Ok(HttpConnectServiceResult {
                        client_stream: request.client_stream,
                        client_address: request.client_address,
                        init_data: None,
                        message_framed_read,
                        message_framed_write,
                        message_id,
                        target_address,
                        source_address,
                        proxy_address_string: connected_proxy_address,
                    });
                }
                return Ok(HttpConnectServiceResult {
                    client_stream: request.client_stream,
                    client_address: request.client_address,
                    init_data,
                    message_framed_read,
                    message_framed_write,
                    message_id,
                    target_address,
                    source_address,
                    proxy_address_string: connected_proxy_address,
                });
            };
            Self::send_error_to_client(http_client_framed).await?;
            return Err(CommonError::OtherError);
        })
    }
}
