use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{
    fmt::{Debug, Formatter},
    marker::PhantomData,
};

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use futures::{future::BoxFuture, SinkExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};

use tower::util::BoxCloneService;
use tower::{service_fn, Service, ServiceBuilder};
use tracing::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest, PayloadEncryptionTypeSelectServiceResult, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceError, ReadMessageServiceRequest, ReadMessageServiceResult,
    ReadMessageServiceResultContent, RsaCryptoFetcher, WriteMessageService, WriteMessageServiceError, WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::service::http::{HttpFlowRequest, HttpFlowService};
use crate::service::socks5::{Socks5FlowRequest, Socks5FlowService};
use crate::{
    codec::{Protocol, SwitchClientProtocolDecoder},
    config::AgentConfig,
};

pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;

pub const DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS: u64 = 20;

pub const DEFAULT_RATE_LIMIT: u64 = 1024;
pub const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;
pub const DEFAULT_BUFFERED_CONNECTION_NUMBER: usize = 1024;

pub(crate) struct ClientConnectionInfo {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

impl Debug for ClientConnectionInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientConnectionInfo: client_address={}", self.client_address)
    }
}

pub(crate) struct HandleClientConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub rsa_crypto_fetcher: Arc<T>,
    pub confiugration: Arc<AgentConfig>,
}

impl<T> Debug for HandleClientConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "HandleClientConnectionService: proxy_addresses={:#?}", self.proxy_addresses)
    }
}

impl<T> HandleClientConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    pub(crate) fn new(proxy_addresses: Arc<Vec<SocketAddr>>, rsa_crypto_fetcher: Arc<T>, confiugration: Arc<AgentConfig>) -> Self {
        Self {
            proxy_addresses,
            rsa_crypto_fetcher,
            confiugration,
        }
    }
}
impl<T> Service<ClientConnectionInfo> for HandleClientConnectionService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = ();
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: ClientConnectionInfo) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
        let proxy_addresses = self.proxy_addresses.clone();
        let configuration = self.confiugration.clone();
        Box::pin(async move {
            let mut socks5_flow_service = ServiceBuilder::new().service(Socks5FlowService::new(rsa_crypto_fetcher.clone()));
            let mut http_flow_service = ServiceBuilder::new().service(HttpFlowService::new(rsa_crypto_fetcher));
            let mut framed = Framed::with_capacity(
                &mut req.client_stream,
                SwitchClientProtocolDecoder,
                configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );
            return match framed.next().await {
                None => Ok(()),
                Some(Err(e)) => {
                    error!("Can not parse protocol from client input stream because of error: {:#?}.", e);
                    Err(anyhow!(e))
                },
                Some(Ok(Protocol::Http)) => {
                    let FramedParts { read_buf: buffer, .. } = framed.into_parts();
                    ready_and_call_service(
                        &mut http_flow_service,
                        HttpFlowRequest {
                            proxy_addresses,
                            client_stream: req.client_stream,
                            client_address: req.client_address,
                            buffer,
                            configuration,
                        },
                    )
                    .await?;
                    Ok(())
                },
                Some(Ok(Protocol::Socks5)) => {
                    let FramedParts { read_buf: buffer, .. } = framed.into_parts();
                    let flow_result = ready_and_call_service(
                        &mut socks5_flow_service,
                        Socks5FlowRequest {
                            proxy_addresses,
                            client_stream: req.client_stream,
                            client_address: req.client_address,
                            buffer,
                            configuration,
                        },
                    )
                    .await?;
                    debug!("Client {} complete socks5 relay", flow_result.client_address);
                    Ok(())
                },
            };
        })
    }
}

#[derive(Clone)]
pub(crate) struct ConnectToProxyServiceRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
}

impl Debug for ConnectToProxyServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ConnectToProxyServiceRequest: proxy_addresses={:#?}, client_address={}",
            self.proxy_addresses, self.client_address
        )
    }
}

#[derive(Clone)]
struct ConcreteConnectToProxyRequest {
    proxy_addresses: Arc<Vec<SocketAddr>>,
    client_address: SocketAddr,
}

impl Debug for ConcreteConnectToProxyRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ConcreteConnectToProxyRequest: proxy_addresses={:#?}, client_address={}",
            self.proxy_addresses, self.client_address
        )
    }
}
pub(crate) struct ConnectToProxyServiceResult {
    pub proxy_stream: TcpStream,
}

#[derive(Clone)]
pub(crate) struct ConnectToProxyService {
    concrete_service: BoxCloneService<ConcreteConnectToProxyRequest, ConnectToProxyServiceResult, anyhow::Error>,
}

impl ConnectToProxyService {
    pub(crate) fn new(proxy_stream_so_linger: u64) -> Self {
        let concrete_service = service_fn(move |request: ConcreteConnectToProxyRequest| async move {
            debug!("Client {}, begin connect to proxy", request.client_address);
            let proxy_stream = TcpStream::connect(request.proxy_addresses.as_slice()).await?;
            proxy_stream.set_nodelay(true)?;
            proxy_stream.set_linger(Some(Duration::from_secs(proxy_stream_so_linger)))?;
            debug!("Client {}, success connect to proxy", request.client_address);
            Ok(ConnectToProxyServiceResult { proxy_stream })
        });
        Self {
            concrete_service: BoxCloneService::new(concrete_service),
        }
    }
}

impl Service<ConnectToProxyServiceRequest> for ConnectToProxyService {
    type Response = ConnectToProxyServiceResult;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<ConnectToProxyServiceResult, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.concrete_service.poll_ready(cx)
    }

    fn call(&mut self, request: ConnectToProxyServiceRequest) -> Self::Future {
        let mut concrete_connect_service = self.concrete_service.clone();
        let connect_future = async move {
            let concrete_connect_result = ready_and_call_service(
                &mut concrete_connect_service,
                ConcreteConnectToProxyRequest {
                    proxy_addresses: request.proxy_addresses,
                    client_address: request.client_address,
                },
            )
            .await;
            return match concrete_connect_result {
                Ok(r) => Ok(r),
                Err(e) => {
                    error!("Client {} fail to connect proxy because of error: {:#?}", request.client_address, e);
                    Err(e)
                },
            };
        };
        Box::pin(connect_future)
    }
}

#[allow(unused)]
pub(crate) struct TcpRelayServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
    pub connect_response_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: Option<SocketAddr>,
    pub configuration: Arc<AgentConfig>,
}

impl<T> Debug for TcpRelayServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TcpRelayServiceRequest: client_address={}, source_address={}, target_address={}",
            self.client_address,
            self.source_address.to_string(),
            self.target_address.to_string()
        )
    }
}

#[allow(unused)]
pub(crate) struct TcpRelayServiceResult {
    pub client_address: SocketAddr,
}

#[derive(Default)]
pub(crate) struct TcpRelayService<T>
where
    T: RsaCryptoFetcher,
{
    _marker: PhantomData<T>,
}

impl<T> TcpRelayService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<T> Service<TcpRelayServiceRequest<T>> for TcpRelayService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = TcpRelayServiceResult;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: TcpRelayServiceRequest<T>) -> Self::Future {
        Box::pin(async move {
            let client_stream = request.client_stream;
            let message_framed_read = request.message_framed_read;
            let message_framed_write = request.message_framed_write;
            let source_address_a2t = request.source_address.clone();
            let target_address_a2t = request.target_address.clone();
            let target_address_t2a = request.target_address.clone();
            let client_address = request.client_address;
            let (client_stream_read_half, client_stream_write_half) = client_stream.into_split();
            let configuration_clone = request.configuration.clone();
            tokio::spawn(async move {
                if let Err((mut message_framed_write, _client_stream_read_half, original_error)) = Self::relay_client_to_proxy(
                    request.init_data,
                    request.connect_response_message_id,
                    message_framed_write,
                    source_address_a2t,
                    target_address_a2t,
                    client_stream_read_half,
                    configuration_clone,
                )
                .await
                {
                    error!("Error happen when relay data from client to proxy, error: {:#?}", original_error);
                    if let Err(e) = message_framed_write.flush().await {
                        error!("Fail to flush proxy message writer when relay data from client to proxy have error:{:#?}", e);
                    };
                    if let Err(e) = message_framed_write.close().await {
                        error!("Fail to close proxy message writer when relay data from client to proxy have error:{:#?}", e);
                    };
                }
            });
            tokio::spawn(async move {
                if let Err((_message_framed_read, mut client_stream_write_half, original_error)) = Self::relay_proxy_to_client(
                    request.proxy_address,
                    target_address_t2a,
                    message_framed_read,
                    client_stream_write_half,
                    request.configuration,
                )
                .await
                {
                    error!("Error happen when relay data from proxy to client, error: {:#?}", original_error);
                    if let Err(e) = client_stream_write_half.flush().await {
                        error!("Fail to flush client stream writer when relay data from proxy to client have error:{:#?}", e);
                    };
                    if let Err(e) = client_stream_write_half.shutdown().await {
                        error!("Fail to shutdown client stream writer when relay data from proxy to client have error:{:#?}", e);
                    };
                }
            });
            Ok(TcpRelayServiceResult { client_address })
        })
    }
}

impl<T> TcpRelayService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    async fn relay_client_to_proxy(
        init_data: Option<Vec<u8>>, connect_response_message_id: String, mut message_framed_write: MessageFramedWrite<T>, source_address_a2t: NetAddress,
        target_address_a2t: NetAddress, mut client_stream_read_half: OwnedReadHalf, configuration: Arc<AgentConfig>,
    ) -> Result<(), (MessageFramedWrite<T>, OwnedReadHalf, anyhow::Error)> {
        let mut payload_encryption_type_select_service: PayloadEncryptionTypeSelectService = Default::default();
        let mut write_agent_message_service: WriteMessageService = Default::default();
        let user_token = configuration.user_token().clone().unwrap();
        if let Some(init_data) = init_data {
            let payload_encryption_type = match ready_and_call_service(
                &mut payload_encryption_type_select_service,
                PayloadEncryptionTypeSelectServiceRequest {
                    encryption_token: generate_uuid().into(),
                    user_token: user_token.clone(),
                },
            )
            .await
            {
                Err(e) => {
                    error!("Fail to select payload encryption type because of error: {:#?}", e);
                    return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
                },
                Ok(PayloadEncryptionTypeSelectServiceResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            let write_agent_message_result = ready_and_call_service(
                &mut write_agent_message_service,
                WriteMessageServiceRequest {
                    message_framed_write,
                    ref_id: Some(connect_response_message_id.clone()),
                    user_token: configuration.user_token().clone().unwrap(),
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
            message_framed_write = match write_agent_message_result {
                Err(WriteMessageServiceError { message_framed_write, source }) => {
                    error!("Fail to write agent message to proxy because of error: {:#?}", source);
                    return Err((message_framed_write, client_stream_read_half, anyhow!(source)));
                },
                Ok(WriteMessageServiceResult { mut message_framed_write }) => {
                    if let Err(e) = message_framed_write.flush().await {
                        error!("Fail to flush agent message to proxy because of error: {:#?}", e);
                        return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
                    };
                    message_framed_write
                },
            };
        }
        loop {
            let client_buffer_size = configuration.client_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let mut client_buffer = BytesMut::with_capacity(client_buffer_size);

            match client_stream_read_half.read_buf(&mut client_buffer).await {
                Err(e) => {
                    error!(
                        "Error happen when relay data from client to proxy,  agent address={:?}, target address={:?}, error: {:#?}",
                        source_address_a2t, target_address_a2t, e
                    );
                    return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
                },
                Ok(0) => {
                    debug!("Read all data from client, target address: {:?}", target_address_a2t);
                    if let Err(e) = message_framed_write.flush().await {
                        return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
                    };
                    if let Err(e) = message_framed_write.close().await {
                        return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
                    };
                    return Ok(());
                },
                Ok(size) => {
                    debug!("Read {} bytes from client, target address: {:?}", size, target_address_a2t);
                    size
                },
            };
            let payload_encryption_type = match ready_and_call_service(
                &mut payload_encryption_type_select_service,
                PayloadEncryptionTypeSelectServiceRequest {
                    encryption_token: generate_uuid().into(),
                    user_token: user_token.clone(),
                },
            )
            .await
            {
                Err(e) => {
                    error!("Fail to select payload encryption type because of error: {:#?}", e);
                    return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
                },
                Ok(PayloadEncryptionTypeSelectServiceResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            let payload_data = client_buffer.split().freeze();
            let payload_data_chunks = payload_data.chunks(configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE));
            for (_, chunk) in payload_data_chunks.enumerate() {
                let chunk_data = Bytes::copy_from_slice(chunk);
                let write_agent_message_result = ready_and_call_service(
                    &mut write_agent_message_service,
                    WriteMessageServiceRequest {
                        message_framed_write,
                        ref_id: Some(connect_response_message_id.clone()),
                        user_token: configuration.user_token().clone().unwrap(),
                        payload_encryption_type: payload_encryption_type.clone(),
                        message_payload: Some(MessagePayload::new(
                            source_address_a2t.clone(),
                            target_address_a2t.clone(),
                            PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                            chunk_data,
                        )),
                    },
                )
                .await;
                message_framed_write = match write_agent_message_result {
                    Err(WriteMessageServiceError { message_framed_write, source }) => {
                        error!("Fail to write agent message to proxy because of error: {:#?}", source);
                        return Err((message_framed_write, client_stream_read_half, anyhow!(source)));
                    },
                    Ok(WriteMessageServiceResult { message_framed_write }) => message_framed_write,
                };
            }
            if let Err(e) = message_framed_write.flush().await {
                return Err((message_framed_write, client_stream_read_half, anyhow!(e)));
            };
        }
    }

    async fn relay_proxy_to_client(
        read_from_address: Option<SocketAddr>, _target_address_t2a: NetAddress, mut message_framed_read: MessageFramedRead<T>,
        mut client_stream_write_half: OwnedWriteHalf, configuration: Arc<AgentConfig>,
    ) -> Result<(), (MessageFramedRead<T>, OwnedWriteHalf, anyhow::Error)> {
        let mut read_proxy_message_service: ReadMessageService = Default::default();
        loop {
            let read_proxy_message_result = ready_and_call_service(
                &mut read_proxy_message_service,
                ReadMessageServiceRequest {
                    message_framed_read,
                    read_from_address,
                },
            )
            .await;
            let proxy_raw_data = match read_proxy_message_result {
                Err(ReadMessageServiceError { message_framed_read, source }) => {
                    return Err((message_framed_read, client_stream_write_half, anyhow!(source)));
                },
                Ok(ReadMessageServiceResult {
                    message_framed_read: message_framed_read_give_back,
                    content:
                        Some(ReadMessageServiceResultContent {
                            message_payload:
                                Some(MessagePayload {
                                    data,
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                                    ..
                                }),
                            ..
                        }),
                    ..
                }) => {
                    message_framed_read = message_framed_read_give_back;
                    data
                },
                Ok(ReadMessageServiceResult {
                    message_framed_read,
                    content: None,
                    ..
                }) => {
                    client_stream_write_half
                        .flush()
                        .await
                        .map_err(|e| (message_framed_read, client_stream_write_half, anyhow!(e)))?;
                    return Ok(());
                },
                Ok(ReadMessageServiceResult { message_framed_read, .. }) => {
                    return Err((message_framed_read, client_stream_write_half, anyhow!(PpaassError::CodecError)))
                },
            };

            let client_buffer_size = configuration.client_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let proxy_raw_data_chunks = proxy_raw_data.chunks(client_buffer_size);
            for (_, chunk) in proxy_raw_data_chunks.enumerate() {
                if let Err(e) = client_stream_write_half.write(chunk).await {
                    return Err((message_framed_read, client_stream_write_half, anyhow!(e)));
                }
            }
            if let Err(e) = client_stream_write_half.flush().await {
                return Err((message_framed_read, client_stream_write_half, anyhow!(e)));
            }
        }
    }
}
