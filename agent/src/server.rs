use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener as StdTcpListener};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, SockAddr, Socket, Type};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tower::ServiceBuilder;
use tracing::{error, info};

use common::ready_and_call_service;

use crate::service::{
    common::{
        ClientConnectionInfo, HandleClientConnectionService, DEFAULT_BUFFERED_CONNECTION_NUMBER,
    },
    AgentRsaCryptoFetcher,
};
use crate::{
    config::SERVER_CONFIG,
    service::common::{DEFAULT_CONCURRENCY_LIMIT, DEFAULT_RATE_LIMIT},
};

const DEFAULT_SERVER_PORT: u16 = 10080;

pub(crate) struct AgentServer {
    runtime: Runtime,
}

impl AgentServer {
    pub(crate) fn new() -> Self {
        let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
        runtime_builder
            .enable_all()
            .thread_keep_alive(Duration::from_secs(
                SERVER_CONFIG.thread_timeout().unwrap_or(2),
            ))
            .max_blocking_threads(SERVER_CONFIG.max_blocking_threads().unwrap_or(32))
            .worker_threads(SERVER_CONFIG.thread_number().unwrap_or(1024));
        let runtime = match runtime_builder.build() {
            Err(e) => {
                error!(
                    "Fail to create agent server runtime because of error: {:#?}",
                    e
                );
                panic!(
                    "Fail to create agent server runtime because of error: {:#?}",
                    e
                );
            },
            Ok(r) => r,
        };
        Self { runtime }
    }

    pub(crate) fn run(&self) {
        let proxy_addresses_from_config = SERVER_CONFIG
            .proxy_addresses()
            .as_ref()
            .expect("No proxy addresses configuration item");
        let mut proxy_addresses: Vec<SocketAddr> = Vec::new();
        for address in proxy_addresses_from_config {
            match SocketAddr::from_str(address) {
                Ok(r) => {
                    proxy_addresses.push(r);
                },
                Err(e) => {
                    error!(
                        "Fail to convert proxy address to socket address because of error: {:#?}",
                        e
                    )
                },
            }
        }
        let proxy_addresses = Arc::new(proxy_addresses);
        self.runtime.block_on(async {
            let socket2 = match Socket::new(Domain::IPV4, Type::STREAM, None) {
                Ok(v) => v,
                Err(e) => {
                    panic!(
                        "Fail to bind agent server port because of error: {:#?}",
                        e
                    );
                }
            };
            if let Err(e) = socket2.set_reuse_address(true) {
                panic!("Fail to bind agent server port because of error: {:#?}", e);
            };
           if let Err(e)= socket2.set_nodelay(true){
               panic!(
                   "Fail to bind agent server port because of error: {:#?}",
                   e
               );
           };
            if let Err(e)=    socket2.set_nonblocking(true){
                panic!(
                    "Fail to bind agent server port because of error: {:#?}",
                    e
                );
            };

            let local_socket_address = SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(0, 0, 0, 0),
                SERVER_CONFIG.port().unwrap_or(DEFAULT_SERVER_PORT),
            ));
            if let Err(e)=socket2.bind(&SockAddr::from(local_socket_address)) {
                panic!(
                    "Fail to bind agent server port because of error: {:#?}",
                    e
                );
            };
            if let Err(e)=   socket2.listen(SERVER_CONFIG.so_backlog().unwrap_or(1024)){
                panic!(
                    "Fail to bind agent server port because of error: {:#?}",
                    e
                );
            };
            if let Err(e)= socket2.set_recv_buffer_size(SERVER_CONFIG.so_recv_buffer_size().unwrap_or(1024*64)){
                panic!(
                        "Fail to bind agent server port because of error: {:#?}",
                        e
                    );
            };
             if let Err(e)= socket2.set_send_buffer_size(SERVER_CONFIG.so_send_buffer_size().unwrap_or(1024*64)){
                panic!(
                        "Fail to bind agent server port because of error: {:#?}",
                        e
                    );
            };
            let std_listener: StdTcpListener = socket2.into();
            let listener = match TcpListener::from_std(std_listener) {
                Err(e) => {
                    panic!(
                        "Fail to generate agent server listener from std listener because of error: {:#?}",
                        e
                    );
                }
                Ok(listener) => {
                    info!("Success to generate agent server listener.");
                    listener
                }
            };
            let agent_rsa_crypto_fetcher = match AgentRsaCryptoFetcher::new() {
                Err(e) => {
                    panic!(
                        "Fail to generate agent rsa crypto because of error: {:#?}",
                        e
                    );
                }
                Ok(v) => v
            };
            let agent_rsa_crypto_fetcher = Arc::new(agent_rsa_crypto_fetcher);
            loop {
                let agent_rsa_crypto_fetcher = agent_rsa_crypto_fetcher.clone();
                let (client_stream, client_address) = match listener.accept().await {
                    Err(e) => {
                        error!(
                            "Fail to accept client connection because of error: {:#?}",
                            e
                        );
                        continue;
                    }
                    Ok((client_stream, client_address)) => (client_stream, client_address),
                };
                if let Err(e) = client_stream.set_nodelay(true) {
                    error!(
                        "Fail to set client connection no delay because of error: {:#?}",
                        e
                    );
                    continue;
                }
                if let Some(so_linger) = SERVER_CONFIG.client_stream_so_linger() {
                    if let Err(e) = client_stream.set_linger(Some(Duration::from_secs(so_linger))) {
                        error!(
                            "Fail to set client connection linger because of error: {:#?}",
                            e
                        );
                    }
                }
                let proxy_addresses = proxy_addresses.clone();
                tokio::spawn(async move {
                    let mut handle_client_connection_service = ServiceBuilder::new()
                        .buffer(
                            SERVER_CONFIG
                                .buffered_connection_number()
                                .unwrap_or(DEFAULT_BUFFERED_CONNECTION_NUMBER),
                        )
                        .concurrency_limit(
                            SERVER_CONFIG
                                .concurrent_connection_number()
                                .unwrap_or(DEFAULT_CONCURRENCY_LIMIT),
                        )
                        .rate_limit(
                            SERVER_CONFIG.rate_limit().unwrap_or(DEFAULT_RATE_LIMIT),
                            Duration::from_secs(60),
                        )
                        .service(HandleClientConnectionService::new(proxy_addresses, agent_rsa_crypto_fetcher.clone()));
                    if let Err(e) = ready_and_call_service(
                        &mut handle_client_connection_service,
                        ClientConnectionInfo {
                            client_stream,
                            client_address,
                        },
                    )
                        .await
                    {
                        error!(
                            "Error happen when handle client connection [{}], error:{:#?}",
                            client_address, e
                        );
                    }
                });
            }
        });
    }
}
