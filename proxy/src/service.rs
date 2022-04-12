use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::future;
use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{
    retry::{Policy, Retry},
    ServiceBuilder,
};
use tower::{service_fn, Service};
use tracing::{debug, error};

use common::{ready_and_call_service, CommonError, PrepareMessageFramedService};

use crate::config::{AGENT_PUBLIC_KEY, PROXY_PRIVATE_KEY};
use crate::service::tcp::connect::{TcpConnectService, TcpConnectServiceRequest};
use crate::service::tcp::relay::{TcpRelayService, TcpRelayServiceRequest};
use crate::SERVER_CONFIG;

mod tcp;
mod udp;
const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
const DEFAULT_MAX_FRAME_SIZE: usize = DEFAULT_BUFFER_SIZE * 2;
pub const DEFAULT_BUFFERED_CONNECTION_NUMBER: usize = 1024;
pub const DEFAULT_RATE_LIMIT: u64 = 1024;
pub const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;
#[derive(Debug)]
pub(crate) struct AgentConnectionInfo {
    pub agent_stream: TcpStream,
    pub agent_address: SocketAddr,
}

#[derive(Debug, Default)]
pub(crate) struct HandleAgentConnectionService;

impl Service<AgentConnectionInfo> for HandleAgentConnectionService {
    type Response = ();
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: AgentConnectionInfo) -> Self::Future {
        Box::pin(async move {
            let agent_public_key = &(*AGENT_PUBLIC_KEY);
            let proxy_private_key = &(*PROXY_PRIVATE_KEY);
            let mut prepare_message_frame_service =
                ServiceBuilder::new().service(PrepareMessageFramedService::new(
                    agent_public_key,
                    proxy_private_key,
                    SERVER_CONFIG
                        .max_frame_size()
                        .unwrap_or(DEFAULT_MAX_FRAME_SIZE),
                    SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                    SERVER_CONFIG.compress().unwrap_or(true),
                ));
            let mut tcp_connect_service =
                ServiceBuilder::new().service(TcpConnectService::default());
            let mut tcp_relay_service = ServiceBuilder::new().service(TcpRelayService::default());
            let framed_result =
                ready_and_call_service(&mut prepare_message_frame_service, req.agent_stream)
                    .await?;
            let tcp_connect_result = ready_and_call_service(
                &mut tcp_connect_service,
                TcpConnectServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                    message_framed_write: framed_result.message_framed_write,
                    agent_address: req.agent_address,
                },
            )
            .await?;
            let relay_result = ready_and_call_service(
                &mut tcp_relay_service,
                TcpRelayServiceRequest {
                    message_framed_read: tcp_connect_result.message_framed_read,
                    message_framed_write: tcp_connect_result.message_framed_write,
                    agent_address: req.agent_address,
                    target_stream: tcp_connect_result.target_stream,
                    source_address: tcp_connect_result.source_address,
                    target_address: tcp_connect_result.target_address,
                    user_token: tcp_connect_result.user_token,
                    agent_tcp_connect_message_id: tcp_connect_result.agent_tcp_connect_message_id,
                },
            )
            .await;
            match relay_result {
                Err(e) => {
                    error!("Error happen when relay agent connection, error: {:#?}", e);
                },
                Ok(r) => {
                    debug!("Relay process started for agent: {:#?}", r.agent_address);
                },
            }
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
    pub(crate) fn new(retry: u16, connect_timeout_seconds: u64) -> Self {
        let connect_timeout_seconds = connect_timeout_seconds;
        let concrete_service = Retry::new(
            ConnectToTargetAttempts { retry },
            service_fn(move |request: ConnectToTargetServiceRequest| async move {
                debug!("Begin connect to target: {}", request.target_address);
                let target_stream = match tokio::time::timeout(
                    Duration::from_secs(connect_timeout_seconds),
                    TcpStream::connect(&request.target_address),
                )
                .await
                {
                    Err(e) => {
                        error!("The connect to target timeout: {:#?}.", e);
                        return Err(CommonError::TimeoutError);
                    },
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        error!(
                            "Fail connect to target {} because of error: {:#?}",
                            &request.target_address, e
                        );
                        return Err(CommonError::IoError { source: e });
                    },
                };

                target_stream
                    .set_nodelay(true)
                    .map_err(|e| CommonError::IoError { source: e })?;
                target_stream
                    .set_linger(None)
                    .map_err(|e| CommonError::IoError { source: e })?;
                debug!("Success connect to target: {}", request.target_address);
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
        _req: &ConnectToTargetServiceRequest,
        result: Result<&ConnectToTargetServiceResult, &CommonError>,
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
                    return Some(future::ready(ConnectToTargetAttempts {
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
        self.concrete_service.poll_ready(cx)
    }

    fn call(&mut self, request: ConnectToTargetServiceRequest) -> Self::Future {
        let mut concrete_connect_service = self.concrete_service.clone();
        Box::pin(async move {
            let concrete_connect_result =
                ready_and_call_service(&mut concrete_connect_service, request.clone()).await;
            match concrete_connect_result {
                Ok(r) => Ok(r),
                Err(e) => {
                    error!(
                        "Agent {} fail to connect target: {} because of error: {:#?}",
                        request.agent_address, request.target_address, e
                    );
                    Err(e)
                },
            }
        })
    }
}
