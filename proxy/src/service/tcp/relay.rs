use std::task::{Context, Poll};
use std::{
    fmt::{Debug, Formatter},
    marker::PhantomData,
};
use std::{net::SocketAddr, time::Duration};

use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use futures::{future::BoxFuture, SinkExt, TryFutureExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};

use tower::Service;
use tower::ServiceBuilder;
use tracing::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, MessageFramedRead,
    MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionTypeSelectService,
    PayloadEncryptionTypeSelectServiceRequest, PayloadEncryptionTypeSelectServiceResult,
    PayloadType, PpaassError, ProxyMessagePayloadTypeValue, ReadMessageService,
    ReadMessageServiceRequest, ReadMessageServiceResult, RsaCryptoFetcher, WriteMessageService,
    WriteMessageServiceError, WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::config::DEFAULT_READ_AGENT_TIMEOUT_SECONDS;
use crate::SERVER_CONFIG;

const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;
const DEFAULT_READ_TARGET_TIMEOUT_SECONDS: u64 = 20;

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

#[allow(unused)]
pub(crate) struct TcpRelayServiceResult {
    pub agent_address: SocketAddr,
    pub target_address: NetAddress,
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
        Self {
            _mark: Default::default(),
        }
    }
}

impl<T> Service<TcpRelayServiceRequest<T>> for TcpRelayService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = TcpRelayServiceResult;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TcpRelayServiceRequest<T>) -> Self::Future {
        Box::pin(async move {
            let message_framed_read = req.message_framed_read;
            let message_framed_write = req.message_framed_write;
            let target_stream = req.target_stream;
            let agent_tcp_connect_message_id = req.agent_tcp_connect_message_id.clone();
            let user_token = req.user_token.clone();
            let agent_connect_message_source_address = req.source_address.clone();
            let agent_connect_message_target_address = req.target_address.clone();
            let target_address_for_return = agent_connect_message_target_address.clone();
            let (target_stream_read, target_stream_write) = target_stream.into_split();
            tokio::spawn(Self::generate_proxy_to_target_relay(
                req.agent_address,
                message_framed_read,
                target_stream_write,
            ));
            tokio::spawn(async move {
                if let Err(mut message_framed_write) = Self::relay_target_to_proxy(
                    message_framed_write,
                    agent_tcp_connect_message_id,
                    user_token,
                    agent_connect_message_source_address,
                    agent_connect_message_target_address,
                    target_stream_read,
                )
                .await
                {
                    if let Err(e) = message_framed_write.flush().await {
                        error!("Fail to flush proxy writer when relay data from target to proxy have error:{:#?}", e);
                    };
                    if let Err(e) = message_framed_write.close().await {
                        error!("Fail to close proxy writer when relay data from target to proxy have error:{:#?}", e);
                    };
                }
            });
            Ok(TcpRelayServiceResult {
                target_address: target_address_for_return,
                agent_address: req.agent_address,
            })
        })
    }
}

impl<T> TcpRelayService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    async fn generate_proxy_to_target_relay(
        agent_address: SocketAddr, mut message_framed_read: MessageFramedRead<T>,
        mut target_stream_write: OwnedWriteHalf,
    ) {
        let mut read_agent_message_service =
            ServiceBuilder::new().service(ReadMessageService::new(
                SERVER_CONFIG
                    .read_agent_timeout_seconds()
                    .unwrap_or(DEFAULT_READ_AGENT_TIMEOUT_SECONDS),
            ));
        loop {
            let read_agent_message_result = ready_and_call_service(
                &mut read_agent_message_service,
                ReadMessageServiceRequest {
                    message_framed_read,
                    read_from_address: Some(agent_address),
                },
            )
            .await;
            let ReadMessageServiceResult {
                message_payload:
                    MessagePayload {
                        data: agent_data, ..
                    },
                message_framed_read: message_framed_read_from_read_agent_result,
                ..
            } = match read_agent_message_result {
                Ok(None) => {
                    debug!("Read all data from agent: {:#?}", agent_address);
                    if let Err(e) = target_stream_write.flush().await {
                        error!(
                            "Fail to flush from agent to target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                    };
                    if let Err(e) = target_stream_write.shutdown().await {
                        error!("Fail to shutdown target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                    };
                    return;
                },
                Ok(Some(
                    v @ ReadMessageServiceResult {
                        message_payload:
                            MessagePayload {
                                payload_type:
                                    PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                                ..
                            },
                        ..
                    },
                )) => v,
                Ok(_) => {
                    error!("Invalid payload type from agent: {:#?}", agent_address);
                    if let Err(e) = target_stream_write.flush().await {
                        error!(
                            "Fail to flush from agent to target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                    };
                    if let Err(e) = target_stream_write.shutdown().await {
                        error!("Fail to shutdown target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                    };
                    return;
                },
                Err(e) => {
                    debug!("Fail to read from agent because of error: {:#?}", e);
                    if let Err(e) = target_stream_write.flush().await {
                        error!(
                            "Fail to flush from agent to target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                    };
                    if let Err(e) = target_stream_write.shutdown().await {
                        error!("Fail to shutdown target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                    };
                    return;
                },
            };
            let agent_data_chunks = agent_data
                .chunks(SERVER_CONFIG.target_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE) as usize);
            for (_, chunk) in agent_data_chunks.enumerate() {
                if let Err(e) = target_stream_write.write(chunk).await {
                    error!("Fail to write from agent to target because of error: {:#?}", e);
                    if let Err(e) = target_stream_write.shutdown().await {
                        error!("Fail to shutdown target stream because of error: {:#?}", e);
                    };
                    return;
                };
            }
            if let Err(e) = target_stream_write.flush().await {
                error!(
                            "Fail to flush from agent to target because of error, agent address:{:?}, error: {:#?}",
                           agent_address, e
                        );
                if let Err(e) = target_stream_write.shutdown().await {
                    error!("Fail to shutdown target stream because of error: {:#?}", e);
                };
            };
            message_framed_read = message_framed_read_from_read_agent_result;
        }
    }

    async fn relay_target_to_proxy(
        mut message_framed_write: MessageFramedWrite<T>, agent_tcp_connect_message_id: String,
        user_token: String, agent_connect_message_source_address: NetAddress,
        agent_connect_message_target_address: NetAddress, mut target_stream_read: OwnedReadHalf,
    ) -> Result<(), MessageFramedWrite<T>> {
        loop {
            let mut write_proxy_message_service: WriteMessageService = Default::default();
            let mut payload_encryption_type_select_service: PayloadEncryptionTypeSelectService =
                Default::default();
            let target_buffer_size =
                SERVER_CONFIG.target_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
            let mut target_buffer = BytesMut::with_capacity(target_buffer_size);
            let source_address = agent_connect_message_source_address.clone();
            let target_address = agent_connect_message_target_address.clone();
            let timeout_seconds = SERVER_CONFIG
                .read_target_timeout_seconds()
                .unwrap_or(DEFAULT_READ_TARGET_TIMEOUT_SECONDS);
            match timeout(
                Duration::from_secs(timeout_seconds),
                target_stream_read.read_buf(&mut target_buffer),
            )
            .await
            {
                Err(_e) => {
                    return Err(message_framed_write);
                },
                Ok(Err(_e)) => {
                    return Err(message_framed_write);
                },
                Ok(Ok(0)) => {
                    message_framed_write.flush().await.map_err(|_e| message_framed_write)?;
                    return Ok(());
                },
                Ok(Ok(size)) => size,
            };
            let payload_data = target_buffer.split().freeze();
            let payload_data_chunks = payload_data
                .chunks(SERVER_CONFIG.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE));
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
                    Err(e) => return Err(message_framed_write),
                    Ok(PayloadEncryptionTypeSelectServiceResult {
                        payload_encryption_type,
                        ..
                    }) => payload_encryption_type,
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
                    Err(WriteMessageServiceError {
                        message_framed_write,
                        ..
                    }) => return Err(message_framed_write),
                    Ok(WriteMessageServiceResult {
                        message_framed_write,
                        ..
                    }) => message_framed_write,
                };
            }
        }
    }
}
