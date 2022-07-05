use std::sync::Arc;
use std::{fmt::Debug, net::SocketAddr};

use std::io::ErrorKind;

use anyhow::anyhow;
use anyhow::Result;
use bytecodec::bytes::BytesEncoder;
use bytecodec::EncodeExt;
use bytes::BytesMut;

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedGenerateResult, MessageFramedGenerator, MessageFramedRead, MessageFramedReader,
    MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult,
    PayloadEncryptionTypeSelector, PayloadType, PpaassError, ProxyMessagePayloadTypeValue, ReadMessageFramedError, ReadMessageFramedRequest,
    ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};

use futures::{SinkExt, StreamExt};
use httpcodec::{BodyEncoder, HttpVersion, ReasonPhrase, RequestEncoder, Response, StatusCode};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};

use tracing::{error, instrument};
use url::Url;

use crate::{
    codec::http::HttpCodec,
    config::AgentConfig,
    service::pool::{ProxyConnection, ProxyConnectionPool},
};
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

type HttpFramed<'a> = Framed<&'a mut TcpStream, HttpCodec>;

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct HttpConnectFlowRequest<'a> {
    pub client_connection_id: &'a str,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub initial_buf: BytesMut,
}

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct HttpConnectFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub proxy_address: SocketAddr,
    pub init_data: Option<Vec<u8>>,
    pub message_framed_read: MessageFramedRead<T, ProxyConnection>,
    pub message_framed_write: MessageFramedWrite<T, ProxyConnection>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub proxy_connection_id: String,
}

pub(crate) struct HttpConnectFlow;

impl HttpConnectFlow {
    #[instrument(skip_all, fields(_client_connection_id))]
    async fn send_error_to_client(_client_connection_id: &str, mut client_http_framed: HttpFramed<'_>) -> Result<()> {
        let bad_request_status_code = StatusCode::new(ERROR_CODE).unwrap();
        let error_response_reason = ReasonPhrase::new(ERROR_REASON).unwrap();
        let connect_error_response = Response::new(HttpVersion::V1_1, bad_request_status_code, error_response_reason, vec![]);
        client_http_framed.send(connect_error_response).await?;
        client_http_framed.flush().await?;
        Ok(())
    }
}

impl HttpConnectFlow {
    pub async fn exec<'a, T>(
        request: HttpConnectFlowRequest<'a>, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<ProxyConnectionPool>,
    ) -> Result<HttpConnectFlowResult<T>>
    where
        T: RsaCryptoFetcher + Debug,
    {
        let HttpConnectFlowRequest {
            mut client_stream,
            client_address,
            initial_buf,
            client_connection_id,
            ..
        } = request;
        let mut framed_parts = FramedParts::new(&mut client_stream, HttpCodec::default());
        framed_parts.read_buf = initial_buf;
        let mut http_client_framed = Framed::from_parts(framed_parts);
        let http_message = match http_client_framed.next().await {
            Some(Ok(v)) => v,
            _ => {
                return {
                    Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
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
                    Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
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
                Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
                return Err(anyhow!(PpaassError::CodecError));
            },
            Some(v) => v.to_string(),
        };
        let target_address = NetAddress::Domain(target_host, target_port);
        let proxy_connection = match proxy_connection_pool.fetch_connection().await {
            Err(e) => {
                Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
                return Err(anyhow!(e));
            },
            Ok(v) => v,
        };
        let proxy_connection_id = proxy_connection.id.clone();
        let connected_proxy_address = proxy_connection.peer_addr()?;
        let MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
        } = MessageFramedGenerator::generate(
            proxy_connection,
            configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            configuration.compress().unwrap_or(true),
            rsa_crypto_fetcher,
        )
        .await;

        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
        })
        .await?;
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            connection_id: Some(proxy_connection_id.as_str()),
            message_framed_write,
            payload_encryption_type,
            user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
            ref_id: None,
            message_payloads: Some(vec![MessagePayload {
                source_address: Some(request.client_address.into()),
                target_address: Some(target_address),
                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                data: None,
            }]),
        })
        .await
        {
            Err(WriteMessageFramedError { source, .. }) => {
                Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
                return Err(anyhow!(source));
            },
            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
        };
        match MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: proxy_connection_id.as_str(),
            message_framed_read,
            timeout: None,
        })
        .await
        {
            Err(ReadMessageFramedError { source, .. }) => {
                Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
                return Err(anyhow!(source));
            },
            Ok(ReadMessageFramedResult { content: None, .. }) => {
                Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
                return Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                }));
            },
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_payload:
                            Some(MessagePayload {
                                target_address: Some(target_address),
                                source_address: Some(source_address),
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                                ..
                            }),
                        ..
                    }),
            }) => match init_data {
                None => {
                    let http_connect_success_response = Response::new(
                        HttpVersion::V1_1,
                        StatusCode::new(OK_CODE).unwrap(),
                        ReasonPhrase::new(CONNECTION_ESTABLISHED).unwrap(),
                        vec![],
                    );
                    http_client_framed.send(http_connect_success_response).await?;
                    http_client_framed.flush().await?;
                    return Ok(HttpConnectFlowResult {
                        client_stream,
                        client_address,
                        proxy_address: connected_proxy_address,
                        init_data: None,
                        message_framed_read,
                        message_framed_write,
                        target_address,
                        source_address,
                        proxy_connection_id,
                    });
                },
                Some(init_data) => {
                    return Ok(HttpConnectFlowResult {
                        client_stream,
                        client_address,
                        proxy_address: connected_proxy_address,
                        init_data: Some(init_data),
                        message_framed_read,
                        message_framed_write,
                        target_address,
                        source_address,
                        proxy_connection_id,
                    });
                },
            },
            Ok(ReadMessageFramedResult { .. }) => {
                Self::send_error_to_client(&client_connection_id, http_client_framed).await?;
                return Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "Invalid payload type read from proxy.",),
                }));
            },
        };
    }
}
