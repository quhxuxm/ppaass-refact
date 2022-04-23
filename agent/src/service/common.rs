use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use futures_util::future;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::retry::{Policy, Retry};
use tower::util::BoxCloneService;
use tower::{service_fn, Service, ServiceBuilder};
use tracing::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest,
    PayloadEncryptionTypeSelectServiceResult, PayloadType, PrepareMessageFramedService,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, WriteMessageService, WriteMessageServiceRequest,
};

use crate::config::{AGENT_PRIVATE_KEY, PROXY_PUBLIC_KEY, SERVER_CONFIG};
use crate::service::http::{HttpFlowRequest, HttpFlowService};
use crate::service::socks5::{Socks5FlowRequest, Socks5FlowService};

const SOCKS5_PROTOCOL_FLAG: u8 = 5;
const SOCKS4_PROTOCOL_FLAG: u8 = 4;
pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
pub const DEFAULT_MAX_FRAME_SIZE: usize = DEFAULT_BUFFER_SIZE * 2;
pub const DEFAULT_RETRY_TIMES: u16 = 3;
pub const DEFAULT_READ_PROXY_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_READ_CLIENT_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_RATE_LIMIT: u64 = 1024;
pub const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;
pub const DEFAULT_BUFFERED_CONNECTION_NUMBER: usize = 1024;
#[derive(Debug)]
pub(crate) struct ClientConnectionInfo {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

#[derive(Debug, Default)]
pub(crate) struct HandleClientConnectionService;

impl Service<ClientConnectionInfo> for HandleClientConnectionService {
    type Response = ();
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ClientConnectionInfo) -> Self::Future {
        Box::pin(async move {
            let mut socks5_flow_service =
                ServiceBuilder::new().service(Socks5FlowService::default());
            let mut http_flow_service = ServiceBuilder::new().service(HttpFlowService::default());
            let mut protocol_buf: [u8; 1] = [0];
            let peek_result = req.client_stream.peek(&mut protocol_buf).await;
            let protocol = match peek_result {
                Err(e) => {
                    error!(
                        "Fail to peek protocol from client stream because of error: {:#?}",
                        e
                    );
                    return Err(CommonError::IoError { source: e });
                },
                Ok(1) => protocol_buf[0],
                Ok(protocol_flag) => {
                    error!(
                        "Fail to peek protocol from client stream because of unknown protocol flag: {}",
                        protocol_flag
                    );
                    return Err(CommonError::CodecError);
                },
            };
            if protocol == SOCKS4_PROTOCOL_FLAG {
                error!("Can not support socks4 protocol.");
                return Err(CommonError::CodecError);
            }
            if protocol == SOCKS5_PROTOCOL_FLAG {
                debug!("Incoming request is for socks5 protocol.");
                let flow_result = ready_and_call_service(
                    &mut socks5_flow_service,
                    Socks5FlowRequest {
                        client_stream: req.client_stream,
                        client_address: req.client_address,
                    },
                )
                .await?;
                debug!(
                    "Client {} complete socks5 relay",
                    flow_result.client_address
                );
                return Ok(());
            }
            debug!("Incoming request is for http protocol.");
            let _flow_result = ready_and_call_service(
                &mut http_flow_service,
                HttpFlowRequest {
                    client_stream: req.client_stream,
                    client_address: req.client_address,
                },
            )
            .await?;
            Ok(())
        })
    }
}

#[derive(Clone)]
pub(crate) struct ConnectToProxyServiceRequest {
    pub proxy_address: Option<String>,
    pub client_address: SocketAddr,
}

#[derive(Clone)]
struct ConcreteConnectToProxyRequest {
    proxy_address: String,
    client_address: SocketAddr,
}

pub(crate) struct ConnectToProxyServiceResult {
    pub proxy_stream: TcpStream,
    pub connected_proxy_address: String,
}

#[derive(Clone)]
struct ConnectToProxyAttempts {
    retry: u16,
}

#[derive(Clone)]
pub(crate) struct ConnectToProxyService {
    concrete_service:
        BoxCloneService<ConcreteConnectToProxyRequest, ConnectToProxyServiceResult, CommonError>,
}

impl ConnectToProxyService {
    pub(crate) fn new(retry: u16, connect_timeout_seconds: u64) -> Self {
        let concrete_service = Retry::new(
            ConnectToProxyAttempts { retry },
            service_fn(move |request: ConcreteConnectToProxyRequest| async move {
                debug!(
                    "Client {}, begin connect to proxy: {}",
                    request.client_address, request.proxy_address
                );
                let proxy_stream = match tokio::time::timeout(
                    Duration::from_secs(connect_timeout_seconds),
                    TcpStream::connect(&request.proxy_address),
                )
                .await
                {
                    Err(e) => {
                        error!("The connect to proxy timeout: {:#?}.", e);
                        return Err(CommonError::TimeoutError);
                    },
                    Ok(Err(e)) => {
                        error!(
                            "Fail connect to proxy {} because of error: {:#?}",
                            &request.proxy_address, e
                        );
                        return Err(CommonError::IoError { source: e });
                    },
                    Ok(Ok(v)) => v,
                };
                proxy_stream
                    .set_nodelay(true)
                    .map_err(|e| CommonError::IoError { source: e })?;
                proxy_stream
                    .set_linger(None)
                    .map_err(|e| CommonError::IoError { source: e })?;
                debug!(
                    "Client {}, success connect to proxy: {}",
                    request.client_address, request.proxy_address
                );
                Ok(ConnectToProxyServiceResult {
                    proxy_stream,
                    connected_proxy_address: request.proxy_address,
                })
            }),
        );
        Self {
            concrete_service: BoxCloneService::new(concrete_service),
        }
    }
}

impl Policy<ConcreteConnectToProxyRequest, ConnectToProxyServiceResult, CommonError>
    for ConnectToProxyAttempts
{
    type Future = futures_util::future::Ready<Self>;

    fn retry(
        &self,
        _req: &ConcreteConnectToProxyRequest,
        result: Result<&ConnectToProxyServiceResult, &CommonError>,
    ) -> Option<Self::Future> {
        match result {
            Ok(_) => {
                // Treat all `Response`s as success,
                // so don't retry...
                None
            },
            Err(_) => {
                // Treat all errors as failures...
                // But we limit the number of attempts...
                if self.retry > 0 {
                    // Try again!
                    return Some(future::ready(ConnectToProxyAttempts {
                        retry: self.retry - 1,
                    }));
                }
                // Used all our attempts, no retry...
                None
            },
        }
    }

    fn clone_request(
        &self,
        req: &ConcreteConnectToProxyRequest,
    ) -> Option<ConcreteConnectToProxyRequest> {
        Some(req.clone())
    }
}

impl Service<ConnectToProxyServiceRequest> for ConnectToProxyService {
    type Response = ConnectToProxyServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<ConnectToProxyServiceResult, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.concrete_service.poll_ready(cx)
    }

    fn call(&mut self, request: ConnectToProxyServiceRequest) -> Self::Future {
        let proxy_addresses = SERVER_CONFIG
            .proxy_addresses()
            .as_ref()
            .expect("No proxy addresses configuration item")
            .clone();
        let mut concrete_connect_service = self.concrete_service.clone();
        let connect_future = async move {
            if let Some(proxy_address) = request.proxy_address {
                return ready_and_call_service(
                    &mut concrete_connect_service,
                    ConcreteConnectToProxyRequest {
                        proxy_address,
                        client_address: request.client_address,
                    },
                )
                .await;
            }
            for address in proxy_addresses.into_iter() {
                let concrete_connect_result = ready_and_call_service(
                    &mut concrete_connect_service,
                    ConcreteConnectToProxyRequest {
                        proxy_address: address.clone(),
                        client_address: request.client_address,
                    },
                )
                .await;
                match concrete_connect_result {
                    Ok(r) => return Ok(r),
                    Err(e) => {
                        error!(
                            "Client {} fail to connect proxy: {} because of error: {:#?}",
                            request.client_address, address, e
                        );
                        continue;
                    },
                }
            }
            Err(CommonError::IoError {
                source: std::io::Error::new(
                    ErrorKind::NotConnected,
                    "No proxy address is connnectable.",
                ),
            })
        };
        Box::pin(connect_future)
    }
}

pub fn generate_prepare_message_framed_service() -> PrepareMessageFramedService {
    ServiceBuilder::new().service(PrepareMessageFramedService::new(
        &(*PROXY_PUBLIC_KEY),
        &(*AGENT_PRIVATE_KEY),
        SERVER_CONFIG
            .max_frame_size()
            .unwrap_or(DEFAULT_MAX_FRAME_SIZE),
        SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
        SERVER_CONFIG.compress().unwrap_or(true),
    ))
}

#[allow(unused)]
pub(crate) struct RelayServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite,
    pub message_framed_read: MessageFramedRead,
    pub connect_response_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
}

#[allow(unused)]
pub(crate) struct RelayServiceResult {
    pub client_address: SocketAddr,
}

#[derive(Clone, Default)]
pub(crate) struct RelayService;

impl Service<RelayServiceRequest> for RelayService {
    type Response = RelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: RelayServiceRequest) -> Self::Future {
        Box::pin(async move {
            let client_stream = request.client_stream;
            let mut message_framed_read = request.message_framed_read;
            let mut message_framed_write = request.message_framed_write;
            let source_address_a2t = request.source_address.clone();
            let target_address_a2t = request.target_address.clone();
            let target_address_t2a = request.target_address.clone();
            let (mut client_stream_read_half, mut client_stream_write_half) =
                client_stream.into_split();
            tokio::spawn(async move {
                let mut payload_encryption_type_select_service =
                    ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
                let mut write_agent_message_service =
                    ServiceBuilder::new().service(WriteMessageService::default());

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
                        },
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
                        },
                        Ok(v) => message_framed_write = v.message_framed_write,
                    };
                }
                loop {
                    let mut buf = BytesMut::with_capacity(
                        SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                    );
                    let read_client_timeout_seconds = SERVER_CONFIG
                        .read_client_timeout_seconds()
                        .unwrap_or(DEFAULT_READ_CLIENT_TIMEOUT_SECONDS);
                    match tokio::time::timeout(
                        Duration::from_secs(read_client_timeout_seconds),
                        client_stream_read_half.read_buf(&mut buf),
                    )
                    .await
                    {
                        Err(e) => {
                            error!("The read client data timeout: {:#?}.", e);
                            return;
                        },
                        Ok(Err(e)) => {
                            error!(
                                "Fail to read client data from {:#?} because of error: {:#?}",
                                target_address_a2t, e
                            );
                            return;
                        },
                        Ok(Ok(0)) if buf.remaining_mut() > 0 => {
                            debug!("Read all data from agent");
                            return;
                        },
                        Ok(Ok(size)) => {
                            debug!("Read {} bytes from client", size);
                        },
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
                        },
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
                        },
                        Ok(v) => v,
                    };
                    message_framed_write = write_agent_message_result.message_framed_write;
                }
            });
            tokio::spawn(async move {
                let mut read_proxy_message_service =
                    ServiceBuilder::new().service(ReadMessageService::new(
                        SERVER_CONFIG
                            .read_proxy_timeout_seconds()
                            .unwrap_or(DEFAULT_READ_PROXY_TIMEOUT_SECONDS),
                    ));
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
                        },
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
            Ok(RelayServiceResult {
                client_address: request.client_address,
            })
        })
    }
}
