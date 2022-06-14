use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::{
    fmt::{Debug, Formatter},
    marker::PhantomData,
};

use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};

use futures::{future::BoxFuture, SinkExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use tower::Service;

use tracing::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectService, PayloadEncryptionTypeSelectServiceRequest, PayloadEncryptionTypeSelectServiceResult, PayloadType, PpaassError,
    ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceError, ReadMessageServiceRequest, ReadMessageServiceResult,
    ReadMessageServiceResultContent, RsaCryptoFetcher, WriteMessageService, WriteMessageServiceError, WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::SERVER_CONFIG;

const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;

#[allow(unused)]
pub(crate) struct TcpRelayServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub agent_address: SocketAddr,
    pub target_stream: TcpStream,
    pub agent_tcp_connect_message_id: String,
    pub user_token: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}

impl<T> Debug for TcpRelayServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "TcpRelayServiceRequest: gent_address={}, target_address={}",
            self.agent_address,
            self.target_address.to_string()
        )
    }
}

#[derive(Clone)]
pub(crate) struct TcpRelayService<T>
where
    T: RsaCryptoFetcher,
{
    _mark: PhantomData<T>,
}

impl<T> Default for TcpRelayService<T>
where
    T: RsaCryptoFetcher,
{
    fn default() -> Self {
        Self { _mark: Default::default() }
    }
}

impl<T> Service<TcpRelayServiceRequest<T>> for TcpRelayService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = ();
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpRelayServiceRequest<T>) -> Self::Future {
        Box::pin(async move {
            let TcpRelayServiceRequest {
                message_framed_read,
                message_framed_write,
                agent_address,
                target_stream,
                agent_tcp_connect_message_id,
                user_token,
                source_address,
                target_address,
            } = req;
            let (target_stream_read, target_stream_write) = target_stream.into_split();
            tokio::spawn(async move {
                if let Err((_message_framed_read, mut target_stream_write, original_error)) =
                    Self::relay_proxy_to_target(agent_address, message_framed_read, target_stream_write).await
                {
                    error!("Error happen when relay data from proxy to target, error: {:#?}", original_error);
                    if let Err(e) = target_stream_write.flush().await {
                        error!("Fail to flush target stream writer when relay data from proxy to target have error:{:#?}", e);
                    };
                    if let Err(e) = target_stream_write.shutdown().await {
                        error!("Fail to shutdown target stream writer when relay data from proxy to target have error:{:#?}", e);
                    };
                }
            });
            tokio::spawn(async move {
                if let Err((mut message_framed_write, _target_stream_read, original_error)) = Self::relay_target_to_proxy(
                    message_framed_write,
                    agent_tcp_connect_message_id,
                    user_token,
                    source_address,
                    target_address,
                    target_stream_read,
                )
                .await
                {
                    error!("Error happen when relay data from target to proxy, error: {:#?}", original_error);
                    if let Err(e) = message_framed_write.flush().await {
                        error!("Fail to flush proxy writer when relay data from target to proxy have error:{:#?}", e);
                    };
                    if let Err(e) = message_framed_write.close().await {
                        error!("Fail to close proxy writer when relay data from target to proxy have error:{:#?}", e);
                    };
                }
            });
            Ok(())
        })
    }
}

impl<T> TcpRelayService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    async fn relay_proxy_to_target(
        agent_address: SocketAddr, mut message_framed_read: MessageFramedRead<T>, mut target_stream_write: OwnedWriteHalf,
    ) -> Result<(), (MessageFramedRead<T>, OwnedWriteHalf, anyhow::Error)> {
        loop {
            let mut read_agent_message_service: ReadMessageService = Default::default();
            let read_agent_message_result = ready_and_call_service(
                &mut read_agent_message_service,
                ReadMessageServiceRequest {
                    message_framed_read,
                    read_from_address: Some(agent_address),
                },
            )
            .await;
            let (message_framed_read_move_back, agent_data) = match read_agent_message_result {
                Err(ReadMessageServiceError { message_framed_read, source }) => {
                    error!(
                        "Error happen when relay data from proxy to target,  agent address={:?}, target address={:?}, error: {:#?}",
                        agent_address,
                        target_stream_write.peer_addr(),
                        source
                    );
                    return Err((message_framed_read, target_stream_write, anyhow!(source)));
                },
                Ok(ReadMessageServiceResult {
                    message_framed_read,
                    content: None,
                }) => {
                    debug!(
                        "Read all data from agent, agent address={:?}, target address = {:?}.",
                        agent_address,
                        target_stream_write.peer_addr()
                    );
                    target_stream_write
                        .flush()
                        .await
                        .map_err(|e| (message_framed_read, target_stream_write, anyhow!(e)))?;
                    return Ok(());
                },
                Ok(ReadMessageServiceResult {
                    message_framed_read,
                    content:
                        Some(ReadMessageServiceResultContent {
                            message_payload:
                                Some(MessagePayload {
                                    payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                                    data,
                                    ..
                                }),
                            ..
                        }),
                }) => (message_framed_read, data),
                Ok(ReadMessageServiceResult { message_framed_read, .. }) => {
                    error!(
                        "Receive invalid data from agent when relay data from proxy to target,  agent address={:?}, target address={:?}",
                        agent_address,
                        target_stream_write.peer_addr()
                    );
                    return Err((message_framed_read, target_stream_write, anyhow!(PpaassError::CodecError)));
                },
            };
            message_framed_read = message_framed_read_move_back;
            let agent_data_chunks = agent_data.chunks(SERVER_CONFIG.target_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE) as usize);
            for (_, chunk) in agent_data_chunks.enumerate() {
                if let Err(e) = target_stream_write.write(chunk).await {
                    return Err((message_framed_read, target_stream_write, anyhow!(e)));
                };
            }
            if let Err(e) = target_stream_write.flush().await {
                return Err((message_framed_read, target_stream_write, anyhow!(e)));
            }
        }
    }

    async fn relay_target_to_proxy(
        mut message_framed_write: MessageFramedWrite<T>, agent_tcp_connect_message_id: String, user_token: String,
        agent_connect_message_source_address: NetAddress, agent_connect_message_target_address: NetAddress, mut target_stream_read: OwnedReadHalf,
    ) -> Result<(), (MessageFramedWrite<T>, OwnedReadHalf, anyhow::Error)> {
        loop {
            let mut write_proxy_message_service: WriteMessageService = Default::default();
            let mut payload_encryption_type_select_service: PayloadEncryptionTypeSelectService = Default::default();
            let target_buffer_size = SERVER_CONFIG.target_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let mut target_buffer = BytesMut::with_capacity(target_buffer_size);
            let source_address = agent_connect_message_source_address.clone();
            let target_address = agent_connect_message_target_address.clone();
            match target_stream_read.read_buf(&mut target_buffer).await {
                Err(e) => {
                    error!(
                        "Error happen when relay data from target to proxy, target address={:?}, source address={:?}, error: {:#?}",
                        target_address, source_address, e
                    );
                    return Err((message_framed_write, target_stream_read, anyhow!(e)));
                },
                Ok(0) => {
                    debug!(
                        "Read all data from target, target address={:?}, source address={:?}.",
                        target_address, source_address
                    );
                    if let Err(e) = message_framed_write.flush().await {
                        return Err((message_framed_write, target_stream_read, anyhow!(e)));
                    };
                    if let Err(e) = message_framed_write.close().await {
                        return Err((message_framed_write, target_stream_read, anyhow!(e)));
                    };
                    return Ok(());
                },
                Ok(size) => {
                    debug!(
                        "Read {} bytes from target to proxy, target address={:?}, source address={:?}.",
                        size, target_address, source_address
                    );
                    size
                },
            };
            let payload_data = target_buffer.split().freeze();
            let payload_data_chunks = payload_data.chunks(SERVER_CONFIG.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE));
            for (_, chunk) in payload_data_chunks.enumerate() {
                let chunk_data = Bytes::copy_from_slice(chunk);
                let proxy_message_payload = MessagePayload::new(
                    source_address.clone(),
                    target_address.clone(),
                    PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                    chunk_data,
                );
                let payload_encryption_type = match ready_and_call_service(
                    &mut payload_encryption_type_select_service,
                    PayloadEncryptionTypeSelectServiceRequest {
                        encryption_token: generate_uuid().into(),
                        user_token: user_token.clone(),
                    },
                )
                .await
                {
                    Err(e) => return Err((message_framed_write, target_stream_read, anyhow!(e))),
                    Ok(PayloadEncryptionTypeSelectServiceResult { payload_encryption_type, .. }) => payload_encryption_type,
                };
                message_framed_write = match ready_and_call_service(
                    &mut write_proxy_message_service,
                    WriteMessageServiceRequest {
                        message_framed_write,
                        ref_id: Some(agent_tcp_connect_message_id.clone()),
                        user_token: user_token.clone(),
                        payload_encryption_type,
                        message_payload: Some(proxy_message_payload),
                    },
                )
                .await
                {
                    Err(WriteMessageServiceError { message_framed_write, source }) => return Err((message_framed_write, target_stream_read, anyhow!(source))),
                    Ok(WriteMessageServiceResult { message_framed_write, .. }) => message_framed_write,
                };
            }
        }
    }
}
