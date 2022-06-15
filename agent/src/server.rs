use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::net::TcpSocket;
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime};
use tower::ServiceBuilder;
use tracing::error;

use common::ready_and_call_service;

use crate::service::{
    common::{ClientConnectionInfo, HandleClientConnectionService, DEFAULT_BUFFERED_CONNECTION_NUMBER},
    AgentRsaCryptoFetcher,
};
use crate::{
    config::AgentConfig,
    service::common::{DEFAULT_CONCURRENCY_LIMIT, DEFAULT_RATE_LIMIT},
};

const DEFAULT_SERVER_PORT: u16 = 10080;

pub(crate) struct AgentServer {
    runtime: Runtime,
    configuration: Arc<AgentConfig>,
}

impl AgentServer {
    pub(crate) fn new(configuration: Arc<AgentConfig>) -> Result<Self> {
        let mut runtime_builder = TokioRuntimeBuilder::new_multi_thread();
        runtime_builder
            .enable_all()
            .thread_keep_alive(Duration::from_secs(configuration.thread_timeout().unwrap_or(2)))
            .max_blocking_threads(configuration.max_blocking_threads().unwrap_or(32))
            .worker_threads(configuration.thread_number().unwrap_or(1024));

        Ok(Self {
            runtime: runtime_builder.build()?,
            configuration,
        })
    }

    async fn concrete_run(&self) -> Result<()> {
        let proxy_addresses_from_config = self.configuration.proxy_addresses().as_ref().expect("No proxy addresses configuration item");
        let mut proxy_addresses: Vec<SocketAddr> = Vec::new();
        for address in proxy_addresses_from_config {
            match SocketAddr::from_str(address) {
                Ok(r) => {
                    proxy_addresses.push(r);
                },
                Err(e) => {
                    error!("Fail to convert proxy address to socket address because of error: {:#?}", e)
                },
            }
        }
        let proxy_addresses = Arc::new(proxy_addresses);
        let server_socket = TcpSocket::new_v4()?;
        server_socket.set_reuseaddr(true)?;
        if let Some(so_recv_buffer_size) = self.configuration.so_recv_buffer_size() {
            server_socket.set_recv_buffer_size(so_recv_buffer_size)?;
        }
        if let Some(so_send_buffer_size) = self.configuration.so_send_buffer_size() {
            server_socket.set_send_buffer_size(so_send_buffer_size)?;
        }
        let local_socket_address = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(0, 0, 0, 0),
            self.configuration.port().unwrap_or(DEFAULT_SERVER_PORT),
        ));
        server_socket.bind(local_socket_address)?;
        let listener = server_socket.listen(self.configuration.so_backlog().unwrap_or(1024))?;
        let agent_rsa_crypto_fetcher = AgentRsaCryptoFetcher::new(self.configuration.clone())?;
        let agent_rsa_crypto_fetcher = Arc::new(agent_rsa_crypto_fetcher);
        loop {
            let agent_rsa_crypto_fetcher = agent_rsa_crypto_fetcher.clone();
            let (client_stream, client_address) = match listener.accept().await {
                Err(e) => {
                    error!("Fail to accept client connection because of error: {:#?}", e);
                    continue;
                },
                Ok((client_stream, client_address)) => (client_stream, client_address),
            };
            if let Err(e) = client_stream.set_nodelay(true) {
                error!("Fail to set client connection no delay because of error: {:#?}", e);
                continue;
            }
            if let Some(so_linger) = self.configuration.client_stream_so_linger() {
                if let Err(e) = client_stream.set_linger(Some(Duration::from_secs(so_linger))) {
                    error!("Fail to set client connection linger because of error: {:#?}", e);
                }
            }
            let proxy_addresses = proxy_addresses.clone();
            let configuration = self.configuration.clone();
            tokio::spawn(async move {
                let mut handle_client_connection_service = ServiceBuilder::new()
                    .buffer(configuration.buffered_connection_number().unwrap_or(DEFAULT_BUFFERED_CONNECTION_NUMBER))
                    .concurrency_limit(configuration.concurrent_connection_number().unwrap_or(DEFAULT_CONCURRENCY_LIMIT))
                    .rate_limit(configuration.rate_limit().unwrap_or(DEFAULT_RATE_LIMIT), Duration::from_secs(60))
                    .service(HandleClientConnectionService::new(
                        proxy_addresses,
                        agent_rsa_crypto_fetcher.clone(),
                        configuration.clone(),
                    ));
                if let Err(e) = ready_and_call_service(&mut handle_client_connection_service, ClientConnectionInfo { client_stream, client_address }).await {
                    error!("Error happen when handle client connection [{}], error:{:#?}", client_address, e);
                }
            });
        }
    }

    pub(crate) fn run(&self) -> Result<()> {
        self.runtime.block_on(self.concrete_run())?;
        Ok(())
    }
}
