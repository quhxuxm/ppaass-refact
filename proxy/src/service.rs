use std::{collections::HashMap, net::SocketAddr};
use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};
use std::{
    fs,
    path::Path,
    task::{Context, Poll},
};
use std::time::Duration;

use futures::future;
use futures::future::BoxFuture;
use tokio::net::TcpStream;
use tower::{
    retry::{Policy, Retry},
    ServiceBuilder,
};
use tower::{Service, service_fn};
use tower::util::BoxCloneService;
use tracing::{debug, error};

use common::{
    PpaassError, PrepareMessageFramedService, ready_and_call_service, RsaCrypto, RsaCryptoFetcher,
};

use crate::SERVER_CONFIG;
use crate::service::tcp::connect::{TcpConnectService, TcpConnectServiceRequest};
use crate::service::tcp::relay::{TcpRelayService, TcpRelayServiceRequest};

mod tcp;
mod udp;
const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
pub const DEFAULT_BUFFERED_CONNECTION_NUMBER: usize = 1024;
pub const DEFAULT_RATE_LIMIT: u64 = 1024;
pub const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;
pub(crate) struct ProxyRsaCryptoFetcher {
    cache: HashMap<String, RsaCrypto>,
}

impl ProxyRsaCryptoFetcher {
    pub fn new() -> Result<Self, PpaassError> {
        let mut result = Self {
            cache: HashMap::new(),
        };
        let rsa_dir_path = SERVER_CONFIG
            .rsa_root_dir()
            .as_ref()
            .expect("Fail to read rsa root directory.");
        let rsa_dir = fs::read_dir(rsa_dir_path)?;
        rsa_dir.for_each(|entry| {
            let entry = match entry {
                Err(e) => {
                    error!(
                        "Fail to read {} directory because of error: {:#?}",
                        rsa_dir_path, e
                    );
                    return;
                },
                Ok(v) => v,
            };
            let user_token = entry.file_name();
            let user_token = user_token.to_str();
            let user_token = match user_token {
                None => {
                    error!(
                        "Fail to read {}{:?} directory because of user token not exist",
                        rsa_dir_path,
                        entry.file_name()
                    );
                    return;
                },
                Some(v) => v,
            };
            let public_key = match fs::read_to_string(Path::new(
                format!("{}{}/AgentPublicKey.pem", rsa_dir_path, user_token).as_str(),
            )) {
                Err(e) => {
                    error!(
                        "Fail to read {}{}/AgentPublicKey.pem because of error: {:#?}",
                        rsa_dir_path, user_token, e
                    );
                    return;
                },
                Ok(v) => v,
            };
            let private_key = match fs::read_to_string(Path::new(
                format!("{}{}/ProxyPrivateKey.pem", rsa_dir_path, user_token).as_str(),
            )) {
                Err(e) => {
                    error!(
                        "Fail to read {}{}/ProxyPrivateKey.pem because of error: {:#?}",
                        rsa_dir_path, user_token, e
                    );
                    return;
                },
                Ok(v) => v,
            };
            let rsa_crypto = match RsaCrypto::new(public_key, private_key) {
                Err(e) => {
                    error!(
                        "Fail to create rsa crypto for user: {} because of error: {:#?}",
                        user_token, e
                    );
                    return;
                },
                Ok(v) => v,
            };
            result.cache.insert(user_token.to_string(), rsa_crypto);
        });
        Ok(result)
    }
}

impl RsaCryptoFetcher for ProxyRsaCryptoFetcher {
    fn fetch(&self, user_token: &str) -> Result<Option<&RsaCrypto>, PpaassError> {
        let val = self.cache.get(user_token);
        match val {
            None => Ok(None),
            Some(v) => Ok(Some(v)),
        }
    }
}

pub(crate) struct AgentConnectionInfo {
    pub agent_stream: TcpStream,
    pub agent_address: SocketAddr,
}

impl Debug for AgentConnectionInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AgentConnectionInfo: agent_address={}",
            self.agent_address
        )
    }
}

#[derive(Debug)]
pub(crate) struct HandleAgentConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetch: Arc<T>,
}
impl<T> HandleAgentConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetch: Arc<T>) -> Self {
        Self { rsa_crypto_fetch }
    }
}

impl<T> Service<AgentConnectionInfo> for HandleAgentConnectionService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = ();
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: AgentConnectionInfo) -> Self::Future {
        let rsa_crypto_fetch = self.rsa_crypto_fetch.clone();
        Box::pin(async move {
            let buffer_size = SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let mut prepare_message_frame_service =
                ServiceBuilder::new().service(PrepareMessageFramedService::new(
                    buffer_size * 2,
                    buffer_size,
                    SERVER_CONFIG.compress().unwrap_or(true),
                    rsa_crypto_fetch.clone(),
                ));
            let mut tcp_connect_service =
                ServiceBuilder::new().service(TcpConnectService::default());
            let mut tcp_relay_service =
                ServiceBuilder::new().service(TcpRelayService::<T>::default());
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

impl Debug for ConnectToTargetServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ConnectToTargetServiceRequest: agent_address={}, target_address={}",
            self.agent_address, self.target_address
        )
    }
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
    BoxCloneService<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, PpaassError>,
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
                    Err(_e) => {
                        error!(
                            "The connect to target timeout, duration: {}.",
                            connect_timeout_seconds
                        );
                        return Err(PpaassError::TimeoutError);
                    },
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        error!(
                            "Fail connect to target {} because of error: {:#?}",
                            &request.target_address, e
                        );
                        return Err(PpaassError::IoError { source: e });
                    },
                };
                target_stream
                    .set_nodelay(true)
                    .map_err(|e| PpaassError::IoError { source: e })?;
                if let Some(so_linger) = SERVER_CONFIG.target_stream_so_linger() {
                    target_stream
                        .set_linger(Some(Duration::from_secs(so_linger)))
                        .map_err(|e| PpaassError::IoError { source: e })?;
                }
                debug!("Success connect to target: {}", request.target_address);
                Ok(ConnectToTargetServiceResult { target_stream })
            }),
        );
        Self {
            concrete_service: BoxCloneService::new(concrete_service),
        }
    }
}

impl Policy<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, PpaassError>
for ConnectToTargetAttempts
{
    type Future = future::Ready<Self>;

    fn retry(
        &self, _req: &ConnectToTargetServiceRequest,
        result: Result<&ConnectToTargetServiceResult, &PpaassError>,
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
        &self, req: &ConnectToTargetServiceRequest,
    ) -> Option<ConnectToTargetServiceRequest> {
        Some(req.clone())
    }
}

impl Service<ConnectToTargetServiceRequest> for ConnectToTargetService {
    type Response = ConnectToTargetServiceResult;
    type Error = PpaassError;
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
