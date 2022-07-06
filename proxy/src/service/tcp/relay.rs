use std::fmt::Debug;

use std::net::SocketAddr;

use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};

use futures::SinkExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use tracing::{debug, error};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use crate::config::ProxyConfig;

const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct TcpRelayFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub agent_address: SocketAddr,
    pub target_stream: TcpStream,
    pub agent_tcp_connect_message_id: &'a str,
    pub user_token: &'a str,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}

pub(crate) struct TcpRelayFlow;

impl TcpRelayFlow {
    pub async fn exec<'a, T>(request: TcpRelayFlowRequest<'a, T>, configuration: &ProxyConfig) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let TcpRelayFlowRequest {
            connection_id,
            message_framed_read,
            message_framed_write,
            agent_address,
            target_stream,
            agent_tcp_connect_message_id,
            user_token,
            source_address,
            target_address,
        } = request;
        let (target_stream_read, target_stream_write) = target_stream.into_split();
        let connection_id_p2t = connection_id.to_owned();
        tokio::spawn(async move {
            if let Err(e) = Self::relay_proxy_to_target(&connection_id_p2t, agent_address, message_framed_read, target_stream_write).await {
                error!(
                    "Connection [{}] error happen when relay data from proxy to target, error: {:#?}",
                    connection_id_p2t, e
                );
            }
        });
        let target_buffer_size = configuration.target_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let agent_tcp_connect_message_id_t2p = agent_tcp_connect_message_id.to_owned();
        let user_token_t2p = user_token.to_owned();
        let connection_id_t2p = connection_id.to_owned();
        tokio::spawn(async move {
            if let Err(e) = Self::relay_target_to_proxy(
                &connection_id_t2p,
                message_framed_write,
                &agent_tcp_connect_message_id_t2p,
                &user_token_t2p,
                source_address,
                target_address,
                target_stream_read,
                target_buffer_size,
                message_framed_buffer_size,
            )
            .await
            {
                error!(
                    "Connection [{}] error happen when relay data from target to proxy, error: {:#?}",
                    connection_id_t2p, e
                );
            }
        });
        Ok(())
    }

    async fn relay_proxy_to_target<T>(
        connection_id: &str, agent_address: SocketAddr, mut message_framed_read: MessageFramedRead<T, TcpStream>, mut target_stream_write: OwnedWriteHalf,
    ) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        loop {
            let mut agent_data;
            (message_framed_read, agent_data) = match MessageFramedReader::read(ReadMessageFramedRequest {
                connection_id,
                message_framed_read,
                timeout: None,
            })
            .await
            {
                Err(ReadMessageFramedError { source, .. }) => {
                    let target_peer_addr = target_stream_write.peer_addr();
                    error!(
                        "Connection [{}] error happen when relay data from proxy to target,  agent address={:?}, target address={:?}, error: {:#?}",
                        connection_id, agent_address, target_peer_addr, source
                    );
                    return Err(source.into());
                },
                Ok(ReadMessageFramedResult { content: None, .. }) => {
                    let target_peer_addr = target_stream_write.peer_addr();
                    debug!(
                        "Connection [{}] read all data from agent, agent address={:?}, target address = {:?}.",
                        connection_id, agent_address, target_peer_addr
                    );
                    target_stream_write.flush().await?;
                    return Ok(());
                },
                Ok(ReadMessageFramedResult {
                    message_framed_read,
                    content:
                        Some(ReadMessageFramedResultContent {
                            message_payload:
                                Some(MessagePayload {
                                    payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                                    data: Some(data),
                                    ..
                                }),
                            ..
                        }),
                }) => (message_framed_read, data),
                Ok(ReadMessageFramedResult { .. }) => {
                    let target_peer_addr = target_stream_write.peer_addr();
                    error!(
                        "Connection [{}] receive invalid data from agent when relay data from proxy to target,  agent address={:?}, target address={:?}",
                        connection_id, agent_address, target_peer_addr
                    );
                    return Err(anyhow!(
                        "Connection [{}] receive invalid data from agent when relay data from proxy to target,  agent address={:?}, target address={:?}",
                        connection_id,
                        agent_address,
                        target_peer_addr
                    ));
                },
            };
            target_stream_write.write_all_buf(&mut agent_data).await?;
            target_stream_write.flush().await?;
        }
    }

    async fn relay_target_to_proxy<T>(
        connection_id: &str, mut message_framed_write: MessageFramedWrite<T, TcpStream>, agent_tcp_connect_message_id: &str, user_token: &str,
        agent_connect_message_source_address: NetAddress, agent_connect_message_target_address: NetAddress, mut target_stream_read: OwnedReadHalf,
        target_buffer_size: usize, message_framed_buffer_size: usize,
    ) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        loop {
            let mut target_buffer = BytesMut::with_capacity(target_buffer_size);
            let source_address = agent_connect_message_source_address.clone();
            let target_address = agent_connect_message_target_address.clone();
            match target_stream_read.read_buf(&mut target_buffer).await {
                Err(e) => {
                    error!(
                        "Connection [{}] error happen when relay data from target to proxy, target address={:?}, source address={:?}, error: {:#?}",
                        connection_id, target_address, source_address, e
                    );
                    return Err(e.into());
                },
                Ok(0) => {
                    debug!(
                        "Connection [{}] read all data from target, target address={:?}, source address={:?}.",
                        connection_id, target_address, source_address
                    );
                    message_framed_write.flush().await?;
                    message_framed_write.close().await?;
                    return Ok(());
                },
                Ok(size) => {
                    debug!(
                        "Connection [{}] read {} bytes from target to proxy, target address={:?}, source address={:?}.",
                        connection_id, size, target_address, source_address
                    );
                    size
                },
            };
            let payload_data = target_buffer.split().freeze();
            let payload_data_chunks = payload_data.chunks(message_framed_buffer_size);
            let mut payloads = vec![];
            for (_, chunk) in payload_data_chunks.enumerate() {
                let chunk_data = Bytes::copy_from_slice(chunk);
                let proxy_message_payload = MessagePayload {
                    source_address: Some(source_address.clone()),
                    target_address: Some(target_address.clone()),
                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                    data: Some(chunk_data),
                };
                payloads.push(proxy_message_payload)
            }
            let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                encryption_token: generate_uuid().into(),
                user_token,
            })
            .await
            {
                Err(e) => {
                    error!(
                            "Connection [{}] fail to select payload encryption type when transfer data from target to proxy, target address={:?}, source address={:?}, error: {:#?}.",
                            connection_id, target_address, source_address, e
                        );
                    return Err(e.into());
                },
                Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
            };
            message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                message_framed_write,
                ref_id: Some(agent_tcp_connect_message_id),
                user_token,
                payload_encryption_type,
                message_payloads: Some(payloads),
                connection_id: Some(connection_id),
            })
            .await
            {
                Err(WriteMessageFramedError { source, .. }) => {
                    error!(
                        "Connection [{}] fail to write data from target to proxy, target address={:?}, source address={:?}, error: {:#?}.",
                        connection_id, target_address, source_address, source
                    );
                    return Err(source.into());
                },
                Ok(WriteMessageFramedResult { message_framed_write, .. }) => message_framed_write,
            };
        }
    }
}
