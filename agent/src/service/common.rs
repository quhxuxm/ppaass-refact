use std::sync::Arc;
use std::{io::ErrorKind, net::SocketAddr};

use anyhow::anyhow;
use anyhow::Result;
use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};

use tracing::{debug, error};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher,
    WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
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

use super::{http::HttpFlowResult, pool::ProxyConnectionPool};

pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;

pub const DEFAULT_CONNECT_PROXY_TIMEOUT_SECONDS: u64 = 20;

pub(crate) struct ClientConnection {
    pub id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

impl ClientConnection {
    pub(crate) fn new(client_stream: TcpStream, client_address: SocketAddr) -> Self {
        Self {
            id: generate_uuid(),
            client_stream,
            client_address,
        }
    }

    pub async fn exec<T>(self, rsa_crypto_fetcher: Arc<T>, confiugration: Arc<AgentConfig>, proxy_connection_pool: Arc<ProxyConnectionPool>) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let rsa_crypto_fetcher = rsa_crypto_fetcher.clone();
        let configuration = confiugration.clone();
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
                        client_connection_id: client_connection_id.clone(),
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
pub(crate) struct TcpRelayFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub client_connection_id: String,
    pub proxy_connection_id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: SocketAddr,
}

#[allow(unused)]
pub(crate) struct TcpRelayFlowResult {
    pub client_address: SocketAddr,
}

#[allow(unused)]
struct TcpRelayC2PError<T>
where
    T: RsaCryptoFetcher,
{
    message_framed_write: MessageFramedWrite<T>,
    client_stream_read: OwnedReadHalf,
    source: anyhow::Error,
    connection_closed: bool,
}

#[allow(unused)]
struct TcpRelayP2CError<T>
where
    T: RsaCryptoFetcher,
{
    message_framed_read: MessageFramedRead<T>,
    client_stream_write: OwnedWriteHalf,
    source: anyhow::Error,
    connection_closed: bool,
}

#[derive(Default)]
pub(crate) struct TcpRelayFlow;

impl TcpRelayFlow {
    pub async fn exec<T>(request: TcpRelayFlowRequest<T>, configuration: Arc<AgentConfig>) -> Result<TcpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let TcpRelayFlowRequest {
            client_connection_id,
            client_stream,
            client_address,
            message_framed_write,
            message_framed_read,
            source_address,
            target_address,
            init_data,
            ..
        } = request;

        let (client_stream_read, client_stream_write) = client_stream.into_split();
        let connection_id_p2c = client_connection_id.clone();
        let target_address_p2c = target_address.clone();
        let configuration_p2c = configuration.clone();

        tokio::spawn(async move {
            if let Err(TcpRelayP2CError {
                mut client_stream_write,
                source,
                connection_closed,
                ..
            }) = Self::relay_proxy_to_client(
                connection_id_p2c,
                target_address_p2c,
                message_framed_read,
                client_stream_write,
                configuration_p2c,
            )
            .await
            {
                error!("Error happen when relay data from proxy to client, error: {:#?}", source);
                if let Err(e) = client_stream_write.flush().await {
                    error!("Fail to flush client stream writer when relay data from proxy to client have error:{:#?}", e);
                };
                if !connection_closed {
                    if let Err(e) = client_stream_write.shutdown().await {
                        error!("Fail to shutdown client stream writer when relay data from proxy to client have error:{:#?}", e);
                    };
                }
                drop(client_stream_write);
            }
        });
        tokio::spawn(async move {
            if let Err(TcpRelayC2PError {
                mut message_framed_write,
                source,
                connection_closed,
                ..
            }) = Self::relay_client_to_proxy(
                client_connection_id,
                init_data,
                message_framed_write,
                source_address,
                target_address,
                client_stream_read,
                configuration,
            )
            .await
            {
                error!("Error happen when relay data from client to proxy, error: {:#?}", source);
                if let Err(e) = message_framed_write.flush().await {
                    error!("Fail to flush proxy message writer when relay data from client to proxy have error:{:#?}", e);
                };
                if !connection_closed {
                    if let Err(e) = message_framed_write.close().await {
                        error!("Fail to close proxy message writer when relay data from client to proxy have error:{:#?}", e);
                    };
                }
                drop(message_framed_write);
            }
        });

        Ok(TcpRelayFlowResult { client_address })
    }

    async fn relay_client_to_proxy<T>(
        connection_id: String, init_data: Option<Vec<u8>>, mut message_framed_write: MessageFramedWrite<T>, source_address_a2t: NetAddress,
        target_address_a2t: NetAddress, mut client_stream_read: OwnedReadHalf, configuration: Arc<AgentConfig>,
    ) -> Result<(), TcpRelayC2PError<T>>
    where
        T: RsaCryptoFetcher,
    {
        let user_token = configuration.user_token().clone().unwrap();
        if let Some(init_data) = init_data {
            let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                encryption_token: generate_uuid().into(),
                user_token: user_token.clone(),
            })
            .await
            {
                Err(e) => {
                    error!("Fail to select payload encryption type because of error: {:#?}", e);
                    return Err(TcpRelayC2PError {
                        message_framed_write,
                        client_stream_read,
                        source: anyhow!(e),
                        connection_closed: false,
                    });
                },
                Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                connection_id: Some(connection_id.clone()),
                message_framed_write,
                ref_id: Some(connection_id.clone()),
                user_token: configuration.user_token().clone().unwrap(),
                payload_encryption_type,
                message_payload: Some(MessagePayload {
                    source_address: Some(source_address_a2t.clone()),
                    target_address: Some(target_address_a2t.clone()),
                    payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                    data: init_data.into(),
                }),
            })
            .await;
            message_framed_write = match write_agent_message_result {
                Err(WriteMessageFramedError { message_framed_write, source }) => {
                    error!("Fail to write agent message to proxy because of error: {:#?}", source);
                    return Err(TcpRelayC2PError {
                        message_framed_write,
                        client_stream_read,
                        source: anyhow!(source),
                        connection_closed: true,
                    });
                },
                Ok(WriteMessageFramedResult { mut message_framed_write }) => {
                    if let Err(e) = message_framed_write.flush().await {
                        error!("Fail to flush agent message to proxy because of error: {:#?}", e);
                        return Err(TcpRelayC2PError {
                            message_framed_write,
                            client_stream_read,
                            source: anyhow!(e),
                            connection_closed: false,
                        });
                    };
                    message_framed_write
                },
            };
        }
        loop {
            let client_buffer_size = configuration.client_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let mut client_buffer = BytesMut::with_capacity(client_buffer_size);
            match client_stream_read.read_buf(&mut client_buffer).await {
                Err(e) => {
                    error!(
                        "Error happen when relay data from client to proxy,  agent address={:?}, target address={:?}, error: {:#?}",
                        source_address_a2t, target_address_a2t, e
                    );
                    return Err(TcpRelayC2PError {
                        message_framed_write,
                        client_stream_read,
                        source: anyhow!(e),
                        connection_closed: false,
                    });
                },
                Ok(0) => {
                    debug!("Read all data from client, target address: {:?}", target_address_a2t);
                    if let Err(e) = message_framed_write.flush().await {
                        return Err(TcpRelayC2PError {
                            message_framed_write,
                            client_stream_read,
                            source: anyhow!(e),
                            connection_closed: false,
                        });
                    };
                    if let Err(e) = message_framed_write.close().await {
                        return Err(TcpRelayC2PError {
                            message_framed_write,
                            client_stream_read,
                            source: anyhow!(e),
                            connection_closed: true,
                        });
                    };
                    return Ok(());
                },
                Ok(size) => {
                    debug!("Read {} bytes from client, target address: {:?}", size, target_address_a2t);
                    size
                },
            };
            let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                encryption_token: generate_uuid().into(),
                user_token: user_token.clone(),
            })
            .await
            {
                Err(e) => {
                    error!("Fail to select payload encryption type because of error: {:#?}", e);
                    return Err(TcpRelayC2PError {
                        message_framed_write,
                        client_stream_read,
                        source: anyhow!(e),
                        connection_closed: false,
                    });
                },
                Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            let payload_data = client_buffer.split().freeze();
            let payload_data_chunks = payload_data.chunks(configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE));
            for (_, chunk) in payload_data_chunks.enumerate() {
                let chunk_data = Bytes::copy_from_slice(chunk);
                let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                    connection_id: Some(connection_id.clone()),
                    message_framed_write,
                    ref_id: Some(connection_id.clone()),
                    user_token: configuration.user_token().clone().unwrap(),
                    payload_encryption_type: payload_encryption_type.clone(),
                    message_payload: Some(MessagePayload {
                        source_address: Some(source_address_a2t.clone()),
                        target_address: Some(target_address_a2t.clone()),
                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                        data: chunk_data,
                    }),
                })
                .await;
                message_framed_write = match write_agent_message_result {
                    Err(WriteMessageFramedError { message_framed_write, source }) => {
                        error!("Fail to write agent message to proxy because of error: {:#?}", source);
                        return Err(TcpRelayC2PError {
                            message_framed_write,
                            client_stream_read,
                            source: anyhow!(source),
                            connection_closed: true,
                        });
                    },
                    Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                };
            }
            if let Err(e) = message_framed_write.flush().await {
                return Err(TcpRelayC2PError {
                    message_framed_write,
                    client_stream_read,
                    source: anyhow!(e),
                    connection_closed: false,
                });
            };
        }
    }

    async fn relay_proxy_to_client<T>(
        connection_id: String, _target_address_t2a: NetAddress, mut message_framed_read: MessageFramedRead<T>, mut client_stream_write: OwnedWriteHalf,
        configuration: Arc<AgentConfig>,
    ) -> Result<(), TcpRelayP2CError<T>>
    where
        T: RsaCryptoFetcher,
    {
        loop {
            let connection_id = connection_id.clone();
            let read_proxy_message_result = MessageFramedReader::read(ReadMessageFramedRequest {
                connection_id,
                message_framed_read,
            })
            .await;
            let proxy_raw_data = match read_proxy_message_result {
                Err(ReadMessageFramedError { message_framed_read, source }) => {
                    return Err(TcpRelayP2CError {
                        message_framed_read,
                        client_stream_write,
                        source: anyhow!(source),
                        connection_closed: false,
                    });
                },
                Ok(ReadMessageFramedResult {
                    message_framed_read: message_framed_read_give_back,
                    content:
                        Some(ReadMessageFramedResultContent {
                            message_payload:
                                Some(MessagePayload {
                                    data,
                                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                                    ..
                                }),
                            ..
                        }),
                    ..
                }) => {
                    message_framed_read = message_framed_read_give_back;
                    data
                },
                Ok(ReadMessageFramedResult {
                    message_framed_read,
                    content: None,
                    ..
                }) => {
                    client_stream_write.flush().await.map_err(|e| TcpRelayP2CError {
                        message_framed_read,
                        client_stream_write,
                        source: anyhow!(e),
                        connection_closed: false,
                    })?;
                    return Ok(());
                },
                Ok(ReadMessageFramedResult { message_framed_read, .. }) => {
                    return Err(TcpRelayP2CError {
                        message_framed_read,
                        client_stream_write,
                        source: anyhow!(PpaassError::CodecError),
                        connection_closed: false,
                    })
                },
            };

            let client_buffer_size = configuration.client_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let proxy_raw_data_chunks = proxy_raw_data.chunks(client_buffer_size);
            for (_, chunk) in proxy_raw_data_chunks.enumerate() {
                if let Err(e) = client_stream_write.write(chunk).await {
                    let mut connection_closed = false;
                    if let ErrorKind::ConnectionReset = e.kind() {
                        connection_closed = true;
                    }
                    return Err(TcpRelayP2CError {
                        message_framed_read,
                        client_stream_write,
                        source: anyhow!(e),
                        connection_closed,
                    });
                }
            }
            if let Err(e) = client_stream_write.flush().await {
                return Err(TcpRelayP2CError {
                    message_framed_read,
                    client_stream_write,
                    source: anyhow!(e),
                    connection_closed: false,
                });
            }
        }
    }
}
