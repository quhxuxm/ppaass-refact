use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tower::ServiceBuilder;
use tracing::{error, info};

use common::ready_and_call_service;

use crate::config::SERVER_CONFIG;
use crate::service::common::{ClientConnectionInfo, HandleClientConnectionService};

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
            }
            Ok(r) => r,
        };
        Self { runtime }
    }

    pub(crate) fn run(&self) {
        self.runtime.block_on(async {

            let std_listener = match std::net::TcpListener::bind(SocketAddrV4::new(
                Ipv4Addr::new(0, 0, 0, 0),
                SERVER_CONFIG.port().unwrap_or(DEFAULT_SERVER_PORT),
            )) {
                Err(e) => {
                    panic!("Fail to bind agent server port because of error: {:#?}", e);
                }
                Ok(listener) => {
                    info!("Success to bind agent server port, start listening ... ");
                    listener
                }
            };
            if let Err(e) = std_listener.set_nonblocking(true) {
                panic!(
                    "Fail to set agent server listener to be non-blocking because of error: {:#?}",
                    e
                );
            };

            let listener = match TcpListener::from_std(std_listener) {
                Err(e) => {
                    panic!("Fail to generate agent server listener from std listener because of error: {:#?}", e);
                }
                Ok(listener) => {
                    info!("Success to generate agent server listener.");
                    listener
                }
            };

            loop {
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
                tokio::spawn(async move {
                    let mut handle_client_connection_service = ServiceBuilder::new()
                        .buffer(SERVER_CONFIG.buffered_connection_number().unwrap_or(1024))
                        .concurrency_limit(
                            SERVER_CONFIG.concurrent_connection_number().unwrap_or(1024),
                        )
                        .rate_limit(
                            SERVER_CONFIG.rate_limit().unwrap_or(1024),
                            Duration::from_secs(1),
                        )
                        .service::<HandleClientConnectionService>(Default::default());
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
