use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{future, SinkExt, StreamExt};
use tokio::net::TcpStream;
use tower::retry::{Policy, Retry};
use tower::util::BoxCloneService;
use tower::{service_fn, Service, ServiceExt};
use tracing::{debug, error};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, CommonError, Message, MessageCodec,
    MessageFramedRead, MessageFramedWrite, MessagePayload, PayloadEncryptionType, PayloadType,
    PrepareMessageFramedResult, PrepareMessageFramedService,
};

use crate::config::{AGENT_PUBLIC_KEY, PROXY_PRIVATE_KEY};
use crate::service::tcp::connect::{
    TcpConnectService, TcpConnectServiceRequest, TcpConnectServiceResult,
};
use crate::service::tcp::relay::{TcpRelayService, TcpRelayServiceRequest, TcpRelayServiceResult};
use crate::service::udp::associate::{
    UdpAssociateService, UdpAssociateServiceRequest, UdpAssociateServiceResult,
};
use crate::service::udp::relay::{UdpRelayService, UdpRelayServiceRequest, UdpRelayServiceResult};
use crate::SERVER_CONFIG;

mod tcp;
mod udp;
const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
const DEFAULT_MAX_FRAME_SIZE: usize = DEFAULT_BUFFER_SIZE * 2;
#[derive(Debug)]
pub(crate) struct AgentConnectionInfo {
    pub agent_stream: TcpStream,
    pub agent_address: SocketAddr,
}

pub(crate) struct HandleAgentConnectionService {
    prepare_message_frame_service:
        BoxCloneService<TcpStream, PrepareMessageFramedResult, CommonError>,
    tcp_connect_service:
        BoxCloneService<TcpConnectServiceRequest, TcpConnectServiceResult, CommonError>,
    tcp_relay_service: BoxCloneService<TcpRelayServiceRequest, TcpRelayServiceResult, CommonError>,
    udp_associate_service:
        BoxCloneService<UdpAssociateServiceRequest, UdpAssociateServiceResult, CommonError>,
    udp_relay_service: BoxCloneService<UdpRelayServiceRequest, UdpRelayServiceResult, CommonError>,
}

impl HandleAgentConnectionService {
    pub(crate) fn new() -> Self {
        let agent_public_key = &(*AGENT_PUBLIC_KEY);
        let proxy_private_key = &(*PROXY_PRIVATE_KEY);
        Self {
            prepare_message_frame_service: BoxCloneService::new(PrepareMessageFramedService::new(
                agent_public_key,
                proxy_private_key,
                SERVER_CONFIG
                    .max_frame_size()
                    .unwrap_or(DEFAULT_MAX_FRAME_SIZE),
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                SERVER_CONFIG.compress().unwrap_or(true),
            )),
            tcp_connect_service: BoxCloneService::new(TcpConnectService::new()),
            tcp_relay_service: BoxCloneService::new(TcpRelayService::new()),
            udp_associate_service: BoxCloneService::new(UdpAssociateService),
            udp_relay_service: BoxCloneService::new(UdpRelayService),
        }
    }
}

impl Service<AgentConnectionInfo> for HandleAgentConnectionService {
    type Response = ();
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: AgentConnectionInfo) -> Self::Future {
        let mut prepare_message_frame_service = self.prepare_message_frame_service.clone();
        let mut tcp_connect_service = self.tcp_connect_service.clone();
        let mut tcp_relay_service = self.tcp_relay_service.clone();
        Box::pin(async move {
            let framed_result = prepare_message_frame_service
                .ready()
                .await?
                .call(req.agent_stream)
                .await?;
            let tcp_connect_result = tcp_connect_service
                .ready()
                .await?
                .call(TcpConnectServiceRequest {
                    message_frame_read: framed_result.message_framed_read,
                    message_frame_write: framed_result.message_framed_write,
                    agent_address: req.agent_address,
                })
                .await?;
            tcp_relay_service
                .ready()
                .await?
                .call(TcpRelayServiceRequest {
                    message_frame_read: tcp_connect_result.message_frame_read,
                    message_frame_write: tcp_connect_result.message_frame_write,
                    agent_address: req.agent_address,
                    target_stream: tcp_connect_result.target_stream,
                    source_address: tcp_connect_result.source_address,
                    target_address: tcp_connect_result.target_address,
                    user_token: tcp_connect_result.user_token,
                    agent_tcp_connect_message_id: tcp_connect_result.agent_tcp_connect_message_id,
                })
                .await?;
            Ok(())
        })
    }
}

#[derive(Clone)]
pub(crate) struct ConnectToTargetServiceRequest {
    pub target_address: String,
    pub agent_address: SocketAddr,
}

pub(crate) struct ConnectToTargetServiceResult {
    pub target_stream: TcpStream,
}

#[derive(Clone)]
struct ConnectToTargetAttempts {
    retry: u16,
}

#[derive(Clone)]
pub(crate) struct ConnectToTargetService {
    concrete_service:
        BoxCloneService<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, CommonError>,
}

impl ConnectToTargetService {
    pub(crate) fn new(retry: u16) -> Self {
        let concrete_service = Retry::new(
            ConnectToTargetAttempts { retry },
            service_fn(|request: ConnectToTargetServiceRequest| async move {
                debug!(
                    "Agent {}, begin connect to target: {}",
                    request.agent_address, request.target_address
                );
                let target_stream = TcpStream::connect(&request.target_address)
                    .await
                    .map_err(|e| CommonError::IoError { source: e })?;
                debug!(
                    "Agent {}, success connect to target: {}",
                    request.agent_address, request.target_address
                );
                Ok(ConnectToTargetServiceResult { target_stream })
            }),
        );
        Self {
            concrete_service: BoxCloneService::new(concrete_service),
        }
    }
}

impl Policy<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, CommonError>
    for ConnectToTargetAttempts
{
    type Future = futures_util::future::Ready<Self>;

    fn retry(
        &self,
        req: &ConnectToTargetServiceRequest,
        result: Result<&ConnectToTargetServiceResult, &CommonError>,
    ) -> Option<Self::Future> {
        match result {
            Ok(_) => {
                // Treat all `Response`s as success,
                // so don't retry...
                None
            }
            Err(_) => {
                // Treat all errors as failures...
                // But we limit the number of attempts...
                if self.retry > 0 {
                    // Try again!
                    return Some(future::ready(ConnectToTargetAttempts {
                        retry: self.retry - 1,
                    }));
                }
                // Used all our attempts, no retry...
                None
            }
        }
    }

    fn clone_request(
        &self,
        req: &ConnectToTargetServiceRequest,
    ) -> Option<ConnectToTargetServiceRequest> {
        Some(req.clone())
    }
}

impl Service<ConnectToTargetServiceRequest> for ConnectToTargetService {
    type Response = ConnectToTargetServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: ConnectToTargetServiceRequest) -> Self::Future {
        let mut concrete_connect_service = self.concrete_service.clone();
        Box::pin(async move {
            let concrete_connect_result = concrete_connect_service
                .ready()
                .await?
                .call(request.clone())
                .await;
            match concrete_connect_result {
                Ok(r) => Ok(r),
                Err(e) => {
                    error!(
                        "Agent {} fail to connect target: {} because of error: {:#?}",
                        request.agent_address, request.target_address, e
                    );
                    Err(e)
                }
            }
        })
    }
}
