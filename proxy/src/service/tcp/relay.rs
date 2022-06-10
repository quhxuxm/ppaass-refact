use std::task::{Context, Poll};
use std::{
    fmt::{Debug, Formatter},
    marker::PhantomData,
};
use std::{net::SocketAddr, time::Duration};

use bytes::{BufMut, BytesMut};
use futures::future::BoxFuture;
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
    WriteMessageServiceRequest,
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
            tokio::spawn(Self::generate_target_to_proxy_relay(
                message_framed_write,
                agent_tcp_connect_message_id,
                user_token,
                agent_connect_message_source_address,
                agent_connect_message_target_address,
                target_stream_read,
            ));
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
                message_payload: MessagePayload { data, .. },
                message_framed_read: message_framed_read_from_read_agent_result,
                ..
            } = match read_agent_message_result {
                Ok(None) => {
                    debug!("Read all data from agent: {:#?}", agent_address);
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
                    return;
                },
                Err(e) => {
                    debug!("Fail to read from agent because of error: {:#?}", e);
                    return;
                },
            };
            if let Err(e) = target_stream_write.write(data.as_ref()).await {
                error!(
                    "Fail to write from agent to target because of error: {:#?}",
                    e
                );
                let _ = target_stream_write.shutdown().await;
                return;
            };
            if let Err(e) = target_stream_write.flush().await {
                error!(
                    "Fail to flush from agent to target because of error: {:#?}",
                    e
                );
                let _ = target_stream_write.shutdown().await;
                return;
            };
            message_framed_read = message_framed_read_from_read_agent_result;
        }
    }

    async fn generate_target_to_proxy_relay(
        mut message_framed_write: MessageFramedWrite<T>, agent_tcp_connect_message_id: String,
        user_token: String, agent_connect_message_source_address: NetAddress,
        agent_connect_message_target_address: NetAddress, mut target_stream_read: OwnedReadHalf,
    ) {
        let mut write_proxy_message_service =
            ServiceBuilder::new().service(WriteMessageService::default());
        let mut payload_encryption_type_select_service =
            ServiceBuilder::new().service(PayloadEncryptionTypeSelectService);
        loop {
            let source_address = agent_connect_message_source_address.clone();
            let target_address = agent_connect_message_target_address.clone();
            let read_target_data_future = async move {
                let mut buf = BytesMut::with_capacity(
                    SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                );
                let read_size = match target_stream_read.read_buf(&mut buf).await {
                    Err(e) => {
                        error!("Fail to read data from target because of error, agent source address: {:?}, target address:{:?}, error: {:#?}",source_address, target_address, e);
                        return Err(PpaassError::IoError { source: e });
                    },
                    Ok(size) => {
                        debug!("Read {} bytes from target, agent source address: {:?}, target address:{:?}.", size, source_address, target_address);
                        match size {
                            0 if buf.remaining_mut() > 0 => {
                                debug!("Noting to read from target, agent source address: {:?}, target address:{:?}.", source_address, target_address);
                                return Ok((buf.freeze(), target_stream_read, 0));
                            },
                            s => s,
                        }
                    },
                };
                Ok((buf.freeze(), target_stream_read, read_size))
            };
            let source_address = agent_connect_message_source_address.clone();
            let target_address = agent_connect_message_target_address.clone();
            let timeout_seconds = SERVER_CONFIG
                .read_target_timeout_seconds()
                .unwrap_or(DEFAULT_READ_TARGET_TIMEOUT_SECONDS);
            let (buf, inner_target_stream_read, _read_size) = match timeout(
                Duration::from_secs(timeout_seconds),
                read_target_data_future,
            )
            .await
            {
                Err(_e) => {
                    error!("The read target data timeout, source address:{:?}, target address:{:?}, timeout: {}.", source_address, target_address, timeout_seconds);
                    return;
                },
                Ok(Err(e)) => {
                    debug!("Fail to read target data because of error, source address:{:?}, target address:{:?}, error: {:#?}", source_address, target_address, e);
                    return;
                },
                Ok(Ok((_, _, 0))) => {
                    debug!("Nothing to read from target, return from read target future, source address:{:?}, target address:{:?}.", source_address, target_address);
                    return;
                },
                Ok(Ok(v)) => v,
            };
            let proxy_message_payload = MessagePayload::new(
                source_address.clone(),
                target_address.clone(),
                PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData),
                buf,
            );
            let PayloadEncryptionTypeSelectServiceResult {
                payload_encryption_type,
                ..
            } = match ready_and_call_service(
                &mut payload_encryption_type_select_service,
                PayloadEncryptionTypeSelectServiceRequest {
                    encryption_token: generate_uuid().into(),
                    user_token: user_token.clone(),
                },
            )
            .await
            {
                Err(e) => {
                    error!(
                        "Fail to select payload encryption type because of error, source address:{:?}, target address:{:?}, error: {:#?}",source_address, target_address,
                        e
                    );
                    return;
                },
                Ok(v) => v,
            };
            let write_proxy_message_result = ready_and_call_service(
                &mut write_proxy_message_service,
                WriteMessageServiceRequest {
                    message_framed_write,
                    ref_id: Some(agent_tcp_connect_message_id.clone()),
                    user_token: user_token.clone(),
                    payload_encryption_type,
                    message_payload: Some(proxy_message_payload),
                },
            )
            .await;
            match write_proxy_message_result {
                Err(e) => {
                    error!("Fail to read from target because of error(ready), source address:{:?}, target address:{:?}, error: {:#?}", source_address, target_address, e);
                    return;
                },
                Ok(proxy_message_write_result) => {
                    message_framed_write = proxy_message_write_result.message_framed_write;
                    target_stream_read = inner_target_stream_read;
                },
            }
        }
    }
}
