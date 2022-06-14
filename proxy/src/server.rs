use std::net::SocketAddr;
use std::time::Duration;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

use anyhow::Result;
use tokio::net::TcpSocket;
use tokio::runtime::Builder as TokioRuntimeBuilder;
use tokio::runtime::Runtime as TokioRuntime;
use tower::ServiceBuilder;
use tracing::error;

use common::ready_and_call_service;

use crate::service::{AgentConnectionInfo, HandleAgentConnectionService, ProxyRsaCryptoFetcher, DEFAULT_RATE_LIMIT};
use crate::{
    config::SERVER_CONFIG,
    service::{DEFAULT_BUFFERED_CONNECTION_NUMBER, DEFAULT_CONCURRENCY_LIMIT},
};

const DEFAULT_SERVER_PORT: u16 = 80;

pub(crate) struct ProxyServer {
    runtime: TokioRuntime,
}

impl ProxyServer {
    pub(crate) fn new() -> Result<Self> {
        let mut runtime_builder = TokioRuntimeBuilder::new_multi_thread();
        runtime_builder
            .enable_all()
            .thread_keep_alive(Duration::from_secs(SERVER_CONFIG.thread_timeout().unwrap_or(2)))
            .max_blocking_threads(SERVER_CONFIG.max_blocking_threads().unwrap_or(32))
            .worker_threads(SERVER_CONFIG.thread_number().unwrap_or(1024));
        Ok(Self {
            runtime: runtime_builder.build()?,
        })
    }

    async fn concrete_run() -> Result<()> {
        let server_socket = TcpSocket::new_v4()?;
        server_socket.set_reuseaddr(true)?;
        if let Some(so_recv_buffer_size) = SERVER_CONFIG.so_recv_buffer_size() {
            server_socket.set_recv_buffer_size(so_recv_buffer_size)?;
        }
        if let Some(so_send_buffer_size) = SERVER_CONFIG.so_send_buffer_size() {
            server_socket.set_send_buffer_size(so_send_buffer_size)?
        }
        let local_socket_address = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(0, 0, 0, 0),
            SERVER_CONFIG.port().unwrap_or(DEFAULT_SERVER_PORT),
        ));
        server_socket.bind(local_socket_address)?;
        let listener = server_socket.listen(SERVER_CONFIG.so_backlog().unwrap_or(1024))?;
        let proxy_rsa_crypto_fetcher = ProxyRsaCryptoFetcher::new()?;
        let proxy_rsa_crypto_fetcher = Arc::new(proxy_rsa_crypto_fetcher);
        loop {
            let (agent_stream, agent_address) = match listener.accept().await {
                Err(e) => {
                    error!("Fail to accept agent connection because of error: {:#?}", e);
                    continue;
                },
                Ok((agent_stream, agent_address)) => (agent_stream, agent_address),
            };
            if let Err(e) = agent_stream.set_nodelay(true) {
                error!("Fail to set agent connection no delay because of error: {:#?}", e);
                continue;
            }
            if let Some(so_linger) = SERVER_CONFIG.agent_stream_so_linger() {
                if let Err(e) = agent_stream.set_linger(Some(Duration::from_secs(so_linger))) {
                    error!("Fail to set agent connection linger because of error: {:#?}", e);
                    continue;
                }
            }
            let proxy_rsa_crypto_fetcher = proxy_rsa_crypto_fetcher.clone();
            tokio::spawn(async move {
                let mut handle_agent_connection_service = ServiceBuilder::new()
                    .buffer(SERVER_CONFIG.buffered_connection_number().unwrap_or(DEFAULT_BUFFERED_CONNECTION_NUMBER))
                    .concurrency_limit(SERVER_CONFIG.concurrent_connection_number().unwrap_or(DEFAULT_CONCURRENCY_LIMIT))
                    .rate_limit(SERVER_CONFIG.rate_limit().unwrap_or(DEFAULT_RATE_LIMIT), Duration::from_secs(60))
                    .service(HandleAgentConnectionService::new(proxy_rsa_crypto_fetcher.clone()));
                if let Err(e) = ready_and_call_service(&mut handle_agent_connection_service, AgentConnectionInfo { agent_stream, agent_address }).await {
                    error!("Error happen when handle agent connection [{}], error:{:#?}", agent_address, e)
                }
            });
        }
    }

    pub(crate) fn run(&self) -> Result<()> {
        self.runtime.block_on(Self::concrete_run())?;
        Ok(())
    }
}
