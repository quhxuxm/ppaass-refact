use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Result;
use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};

use tracing::{debug, error, instrument};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use crate::service::socks5::{Socks5FlowProcessor, Socks5FlowRequest};
use crate::service::{
    http::{HttpFlow, HttpFlowRequest},
    socks5::Socks5FlowResult,
};
use crate::{
    codec::{Protocol, SwitchClientProtocolDecoder},
    config::AgentConfig,
};

use super::{
    http::HttpFlowResult,
    pool::{ProxyConnection, ProxyConnectionPool},
};

pub const DEFAULT_BUFFER_SIZE: usize = 65535;

pub const DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS: u64 = 20;

#[derive(Debug)]
pub(crate) struct ClientConnection {
    pub id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

impl ClientConnection {
    #[instrument(skip(client_stream))]
    pub(crate) fn new(client_stream: TcpStream, client_address: SocketAddr) -> Self {
        Self {
            id: generate_uuid(),
            client_stream,
            client_address,
        }
    }

    #[instrument(skip_all)]
    pub async fn exec<T>(self, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<ProxyConnectionPool>) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let rsa_crypto_fetcher = rsa_crypto_fetcher.clone();
        let configuration = configuration.clone();
        let client_connection_id = self.id.clone();
        let mut client_stream = self.client_stream;
        let client_address = self.client_address;
        let mut framed = Framed::with_capacity(
            &mut client_stream,
            SwitchClientProtocolDecoder,
            configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
        );
        return match framed.next().await {
            None => Ok(()),
            Some(Err(e)) => {
                error!("Can not parse protocol from client input stream because of error: {:#?}.", e);
                Err(anyhow!(e))
            },
            Some(Ok(Protocol::Http)) => {
                let FramedParts { read_buf: buffer, .. } = framed.into_parts();
                let HttpFlowResult = HttpFlow::exec(
                    HttpFlowRequest {
                        client_connection_id: client_connection_id.as_str(),
                        client_stream,
                        client_address,
                        buffer,
                    },
                    rsa_crypto_fetcher.clone(),
                    configuration,
                    proxy_connection_pool,
                )
                .await?;
                debug!("Client connection [{}] complete http flow for client: {}", client_connection_id, client_address);
                Ok(())
            },
            Some(Ok(Protocol::Socks5)) => {
                let FramedParts { read_buf: buffer, .. } = framed.into_parts();
                let Socks5FlowResult = Socks5FlowProcessor::exec(
                    Socks5FlowRequest {
                        client_connection_id: client_connection_id.clone(),
                        client_stream,
                        client_address,
                        buffer,
                    },
                    rsa_crypto_fetcher,
                    configuration,
                    proxy_connection_pool,
                )
                .await?;
                debug!(
                    "Client connection [{}] complete socks5 flow for client: {}",
                    client_connection_id, client_address
                );
                Ok(())
            },
        };
    }
}

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct TcpRelayFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub client_connection_id: &'a str,
    pub proxy_connection_id: &'a str,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite<T, ProxyConnection>,
    pub message_framed_read: MessageFramedRead<T, ProxyConnection>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: SocketAddr,
}

#[allow(unused)]
pub(crate) struct TcpRelayFlowResult {
    pub client_address: SocketAddr,
}

#[derive(Default)]
pub(crate) struct TcpRelayFlow;

#[allow(unused)]
struct TcpRelayProxyToClientRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    connection_id: &'a str,
    target_address: NetAddress,
    message_framed_read: MessageFramedRead<T, ProxyConnection>,
    client_stream_write: OwnedWriteHalf,
}

#[allow(unused)]
struct TcpRelayClientToProxyRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    connection_id: &'a str,
    init_data: Option<Vec<u8>>,
    message_framed_write: MessageFramedWrite<T, ProxyConnection>,
    source_address: NetAddress,
    target_address: NetAddress,
    client_stream_read: OwnedReadHalf,
}

impl TcpRelayFlow {
    pub async fn exec<'a, T>(
        TcpRelayFlowRequest {
            client_connection_id,
            client_stream,
            client_address,
            message_framed_write,
            message_framed_read,
            source_address,
            target_address,
            init_data,
            ..
        }: TcpRelayFlowRequest<'a, T>,
        configuration: Arc<AgentConfig>,
    ) -> Result<TcpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let (client_stream_read, client_stream_write) = client_stream.into_split();
        {
            let connection_id = client_connection_id.to_owned();
            let target_address = target_address.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::relay_proxy_to_client(TcpRelayProxyToClientRequest {
                    connection_id: &connection_id,
                    client_stream_write,
                    message_framed_read,
                    target_address,
                })
                .await
                {
                    error!("Error happen when relay data from proxy to client, error: {e:#?}");
                }
            });
        }
        {
            let connection_id = client_connection_id.to_owned();
            tokio::spawn(async move {
                if let Err(e) = Self::relay_client_to_proxy(
                    TcpRelayClientToProxyRequest {
                        connection_id: &connection_id,
                        init_data,
                        message_framed_write,
                        source_address,
                        target_address,
                        client_stream_read,
                    },
                    configuration,
                )
                .await
                {
                    error!("Error happen when relay data from client to proxy, error: {e:#?}");
                }
            });
        }
        Ok(TcpRelayFlowResult { client_address })
    }

    async fn relay_client_to_proxy<'a, T>(
        TcpRelayClientToProxyRequest {
            connection_id,
            init_data,
            mut message_framed_write,
            source_address,
            target_address,
            mut client_stream_read,
        }: TcpRelayClientToProxyRequest<'a, T>,
        configuration: Arc<AgentConfig>,
    ) -> Result<()>
    where
        T: RsaCryptoFetcher,
    {
        let user_token = configuration.user_token().clone().unwrap();
        if let Some(init_data) = init_data {
            let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                encryption_token: generate_uuid().into(),
                user_token: user_token.as_str(),
            })
            .await
            {
                Err(e) => {
                    error!("Fail to select payload encryption type because of error: {:#?}", e);
                    return Err(e.into());
                },
                Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                connection_id: Some(connection_id),
                message_framed_write,
                ref_id: Some(connection_id),
                user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
                payload_encryption_type,
                message_payloads: Some(vec![MessagePayload {
                    source_address: Some(source_address.clone()),
                    target_address: Some(target_address.clone()),
                    payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                    data: Some(init_data.into()),
                }]),
            })
            .await;
            message_framed_write = match write_agent_message_result {
                Err(WriteMessageFramedError { source, .. }) => {
                    error!("Fail to write agent message to proxy because of error: {source:#?}");
                    return Err(source.into());
                },
                Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
            };
        }
        loop {
            let client_buffer_size = configuration.client_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let mut client_buffer = BytesMut::with_capacity(client_buffer_size);
            match client_stream_read.read_buf(&mut client_buffer).await {
                Err(e) => {
                    error!(
                        "Error happen when relay data from client to proxy,  agent address={:?}, target address={:?}, error: {:#?}",
                        source_address, target_address, e
                    );
                    return Err(e.into());
                },
                Ok(0) => {
                    debug!("Read all data from client, target address: {:?}", target_address);
                    message_framed_write.flush().await?;
                    message_framed_write.close().await?;
                    return Ok(());
                },
                Ok(size) => {
                    debug!("Read {} bytes from client, target address: {:?}", size, target_address);
                    size
                },
            };
            let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                encryption_token: generate_uuid().into(),
                user_token: user_token.as_str(),
            })
            .await
            {
                Err(e) => {
                    error!("Fail to select payload encryption type because of error: {:#?}", e);
                    return Err(e.into());
                },
                Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            let payload_data = client_buffer.split().freeze();
            let payload_data_chunks = payload_data.chunks(configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE));
            let mut payloads = vec![];
            for (_, chunk) in payload_data_chunks.enumerate() {
                let chunk_data = Bytes::copy_from_slice(chunk);
                let payload = MessagePayload {
                    source_address: Some(source_address.clone()),
                    target_address: Some(target_address.clone()),
                    payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                    data: Some(chunk_data),
                };
                payloads.push(payload)
            }
            let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                connection_id: Some(connection_id),
                message_framed_write,
                ref_id: Some(connection_id),
                user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
                payload_encryption_type: payload_encryption_type.clone(),
                message_payloads: Some(payloads),
            })
            .await;
            message_framed_write = match write_agent_message_result {
                Err(WriteMessageFramedError { source, .. }) => {
                    error!("Fail to write agent message to proxy because of error: {:#?}", source);
                    return Err(source.into());
                },
                Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
            };
        }
    }

    async fn relay_proxy_to_client<'a, T>(
        TcpRelayProxyToClientRequest {
            connection_id,
            mut message_framed_read,
            mut client_stream_write,
            ..
        }: TcpRelayProxyToClientRequest<'a, T>,
    ) -> Result<()>
    where
        T: RsaCryptoFetcher + Debug,
    {
        loop {
            let mut proxy_raw_data = match MessageFramedReader::read(ReadMessageFramedRequest {
                connection_id,
                message_framed_read,
                timeout: None,
            })
            .await
            {
                Err(ReadMessageFramedError { source, .. }) => {
                    return Err(source.into());
                },
                Ok(ReadMessageFramedResult {
                    message_framed_read: message_framed_read_pass_back,
                    content:
                        Some(ReadMessageFramedResultContent {
                            message_payload:
                                Some(MessagePayload {
                                    data: Some(data),
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                                    ..
                                }),
                            ..
                        }),
                    ..
                }) => {
                    message_framed_read = message_framed_read_pass_back;
                    data
                },
                Ok(ReadMessageFramedResult { content: None, .. }) => {
                    client_stream_write.flush().await?;
                    return Ok(());
                },
                Ok(ReadMessageFramedResult { .. }) => return Err(anyhow!("Connection [{connection_id}] read invalid data from proxy.")),
            };

            client_stream_write.write_all_buf(&mut proxy_raw_data).await?;
            client_stream_write.flush().await?;
        }
    }
}
