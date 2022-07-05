use std::{
    collections::VecDeque,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use std::{fmt::Debug, ops::Deref};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedGenerateResult, MessageFramedGenerator, MessageFramedReader, MessageFramedWriter, MessagePayload,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, TcpConnectRequest,
    TcpConnectResult, TcpConnector, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    sync::{mpsc, Mutex},
};
use tokio_util::codec::FramedParts;
use tracing::{debug, debug_span, error, info, instrument, Instrument};

use crate::config::AgentConfig;
use anyhow::anyhow;
use anyhow::Result;
use pin_project::pin_project;
use pin_project::pinned_drop;

use super::common::{DEFAULT_BUFFER_SIZE, DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS};

const DEFAULT_INIT_PROXY_CONNECTION_NUMBER: usize = 32;
const DEFAULT_MIN_PROXY_CONNECTION_NUMBER: usize = 16;
const DEFAULT_PROXY_CONNECTION_NUMBER_INCREMENTAL: usize = 16;
const DEFAULT_PROXY_CONNECTION_CHECK_INTERVAL_SECONDS: u64 = 30;

#[derive(Debug)]
#[pin_project(PinnedDrop)]
pub struct ProxyConnection {
    pub id: String,
    #[pin]
    pub stream: TcpStream,
    pub active_instant: Instant,
}

impl Deref for ProxyConnection {
    type Target = TcpStream;

    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

#[pinned_drop]
impl PinnedDrop for ProxyConnection {
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();
        drop(this.stream);
        info!("Proxy connection [{}] closed.", this.id)
    }
}

impl AsyncRead for ProxyConnection {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = self.project();
        this.stream.poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyConnection {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        let this = self.project();
        this.stream.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        this.stream.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        this.stream.poll_shutdown(cx)
    }
}

impl PartialOrd for ProxyConnection {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.active_instant.partial_cmp(&other.active_instant)
    }
}

impl PartialEq for ProxyConnection {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Ord for ProxyConnection {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.active_instant.cmp(&other.active_instant)
    }
}

impl Eq for ProxyConnection {}

#[derive(Debug)]
pub struct ProxyConnectionPool {
    pool: Arc<Mutex<VecDeque<Option<ProxyConnection>>>>,
    proxy_addresses: Arc<Vec<SocketAddr>>,
    configuration: Arc<AgentConfig>,
}

impl ProxyConnectionPool {
    #[instrument(skip_all, fields(proxy_addresses))]
    pub async fn new<T>(proxy_addresses: Arc<Vec<SocketAddr>>, configuration: Arc<AgentConfig>, rsa_crypto_fetcher: Arc<T>) -> Result<Self>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let proxy_connection_check_interval_seconds = configuration
            .proxy_connection_check_interval_seconds()
            .unwrap_or(DEFAULT_PROXY_CONNECTION_CHECK_INTERVAL_SECONDS);
        let buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);
        let user_token = configuration.user_token().clone().expect("No user token configured.");
        let pool = Arc::new(Mutex::new(VecDeque::<Option<ProxyConnection>>::new()));
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
        tokio::spawn(
            async move {
                let mut interval = tokio::time::interval(Duration::from_secs(proxy_connection_check_interval_seconds));
                loop {
                    {
                        let mut pool = pool_clone_for_timer.lock().await;
                        let (available_connection_sender, mut available_connection_receiver) = mpsc::channel::<ProxyConnection>(2048);
                        for connection in pool.iter_mut() {
                            let connection = std::mem::take(connection);
                            let connection = match connection {
                                None => continue,
                                Some(v) => v,
                            };
                            let connection_id = connection.id.clone();
                            debug!("Checking proxy connection: {connection_id}");
                            let user_token = user_token.clone();
                            let rsa_crypto_fetcher = rsa_crypto_fetcher.clone();
                            let available_stream_sender_for_single_connection = available_connection_sender.clone();
                            let proxy_connection_check_timeout = configuration.proxy_connection_check_timeout().clone();
                            tokio::spawn(async move {
                                let MessageFramedGenerateResult {
                                    message_framed_write,
                                    message_framed_read,
                                } = MessageFramedGenerator::generate(connection, buffer_size, compress, rsa_crypto_fetcher.clone()).await;
                                let PayloadEncryptionTypeSelectResult {
                                    user_token,
                                    payload_encryption_type,
                                    ..
                                } = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                                    user_token: user_token.as_str(),
                                    encryption_token: generate_uuid().into(),
                                })
                                .await
                                {
                                    Err(e) => {
                                        error!("Connection [{}] check fail because of error: {:#?}", connection_id, e);
                                        return;
                                    },
                                    Ok(v) => v,
                                };
                                let heartbeat_message_payload = MessagePayload {
                                    data: None,
                                    source_address: None,
                                    target_address: None,
                                    payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat),
                                };
                                let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                                    connection_id: None,
                                    message_framed_write,
                                    message_payloads: Some(vec![heartbeat_message_payload]),
                                    payload_encryption_type,
                                    ref_id: None,
                                    user_token: user_token.as_str(),
                                })
                                .await
                                {
                                    Err(WriteMessageFramedError { source, .. }) => {
                                        error!("Connection [{}] check fail because of error(heartbeat write): {:#?}", connection_id, source);
                                        return;
                                    },
                                    Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                                };
                                match MessageFramedReader::read(ReadMessageFramedRequest {
                                    connection_id: connection_id.as_str(),
                                    message_framed_read,
                                    timeout: proxy_connection_check_timeout,
                                })
                                .await
                                {
                                    Err(ReadMessageFramedError { source, .. }) => {
                                        error!("Connection [{}] check fail because of error(heartbeat read): {:#?}", connection_id, source);
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
                                        info!("Heartbeat for proxy connection [{}] success.", connection_id);
                                        let framed = match message_framed_read.reunite(message_framed_write) {
                                            Ok(f) => f,
                                            Err(e) => {
                                                error!("Connection [{}] check fail because of error(merge read and write): {:#?}", connection_id, e);
                                                return;
                                            },
                                        };
                                        let FramedParts { io: mut connection, .. } = framed.into_parts();
                                        connection.active_instant = Instant::now();
                                        if let Err(e) = available_stream_sender_for_single_connection.send(connection).await {
                                            error!("Connection [{}] check fail because of error(send available stream): {:#?}", connection_id, e);
                                            return;
                                        };
                                        return;
                                    },
                                    Ok(ReadMessageFramedResult { .. }) => {
                                        error!("Connection [{}] check fail because of closed(heartbeat read)", connection_id);
                                        return;
                                    },
                                }
                            });
                        }
                        pool.clear();
                        drop(available_connection_sender);
                        while let Some(connection) = available_connection_receiver.recv().await {
                            pool.push_back(Some(connection));
                        }
                        if let Err(e) = Self::initialize_pool(proxy_addresses.clone(), configuration.clone(), &mut pool).await {
                            error!("Fail to initialize proxy connection pool because of error in timer, error: {:#?}", e);
                        };
                        debug!("Current pool size: {}", pool.len());
                    }
                    interval.tick().await;
                }
            }
            .instrument(debug_span!("PROXY_CONNECTION_POOL_HEARTBEAT_TIMER")),
        );
        Ok(result)
    }

    #[instrument(skip_all, fields(proxy_addresses))]
    async fn initialize_pool(
        proxy_addresses: Arc<Vec<SocketAddr>>, configuration: Arc<AgentConfig>, pool: &mut VecDeque<Option<ProxyConnection>>,
    ) -> Result<()> {
        let proxy_stream_so_linger = configuration.proxy_stream_so_linger().unwrap_or(DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS);
        let init_proxy_connection_number = configuration.init_proxy_connection_number().unwrap_or(DEFAULT_INIT_PROXY_CONNECTION_NUMBER);
        let proxy_connection_number_incremental = configuration
            .proxy_connection_number_increasement()
            .unwrap_or(DEFAULT_PROXY_CONNECTION_NUMBER_INCREMENTAL);
        let min_proxy_connection_number = configuration.min_proxy_connection_number().unwrap_or(DEFAULT_MIN_PROXY_CONNECTION_NUMBER);
        let pool_size = pool.len();
        if pool_size >= min_proxy_connection_number {
            return Ok(());
        }
        let (pool_initialize_channel_sender, mut pool_initialize_channel_receiver) = mpsc::channel(2048);
        let increase_to = if pool_size == 0 {
            init_proxy_connection_number
        } else {
            pool_size + proxy_connection_number_incremental
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
                        let proxy_connection_id = generate_uuid();
                        let proxy_connection = ProxyConnection {
                            id: proxy_connection_id.clone(),
                            stream: connected_stream,
                            active_instant: Instant::now(),
                        };
                        if let Err(e) = pool_initialize_channel_sender.send(proxy_connection).await {
                            error!(
                                "Fail to create proxy connection [{}] because of error(sender), skip this connection, create anotherone, error:{:#?}",
                                proxy_connection_id, e
                            );
                        }
                        debug!("Success to create proxy connection : {}", proxy_connection_id);
                    },
                };
            });
        }
        drop(pool_initialize_channel_sender);
        while let Some(connection) = pool_initialize_channel_receiver.recv().await {
            pool.push_back(Some(connection));
        }
        pool.make_contiguous().sort();
        debug!("Success to initialize proxy connection pool: {}", pool.len());
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn fetch_connection(&self) -> Result<ProxyConnection> {
        let min_proxy_connection_number = self.configuration.min_proxy_connection_number().unwrap_or(DEFAULT_MIN_PROXY_CONNECTION_NUMBER);
        let mut pool = self.pool.lock().await;
        let connection = pool.pop_front();
        debug!("Fetch a proxy tcp connection from the pool, current pool size: {}", pool.len());
        let connection = match connection {
            None => {
                debug!("Begin to fill the proxy connection pool(on empty), current pool size: {}", pool.len());
                Self::initialize_pool(self.proxy_addresses.clone(), self.configuration.clone(), &mut pool).await?;
                pool.pop_front()
                    .ok_or(anyhow!("Fail to fetch connection from to pool because of the element is None"))?
                    .ok_or(anyhow!("Fail to fetch connection from to pool because of the element is None"))?
            },
            Some(None) => return Err(anyhow!("Fail to fetch connection from to pool because of the element is None")),
            Some(Some(v)) => v,
        };
        if pool.len() < min_proxy_connection_number {
            debug!("Begin to fill the proxy connection pool, current pool size: {}", pool.len());
            Self::initialize_pool(self.proxy_addresses.clone(), self.configuration.clone(), &mut pool).await?;
        }
        Ok(connection)
    }
}
