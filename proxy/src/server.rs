use std::net::{Ipv4Addr, SocketAddrV4};

use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tower::{Service, ServiceBuilder, ServiceExt};
use tracing::{error, info};

use crate::config::SERVER_CONFIG;
use crate::service::common::{ClientConnectionInfo, HandleClientConnectionService};

const DEFAULT_SERVER_PORT: u16 = 80;

pub(crate) struct ProxyServer {
    runtime: Runtime,
}

impl ProxyServer {
    pub(crate) fn new() -> Self {
        let runtime = match tokio::runtime::Runtime::new() {
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
                SERVER_CONFIG.port().unwrap_or(DEFAULT_SERVER_PORT)
            ))
            .await
            {
                Err(e) => {
                    panic!("Fail to bind proxy server port because of error: {:#?}", e);
                }
                Ok(listener) => {
                    info!("Success to bind proxy server port, start listening ... ");
                    listener
                }
            };
            let mut handle_agent_connection_service = ServiceBuilder::new()
                .buffer(100)
                .concurrency_limit(10)
                .service(HandleClientConnectionService::new());
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
                match handle_client_connection_service.ready().await {
                    Err(e) => {
                        error!(
                            "Error happen when handle client connection [{}] on poll ready, error:{:#?}",
                            client_address, e
                        );
                        continue;
                    }
                    Ok(s) => {
                        if let Err(e) = s.call(ClientConnectionInfo{
                            client_stream, client_address
                        }).await {
                            error!(
                                "Error happen when handle client connection [{}], error:{:#?}",
                                client_address, e
                            )
                        }
                    }
                }
            }
        });
    }
}
