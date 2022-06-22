use std::{collections::VecDeque, net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedGenerateResult, MessageFramedGenerator, MessageFramedReader, MessageFramedWriter, MessagePayload,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, TcpConnectRequest,
    TcpConnectResult, TcpConnector, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, Mutex},
};
use tokio_util::codec::FramedParts;
use tracing::{error, info};

use crate::config::AgentConfig;
use anyhow::anyhow;
use anyhow::Result;

use super::common::{DEFAULT_BUFFER_SIZE, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS};

const DEFAULT_INIT_PROXY_CONNECTION_NUMBER: usize = 32;
const DEFAULT_MIN_PROXY_CONNECTION_NUMBER: usize = 16;
const DEFAULT_PROXY_CONNECTION_NUMBER_INCREASEMENT: usize = 16;
const DEFAULT_PROXY_CONNECTION_CHECK_INTERVAL_SECONDS: u64 = 30;
pub struct ProxyConnectionPool {
    pool: Arc<Mutex<VecDeque<Option<TcpStream>>>>,
    proxy_addresses: Arc<Vec<SocketAddr>>,
    configuration: Arc<AgentConfig>,
}

impl ProxyConnectionPool {
    pub async fn new<T>(proxy_addresses: Arc<Vec<SocketAddr>>, configuration: Arc<AgentConfig>, rsa_crypto_fetcher: Arc<T>) -> Result<Self>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let proxy_connection_check_interval_seconds = configuration
            .proxy_connection_check_interval_seconds()
            .unwrap_or(DEFAULT_PROXY_CONNECTION_CHECK_INTERVAL_SECONDS);
        let buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);
        let user_token = configuration.user_token().clone().expect("No user token configured.");
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
                // let available_streams = vec![];
                let (available_stream_sender, mut available_stream_receiver) = mpsc::channel::<TcpStream>(2048);
                for stream in pool.iter_mut() {
                    let stream_identifier = format!("{:?}", stream);
                    info!("Checking proxy connection: {}", stream_identifier);
                    let stream = match std::mem::take(stream) {
                        None => {
                            error!("Fail to take stream [{}] from pool because of the element is None", stream_identifier);
                            return;
                        },
                        Some(s) => s,
                    };
                    let user_token = user_token.clone();
                    let rsa_crypto_fetcher = rsa_crypto_fetcher.clone();
                    let available_stream_sender = available_stream_sender.clone();
                    tokio::spawn(async move {
                        let MessageFramedGenerateResult {
                            message_framed_write,
                            message_framed_read,
                        } = MessageFramedGenerator::generate(stream, buffer_size, compress, rsa_crypto_fetcher.clone()).await;
                        let PayloadEncryptionTypeSelectResult {
                            user_token,
                            payload_encryption_type,
                            ..
                        } = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            user_token: user_token.clone(),
                            encryption_token: generate_uuid().into(),
                        })
                        .await
                        {
                            Err(e) => {
                                error!("Stream [{}] check fail because of error: {:#?}", stream_identifier, e);
                                return;
                            },
                            Ok(v) => v,
                        };
                        let heartbeat_message_payload = MessagePayload {
                            data: Bytes::new(),
                            source_address: None,
                            target_address: None,
                            payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat),
                        };
                        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                            connection_id: None,
                            message_framed_write,
                            message_payload: Some(heartbeat_message_payload),
                            payload_encryption_type,
                            ref_id: None,
                            user_token: user_token.clone(),
                        })
                        .await
                        {
                            Err(WriteMessageFramedError { source, .. }) => {
                                error!("Stream [{}] check fail because of error(heartbeat write): {:#?}", stream_identifier, source);
                                return;
                            },
                            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                        };
                        match MessageFramedReader::read(ReadMessageFramedRequest {
                            connection_id: stream_identifier.clone(),
                            message_framed_read,
                        })
                        .await
                        {
                            Err(ReadMessageFramedError { source, .. }) => {
                                error!("Stream [{}] check fail because of error(heartbeat read): {:#?}", stream_identifier, source);
                                return;
                            },
                            Ok(ReadMessageFramedResult {
                                message_framed_read,
                                content:
                                    Some(ReadMessageFramedResultContent {
                                        message_payload:
                                            Some(MessagePayload {
                                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::HeartbeatSuccess),
                                                ..
                                            }),
                                        ..
                                    }),
                            }) => {
                                info!("Heartbeat for proxy stream [{}] success.", stream_identifier);
                                let framed = match message_framed_read.reunite(message_framed_write) {
                                    Ok(f) => f,
                                    Err(e) => {
                                        error!("Stream [{}] check fail because of error(merge read and write): {:#?}", stream_identifier, e);
                                        return;
                                    },
                                };
                                let FramedParts { io: stream, .. } = framed.into_parts();
                                if let Err(e) = available_stream_sender.send(stream).await {
                                    error!("Stream [{}] check fail because of error(send available stream): {:#?}", stream_identifier, e);
                                    return;
                                };
                                return;
                            },
                            Ok(ReadMessageFramedResult { .. }) => {
                                error!("Stream [{}] check fail because of closed(heartbeat read)", stream_identifier);
                                return;
                            },
                        }
                    });
                }
                pool.clear();
                drop(available_stream_sender);
                while let Some(stream) = available_stream_receiver.recv().await {
                    pool.push_back(Some(stream));
                }
                if let Err(e) = Self::initialize_pool(proxy_addresses.clone(), configuration.clone(), &mut pool).await {
                    error!("Fail to initialize proxy connection pool because of error in timer, error: {:#?}", e);
                };
                info!("Current pool size: {}", pool.len());
            }
        });
        Ok(result)
    }

    async fn initialize_pool(proxy_addresses: Arc<Vec<SocketAddr>>, configuration: Arc<AgentConfig>, pool: &mut VecDeque<Option<TcpStream>>) -> Result<()> {
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
            pool.push_back(Some(element));
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
                pool.pop_front()
                    .ok_or(anyhow!("Fail to initialize connection pool."))?
                    .ok_or(anyhow!("Fail to fetch connection from to pool because of the element is None"))?
            },
            Some(None) => {
                return Err(anyhow!("Fail to fetch connection from to pool because of the element is None"));
            },
            Some(Some(v)) => v,
        };
        if pool.len() < min_proxy_connection_number {
            info!("Begin to fill the proxy connection pool, current pool size: {}", pool.len());
            Self::initialize_pool(self.proxy_addresses.clone(), self.configuration.clone(), &mut pool).await?;
        }
        Ok(connection)
    }
}
