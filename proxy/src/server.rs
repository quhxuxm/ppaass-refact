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

use tracing::error;

use common::ready_and_call_service;

use crate::{
    config::ProxyConfig,
    service::{AgentConnectionInfo, HandleAgentConnectionService, ProxyRsaCryptoFetcher},
};

const DEFAULT_SERVER_PORT: u16 = 80;

pub(crate) struct ProxyServer {
    runtime: TokioRuntime,
    configuration: Arc<ProxyConfig>,
}

impl ProxyServer {
    pub(crate) fn new(configuration: Arc<ProxyConfig>) -> Result<Self> {
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
        let server_socket = TcpSocket::new_v4()?;
        server_socket.set_reuseaddr(true)?;
        if let Some(so_recv_buffer_size) = self.configuration.so_recv_buffer_size() {
            server_socket.set_recv_buffer_size(so_recv_buffer_size)?;
        }
        if let Some(so_send_buffer_size) = self.configuration.so_send_buffer_size() {
            server_socket.set_send_buffer_size(so_send_buffer_size)?
        }
        let local_socket_address = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(0, 0, 0, 0),
            self.configuration.port().unwrap_or(DEFAULT_SERVER_PORT),
        ));
        server_socket.bind(local_socket_address)?;
        let listener = server_socket.listen(self.configuration.so_backlog().unwrap_or(1024))?;
        let proxy_rsa_crypto_fetcher = ProxyRsaCryptoFetcher::new(self.configuration.clone())?;
        let proxy_rsa_crypto_fetcher = Arc::new(proxy_rsa_crypto_fetcher);
        println!("ppaass-proxy is listening port: {} ", local_socket_address.port());
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
            if let Some(so_linger) = self.configuration.agent_stream_so_linger() {
                if let Err(e) = agent_stream.set_linger(Some(Duration::from_secs(so_linger))) {
                    error!("Fail to set agent connection linger because of error: {:#?}", e);
                    continue;
                }
            }
            let proxy_rsa_crypto_fetcher = proxy_rsa_crypto_fetcher.clone();
            let configuration = self.configuration.clone();
            tokio::spawn(async move {
                let mut handle_agent_connection_service = HandleAgentConnectionService::new(proxy_rsa_crypto_fetcher.clone(), configuration);
                if let Err(e) = ready_and_call_service(&mut handle_agent_connection_service, AgentConnectionInfo { agent_stream, agent_address }).await {
                    error!("Error happen when handle agent connection [{}], error:{:#?}", agent_address, e)
                }
            });
        }
    }

    pub(crate) fn run(&self) -> Result<()> {
        let concrete_run_future = self.concrete_run();
        self.runtime.block_on(concrete_run_future)?;
        Ok(())
    }
}
