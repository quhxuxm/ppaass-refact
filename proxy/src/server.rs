use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tower::{Service, ServiceBuilder, ServiceExt};
use tracing::{error, info};

use crate::config::SERVER_CONFIG;
use crate::service::{AgentConnectionInfo, HandleAgentConnectionService};

const DEFAULT_SERVER_PORT: u16 = 80;

pub(crate) struct ProxyServer {
    runtime: Runtime,
}

impl ProxyServer {
    pub(crate) fn new() -> Self {
        let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
        runtime_builder
            .enable_all()
            .thread_keep_alive(Duration::from_secs(SERVER_CONFIG.thread_timeout().unwrap_or(2), ))
            .max_blocking_threads(SERVER_CONFIG.max_blocking_threads().unwrap_or(32))
            .worker_threads(SERVER_CONFIG.thread_number()
                .unwrap_or(1024));
        let runtime = match runtime_builder.build() {
            Err(e) => {
                error!(
                    "Fail to create proxy server runtime because of error: {:#?}",
                    e
                );
                panic!(
                    "Fail to create proxy server runtime because of error: {:#?}",
                    e
                );
            }
            Ok(r) => r,
        };
        Self { runtime }
    }

    pub(crate) fn run(&self) {
        self.runtime.block_on(async {
            let listener = match TcpListener::bind(SocketAddrV4::new(
                Ipv4Addr::new(0, 0, 0, 0),
                SERVER_CONFIG.port().unwrap_or(DEFAULT_SERVER_PORT),
            )).await {
                Err(e) => {
                    panic!("Fail to bind proxy server port because of error: {:#?}", e);
                }
                Ok(listener) => {
                    info!("Success to bind proxy server port, start listening ... ");
                    listener
                }
            };
            let mut handle_agent_connection_service = ServiceBuilder::new().buffer(SERVER_CONFIG.buffered_connection_number().unwrap_or(1024)).concurrency_limit(SERVER_CONFIG.concurrent_connection_number().unwrap_or(1024)).service::<HandleAgentConnectionService>(Default::default());
            loop {
                let (agent_stream, agent_address) = match listener.accept().await {
                    Err(e) => {
                        error!(
                            "Fail to accept client connection because of error: {:#?}",
                            e
                        );
                        continue;
                    }
                    Ok((agent_stream, agent_address)) => (agent_stream, agent_address),
                };
                match handle_agent_connection_service.ready().await {
                    Err(e) => {
                        error!(
                            "Error happen when handle agent connection [{}] on poll ready, error:{:#?}",
                            agent_address, e
                        );
                        continue;
                    }
                    Ok(s) => {
                        if let Err(e) = s.call(AgentConnectionInfo {
                            agent_stream,
                            agent_address,
                        }).await {
                            error!(
                                "Error happen when handle agent connection [{}], error:{:#?}",
                                agent_address, e
                            )
                        }
                    }
                }
            }
        });
    }
}
