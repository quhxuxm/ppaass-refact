use std::{collections::VecDeque, net::SocketAddr, sync::Arc, time::Duration};

use common::{TcpConnectRequest, TcpConnectResult, TcpConnector};
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{mpsc, Mutex},
};
use tracing::{error, info};

use crate::config::AgentConfig;
use anyhow::anyhow;
use anyhow::Result;

use super::common::DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS;

const DEFAULT_INIT_PROXY_CONNECTION_NUMBER: usize = 32;
const DEFAULT_MIN_PROXY_CONNECTION_NUMBER: usize = 16;
const DEFAULT_PROXY_CONNECTION_NUMBER_INCREASEMENT: usize = 16;
const DEFAULT_PROXY_CONNECTION_CHECK_INTERVAL_SECONDS: u64 = 30;
pub struct ProxyConnectionPool {
    pool: Arc<Mutex<VecDeque<TcpStream>>>,
    proxy_addresses: Arc<Vec<SocketAddr>>,
    configuration: Arc<AgentConfig>,
}

impl ProxyConnectionPool {
    pub async fn new(proxy_addresses: Arc<Vec<SocketAddr>>, configuration: Arc<AgentConfig>) -> Result<Self> {
        let proxy_connection_check_interval_seconds = configuration
            .proxy_connection_check_interval_seconds()
            .unwrap_or(DEFAULT_PROXY_CONNECTION_CHECK_INTERVAL_SECONDS);
        let pool = Arc::new(Mutex::new(VecDeque::new()));
        let result = Self {
            proxy_addresses: proxy_addresses.clone(),
            configuration: configuration.clone(),
            pool,
        };
        {
            let mut locked_pool = result.pool.lock().await;
            Self::initialize_pool(proxy_addresses.clone(), configuration.clone(), &mut locked_pool).await?;
        }
        let pool_clone_for_timer = result.pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(proxy_connection_check_interval_seconds));
            loop {
                interval.tick().await;
                let mut pool = pool_clone_for_timer.lock().await;
                let mut remove_indexes = vec![];
                for (i, stream) in pool.iter_mut().enumerate() {
                    info!("Checking proxy connection: {:?}", stream);
                    if let Err(e) = stream.write(&[0u8; 0]).await {
                        error!(
                            "Proxy connection {:?} has error(write), mark it to be remove from the pool, error:{:?}.",
                            stream, e
                        );
                        remove_indexes.push(i);
                    };
                    if let Err(e) = stream.flush().await {
                        error!(
                            "Proxy connection {:?} has error(flush), mark it to be remove from the pool, error:{:?}.",
                            stream, e
                        );
                        remove_indexes.push(i);
                    };
                }
                for i in remove_indexes.iter() {
                    let target_stream = pool.remove(*i);
                    error!(
                        "Proxy connection {:?} has error, remove it from the pool, current pool size: {}.",
                        target_stream,
                        pool.len()
                    );
                    if let Some(mut target_stream) = target_stream {
                        if let Err(e) = target_stream.shutdown().await {
                            error!(
                                "Fail to close proxy connection {:?} because of error, current pool size: {}, error: {:#?}.",
                                target_stream,
                                pool.len(),
                                e
                            );
                        }
                        drop(target_stream);
                    }
                }

                if let Err(e) = Self::initialize_pool(proxy_addresses.clone(), configuration.clone(), &mut pool).await {
                    error!("Fail to initialize proxy connection pool because of error in timer, error: {:#?}", e);
                };
                info!("Current pool size: {}", pool.len());
            }
        });
        Ok(result)
    }

    async fn initialize_pool(proxy_addresses: Arc<Vec<SocketAddr>>, configuration: Arc<AgentConfig>, pool: &mut VecDeque<TcpStream>) -> Result<()> {
        let proxy_stream_so_linger = configuration.proxy_stream_so_linger().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS);
        let init_proxy_connection_number = configuration.init_proxy_connection_number().unwrap_or(DEFAULT_INIT_PROXY_CONNECTION_NUMBER);
        let proxy_connection_number_increasement = configuration
            .proxy_connection_number_increasement()
            .unwrap_or(DEFAULT_PROXY_CONNECTION_NUMBER_INCREASEMENT);
        let min_proxy_connection_number = configuration.min_proxy_connection_number().unwrap_or(DEFAULT_MIN_PROXY_CONNECTION_NUMBER);
        let pool_size = pool.len();
        if pool_size >= min_proxy_connection_number {
            return Ok(());
        }
        let (pool_initialize_channel_sender, mut pool_initialize_channel_receiver) = mpsc::channel(2048);
        let increase_to = if pool_size == 0 {
            init_proxy_connection_number
        } else {
            pool_size + proxy_connection_number_increasement
        };
        for _ in pool_size..increase_to {
            let proxy_addresses = proxy_addresses.clone();
            let pool_initialize_channel_sender = pool_initialize_channel_sender.clone();
            tokio::spawn(async move {
                match TcpConnector::connect(TcpConnectRequest {
                    connect_addresses: proxy_addresses.to_vec(),
                    connected_stream_so_linger: proxy_stream_so_linger,
                })
                .await
                {
                    Err(e) => {
                        error!(
                            "Fail to create proxy connection because of error, skip this connection, create anotherone, error:{:#?}",
                            e
                        );
                    },
                    Ok(TcpConnectResult { connected_stream }) => {
                        if let Err(e) = pool_initialize_channel_sender.send(connected_stream).await {
                            error!(
                                "Fail to create proxy connection because of error(sender), skip this connection, create anotherone, error:{:#?}",
                                e
                            );
                        }
                    },
                };
            });
        }
        drop(pool_initialize_channel_sender);
        while let Some(element) = pool_initialize_channel_receiver.recv().await {
            pool.push_back(element);
        }
        info!("Success to initialize proxy connection pool: {}", pool.len());
        Ok(())
    }

    pub async fn fetch_connection(&self) -> Result<TcpStream> {
        let min_proxy_connection_number = self.configuration.min_proxy_connection_number().unwrap_or(DEFAULT_MIN_PROXY_CONNECTION_NUMBER);
        let mut pool = self.pool.lock().await;
        let connection = pool.pop_front();
        info!("Fetch a proxy tcp connection from the pool, current pool size: {}", pool.len());
        let connection = match connection {
            None => {
                info!("Begin to fill the proxy connection pool(on empty), current pool size: {}", pool.len());
                Self::initialize_pool(self.proxy_addresses.clone(), self.configuration.clone(), &mut pool).await?;
                pool.pop_front().ok_or(anyhow!("Fail to initialize connection pool."))?
            },
            Some(v) => v,
        };
        if pool.len() < min_proxy_connection_number {
            info!("Begin to fill the proxy connection pool, current pool size: {}", pool.len());
            Self::initialize_pool(self.proxy_addresses.clone(), self.configuration.clone(), &mut pool).await?;
        }
        Ok(connection)
    }
}
