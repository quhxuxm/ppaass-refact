use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::net::TcpSocket;
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime};

use tracing::{error, instrument};

use crate::service::{common::ClientConnection, AgentRsaCryptoFetcher};
use crate::{config::AgentConfig, service::pool::ProxyConnectionPool};

const DEFAULT_SERVER_PORT: u16 = 10080;

#[derive(Debug)]
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

    #[instrument(skip_all)]
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
        let proxy_connection_pool =
            Arc::new(ProxyConnectionPool::new(proxy_addresses.clone(), self.configuration.clone(), agent_rsa_crypto_fetcher.clone()).await?);
        println!("ppaass-agent is listening port: {} ", local_socket_address.port());
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

            let configuration = self.configuration.clone();
            let proxy_connection_pool = proxy_connection_pool.clone();
            tokio::spawn(async move {
                let client_connection = ClientConnection::new(client_stream, client_address);
                if let Err(e) = client_connection
                    .exec(agent_rsa_crypto_fetcher.clone(), configuration.clone(), proxy_connection_pool)
                    .await
                {
                    error!("Error happen when handle client connection [{}], error:{:#?}", client_address, e);
                }
            });
        }
    }

    #[instrument(skip_all)]
    pub(crate) fn run(&self) -> Result<()> {
        self.runtime.block_on(self.concrete_run())?;
        Ok(())
    }
}
