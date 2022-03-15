use std::{
    net::{Ipv4Addr, SocketAddrV4},
    pin::Pin,
    process::Command,
};

use futures_util::Future;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tower::{Layer, Service, ServiceBuilder};
use tracing::{error, info};

use common::CommonError;

use crate::service::common::{ClientConnection, HandleClientConnectionService};

pub(crate) struct AgentServer {
    runtime: Runtime,
}

impl AgentServer {
    pub(crate) fn new() -> Self {
        let runtime = match tokio::runtime::Runtime::new() {
            Err(e) => {
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
            let listener = match TcpListener::bind(SocketAddrV4::new(
                Ipv4Addr::new(0, 0, 0, 0),
                10081,
            ))
            .await
            {
                Err(e) => {
                    panic!("Fail to bind agent because of error: {:#?}", e);
                }
                Ok(listener) => {
                    info!("Success to bind agent, start listening ... ");
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
                let client_connection = ClientConnection::new(client_stream, client_address);
                let mut handle_client_connection_service = ServiceBuilder::new()
                    // .buffer(100)
                    // .concurrency_limit(10)
                    .service(HandleClientConnectionService);
                tokio::runtim
                handle_client_connection_service.poll_ready()
                if let Err(e) = handle_client_connection_service
                    .call(client_connection)
                    .await
                {
                    error!(
                        "Error happen when handle client connection [{}], error:{:#?}",
                        client_address, e
                    )
                }
            }
        });
    }
}
