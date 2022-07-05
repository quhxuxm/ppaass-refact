use std::time::Duration;
use std::{net::SocketAddr, sync::mpsc::Receiver};
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

use crate::{
    config::ProxyConfig,
    service::{AgentConnection, ProxyRsaCryptoFetcher},
};
use std::thread::JoinHandle as StdJoinHandler;
use tokio::{net::TcpSocket, runtime::Builder as TokioRuntimeBuilder};

use anyhow::anyhow;
use anyhow::Result;
use tracing::{debug, error, info, instrument};

const DEFAULT_SERVER_PORT: u16 = 80;

#[derive(Debug)]
pub(crate) enum ProxyServerSignal {
    Startup { configuration: ProxyConfig },
    Shutdown,
}
#[derive(Debug)]
pub(crate) struct ProxyServer {
    signal_receiver: Receiver<ProxyServerSignal>,
}

impl ProxyServer {
    #[instrument(skip_all)]
    pub(crate) fn new(signal_receiver: Receiver<ProxyServerSignal>) -> Result<Self> {
        Ok(Self { signal_receiver })
    }

    pub(crate) fn run(self) -> Result<StdJoinHandler<()>> {
        let signal_receiver = self.signal_receiver;
        let signal = signal_receiver.recv();
        match signal {
            Err(e) => {
                println!("Proxy server going to shutdown because of error: {:#?}.", e);
                error!("Proxy server going to shutdown because of error: {:#?}.", e);
                return Err(anyhow!("Proxy server going to shutdown because of error: {:#?}.", e));
            },
            Ok(ProxyServerSignal::Shutdown) => {
                println!("Proxy server going to shutdown.");
                info!("Proxy server going to shutdown.");
                return Err(anyhow!("Proxy server going to shutdown."));
            },
            Ok(ProxyServerSignal::Startup { configuration }) => {
                println!("Proxy server going to startup.");
                info!("Proxy server going to startup.");
                let configuration = Arc::new(configuration);

                let mut runtime_builder = TokioRuntimeBuilder::new_multi_thread();
                runtime_builder
                    .enable_all()
                    .thread_keep_alive(Duration::from_secs(configuration.thread_timeout().unwrap_or(2)))
                    .max_blocking_threads(configuration.max_blocking_threads().unwrap_or(32))
                    .worker_threads(configuration.thread_number().unwrap_or(1024));

                let runtime = runtime_builder.build()?;
                let proxy_rsa_crypto_fetcher = match ProxyRsaCryptoFetcher::new(&configuration) {
                    Err(e) => {
                        error!("Fail to start up proxy server because of error: {:#?}", e);
                        return Err(anyhow!("Fail to start up proxy server because of error: {:#?}", e));
                    },
                    Ok(v) => v,
                };
                let proxy_rsa_crypto_fetcher = Arc::new(proxy_rsa_crypto_fetcher);
                runtime.spawn(async move {
                    let server_socket = match TcpSocket::new_v4() {
                        Err(e) => {
                            error!("Fail to initialize server tcp socket because of error: {:#?}", e);
                            return;
                        },
                        Ok(v) => v,
                    };
                    if let Err(e) = server_socket.set_reuseaddr(true) {
                        error!("Fail to initialize server tcp socket reuse addr because of error: {:#?}", e);
                        return;
                    };
                    if let Some(so_recv_buffer_size) = configuration.so_recv_buffer_size() {
                        if let Err(e) = server_socket.set_recv_buffer_size(so_recv_buffer_size) {
                            error!("Fail to initialize server tcp socket recv_buffer_size because of error: {:#?}", e);
                            return;
                        };
                    }
                    if let Some(so_send_buffer_size) = configuration.so_send_buffer_size() {
                        if let Err(e) = server_socket.set_send_buffer_size(so_send_buffer_size) {
                            error!("Fail to initialize server tcp socket send_buffer_size because of error: {:#?}", e);
                            return;
                        }
                    }
                    let local_socket_address = SocketAddr::V4(SocketAddrV4::new(
                        Ipv4Addr::new(0, 0, 0, 0),
                        configuration.port().unwrap_or(DEFAULT_SERVER_PORT),
                    ));
                    if let Err(e) = server_socket.bind(local_socket_address) {
                        error!("Fail to bind server tcp socket on address {} because of error: {:#?}", local_socket_address, e);
                        return;
                    }
                    let listener = match server_socket.listen(configuration.so_backlog().unwrap_or(1024)) {
                        Err(e) => {
                            error!("Fail to make tcp server tcp socket listen because of error: {:#?}", e);
                            return;
                        },
                        Ok(v) => v,
                    };

                    println!("ppaass-proxy is listening port: {} ", local_socket_address.port());
                    info!("ppaass-proxy is listening port: {} ", local_socket_address.port());
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
                        if let Some(agent_stream_so_linger) = configuration.agent_stream_so_linger() {
                            if let Err(e) = agent_stream.set_linger(Some(Duration::from_secs(agent_stream_so_linger))) {
                                error!("Fail to set agent connection linger because of error: {:#?}", e);
                                continue;
                            }
                        };

                        let proxy_rsa_crypto_fetcher = proxy_rsa_crypto_fetcher.clone();

                        let configuration = configuration.clone();
                        tokio::spawn(async move {
                            let agent_connection = AgentConnection::new(agent_stream, agent_address);
                            let agent_connection_id = agent_connection.get_id().to_owned();
                            if let Err(e) = agent_connection.exec(proxy_rsa_crypto_fetcher, configuration).await {
                                error!(
                                    "Error happen when handle agent connection: [{}], agent address:[{}], error:{:#?}",
                                    agent_connection_id, agent_address, e
                                );
                                return;
                            }
                            debug!("Agent connection [{agent_connection_id}] complete exec successfully.");
                        });
                    }
                });
                let guard = std::thread::spawn(move || loop {
                    match signal_receiver.recv() {
                        Ok(ProxyServerSignal::Shutdown) => {
                            println!("Proxy server going to shutdown.");
                            info!("Proxy server going to shutdown.");
                            runtime.shutdown_timeout(Duration::from_secs(60));
                            return;
                        },
                        Ok(other_sighal) => {
                            info!("Ignore other single when proxy server is running: {:?}", other_sighal);
                        },
                        Err(e) => {
                            error!("Fail to receive proxy server signal because of error: {:#?}", e);
                            runtime.shutdown_timeout(Duration::from_secs(60));
                            return;
                        },
                    }
                });
                Ok(guard)
            },
        }
    }
}
