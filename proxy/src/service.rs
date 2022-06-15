use std::time::Duration;
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

use futures::future::BoxFuture;
use tokio::net::TcpStream;

use tower::util::BoxCloneService;
use tower::ServiceBuilder;
use tower::{service_fn, Service};
use tracing::{debug, error};

use common::{ready_and_call_service, PpaassError, PrepareMessageFramedService, RsaCrypto, RsaCryptoFetcher};

use crate::service::tcp::relay::{TcpRelayService, TcpRelayServiceRequest};
use crate::{
    config::ProxyConfig,
    service::tcp::connect::{TcpConnectService, TcpConnectServiceRequest},
};

use anyhow::Result;
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
    pub fn new(configuration: Arc<ProxyConfig>) -> Result<Self> {
        let mut result = Self { cache: HashMap::new() };
        let rsa_dir_path = configuration.rsa_root_dir().as_ref().expect("Fail to read rsa root directory.");
        let rsa_dir = fs::read_dir(rsa_dir_path)?;
        rsa_dir.for_each(|entry| {
            let entry = match entry {
                Err(e) => {
                    error!("Fail to read {} directory because of error: {:#?}", rsa_dir_path, e);
                    return;
                },
                Ok(v) => v,
            };
            let user_token = entry.file_name();
            let user_token = user_token.to_str();
            let user_token = match user_token {
                None => {
                    error!("Fail to read {}{:?} directory because of user token not exist", rsa_dir_path, entry.file_name());
                    return;
                },
                Some(v) => v,
            };
            let public_key = match fs::read_to_string(Path::new(format!("{}{}/AgentPublicKey.pem", rsa_dir_path, user_token).as_str())) {
                Err(e) => {
                    error!("Fail to read {}{}/AgentPublicKey.pem because of error: {:#?}", rsa_dir_path, user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            let private_key = match fs::read_to_string(Path::new(format!("{}{}/ProxyPrivateKey.pem", rsa_dir_path, user_token).as_str())) {
                Err(e) => {
                    error!("Fail to read {}{}/ProxyPrivateKey.pem because of error: {:#?}", rsa_dir_path, user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            let rsa_crypto = match RsaCrypto::new(public_key, private_key) {
                Err(e) => {
                    error!("Fail to create rsa crypto for user: {} because of error: {:#?}", user_token, e);
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
    fn fetch<Q>(&self, user_token: Q) -> Result<Option<&RsaCrypto>, PpaassError>
    where
        Q: AsRef<str>,
    {
        Ok(self.cache.get(user_token.as_ref()))
    }
}

pub(crate) struct AgentConnectionInfo {
    pub agent_stream: TcpStream,
    pub agent_address: SocketAddr,
}

impl Debug for AgentConnectionInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgentConnectionInfo: agent_address={}", self.agent_address)
    }
}

#[derive(Debug)]
pub(crate) struct HandleAgentConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetch: Arc<T>,
    configuration: Arc<ProxyConfig>,
}
impl<T> HandleAgentConnectionService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetch: Arc<T>, configuration: Arc<ProxyConfig>) -> Self {
        Self {
            rsa_crypto_fetch,
            configuration,
        }
    }
}

impl<T> Service<AgentConnectionInfo> for HandleAgentConnectionService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = ();
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: AgentConnectionInfo) -> Self::Future {
        let rsa_crypto_fetch = self.rsa_crypto_fetch.clone();
        let message_framed_buffer_size = self.configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = self.configuration.compress().unwrap_or(true);
        let configuration = self.configuration.clone();
        Box::pin(async move {
            let mut prepare_message_frame_service =
                ServiceBuilder::new().service(PrepareMessageFramedService::new(message_framed_buffer_size, compress, rsa_crypto_fetch.clone()));
            let mut tcp_connect_service: TcpConnectService = Default::default();
            let mut tcp_relay_service: TcpRelayService<T> = Default::default();
            let framed_result = ready_and_call_service(&mut prepare_message_frame_service, req.agent_stream).await?;
            let tcp_connect_result = ready_and_call_service(
                &mut tcp_connect_service,
                TcpConnectServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                    message_framed_write: framed_result.message_framed_write,
                    agent_address: req.agent_address,
                    configuration: configuration.clone(),
                },
            )
            .await?;
            ready_and_call_service(
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
                    configuration,
                },
            )
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
pub(crate) struct ConnectToTargetService {
    concrete_service: BoxCloneService<ConnectToTargetServiceRequest, ConnectToTargetServiceResult, anyhow::Error>,
}

impl ConnectToTargetService {
    pub(crate) fn new(target_stream_so_linger: u64) -> Self {
        let concrete_service = service_fn(move |request: ConnectToTargetServiceRequest| async move {
            debug!("Begin connect to target: {}", request.target_address);
            let target_stream = TcpStream::connect(&request.target_address).await?;
            target_stream.set_nodelay(true)?;
            target_stream.set_linger(Some(Duration::from_secs(target_stream_so_linger)))?;
            debug!("Success connect to target: {}", request.target_address);
            Ok(ConnectToTargetServiceResult { target_stream })
        });
        Self {
            concrete_service: BoxCloneService::new(concrete_service),
        }
    }
}

impl Service<ConnectToTargetServiceRequest> for ConnectToTargetService {
    type Response = ConnectToTargetServiceResult;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.concrete_service.poll_ready(cx)
    }

    fn call(&mut self, request: ConnectToTargetServiceRequest) -> Self::Future {
        let mut concrete_connect_service = self.concrete_service.clone();
        Box::pin(async move { ready_and_call_service(&mut concrete_connect_service, request.clone()).await })
    }
}
