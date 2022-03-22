use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::Poll;

use anyhow::Context;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::log::{debug, error};

use common::PayloadType::ProxyPayload;
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, CommonError, MessageFramedRead,
    MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType, PayloadType,
    PrepareMessageFramedResult, PrepareMessageFramedService, ProxyMessagePayloadTypeValue,
    ReadMessageService, ReadMessageServiceRequest, ReadMessageServiceResult, WriteMessageService,
    WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::codec::socks5::Socks5ConnectCodec;
use crate::command::socks5::Socks5ConnectCommandType::Connect;
use crate::command::socks5::{
    Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus, Socks5ConnectCommandType,
};
use crate::config::{AGENT_PRIVATE_KEY, PROXY_PUBLIC_KEY};
use crate::service::common::{ConnectToProxyService, ConnectToProxyServiceRequest};
use crate::SERVER_CONFIG;

const DEFAULT_RETRY_TIMES: u16 = 3;
const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
const DEFAULT_MAX_FRAME_SIZE: usize = DEFAULT_BUFFER_SIZE * 2;
#[derive(Debug)]
pub(crate) struct Socks5ConnectCommandServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

pub(crate) struct Socks5ConnectCommandServiceResult {
    pub client_stream: TcpStream,
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub client_address: SocketAddr,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub proxy_address_string: String,
    pub connect_response_message_id: String,
}

#[derive(Clone)]
pub(crate) struct Socks5ConnectCommandService {
    prepare_message_framed_service:
        BoxCloneService<TcpStream, PrepareMessageFramedResult, CommonError>,
    write_agent_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
    read_proxy_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
}

impl Default for Socks5ConnectCommandService {
    fn default() -> Self {
        Socks5ConnectCommandService {
            prepare_message_framed_service: BoxCloneService::new(PrepareMessageFramedService::new(
                &(*PROXY_PUBLIC_KEY),
                &(*AGENT_PRIVATE_KEY),
                SERVER_CONFIG
                    .max_frame_size()
                    .unwrap_or(DEFAULT_MAX_FRAME_SIZE),
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
                SERVER_CONFIG.compress().unwrap_or(true),
            )),
            write_agent_message_service: BoxCloneService::new(WriteMessageService),
            read_proxy_message_service: BoxCloneService::new(ReadMessageService),
        }
    }
}

impl Socks5ConnectCommandService {
    async fn send_socks5_failure(
        mut socks5_client_framed: Framed<&mut TcpStream, Socks5ConnectCodec>,
    ) {
        let connect_result =
            Socks5ConnectCommandResult::new(Socks5ConnectCommandResultStatus::Failure, None);
        if let Err(e) = socks5_client_framed.send(connect_result).await {
            error!(
                "Fail to write socks5 connect fail result to client because of error: {:#?}",
                e
            );
        };
        if let Err(e) = socks5_client_framed.flush().await {
            error!(
                "Fail to flush socks5 connect fail result to client because of error: {:#?}",
                e
            );
        }
    }
}

impl Service<Socks5ConnectCommandServiceRequest> for Socks5ConnectCommandService {
    type Response = Socks5ConnectCommandServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut std::task::Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5ConnectCommandServiceRequest) -> Self::Future {
        let mut prepare_message_framed_service = self.prepare_message_framed_service.clone();
        let mut write_agent_message_service = self.write_agent_message_service.clone();
        let mut read_proxy_message_service = self.read_proxy_message_service.clone();
        Box::pin(async move {
            let mut socks5_client_framed = Framed::with_capacity(
                &mut request.client_stream,
                Socks5ConnectCodec,
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );
            let connect_command = match socks5_client_framed.next().await {
                Some(Ok(v)) => v,
                _ => {
                    Self::send_socks5_failure(socks5_client_framed).await;
                    return Err(CommonError::CodecError);
                }
            };
            debug!(
                "Client {} send socks 5 connect command: {:#?}",
                request.client_address, connect_command
            );
            let connect_to_proxy_service_result = match connect_command.request_type {
                Socks5ConnectCommandType::Connect => {
                    let mut connect_to_proxy_service = ConnectToProxyService::new(
                        SERVER_CONFIG
                            .proxy_connection_retry()
                            .unwrap_or(DEFAULT_RETRY_TIMES),
                    );
                    let connect_to_proxy_service_ready =
                        match connect_to_proxy_service.ready().await {
                            Ok(v) => v,
                            Err(e) => {
                                Self::send_socks5_failure(socks5_client_framed).await;
                                return Err(e);
                            }
                        };
                    match connect_to_proxy_service_ready
                        .call(ConnectToProxyServiceRequest {
                            proxy_address: None,
                            client_address: request.client_address,
                        })
                        .await
                    {
                        Ok(v) => v,
                        Err(e) => {
                            Self::send_socks5_failure(socks5_client_framed).await;
                            return Err(e);
                        }
                    }
                }
                Socks5ConnectCommandType::Bind => {
                    todo!()
                }
                Socks5ConnectCommandType::UdpAssociate => {
                    todo!()
                }
            };
            let framed_result = prepare_message_framed_service
                .ready()
                .await?
                .call(connect_to_proxy_service_result.proxy_stream)
                .await?;
            let write_message_result = write_agent_message_service
                .ready()
                .await?
                .call(WriteMessageServiceRequest {
                    message_framed_write: framed_result.message_framed_write,
                    payload_encryption_type: PayloadEncryptionType::Blowfish(
                        generate_uuid().into(),
                    ),
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                    ref_id: None,
                    message_payload: Some(MessagePayload::new(
                        request.client_address.into(),
                        connect_command.dest_address.clone().into(),
                        PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                        Bytes::new(),
                    )),
                })
                .await?;
            let read_proxy_message_result = read_proxy_message_service
                .ready()
                .await?
                .call(ReadMessageServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                })
                .await?
                .ok_or(CommonError::CodecError)?;
            if let ReadMessageServiceResult {
                message_payload:
                    MessagePayload {
                        payload_type:
                            PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
                        source_address,
                        target_address,
                        ..
                    },
                message_framed_read,
                message_id,
                ..
            } = read_proxy_message_result
            {
                //Response for socks5 connect command
                let connect_result = Socks5ConnectCommandResult::new(
                    Socks5ConnectCommandResultStatus::Succeeded,
                    Some(connect_command.dest_address),
                );
                socks5_client_framed.send(connect_result).await?;
                socks5_client_framed.flush().await?;
                return Ok(Socks5ConnectCommandServiceResult {
                    client_stream: request.client_stream,
                    client_address: request.client_address,
                    message_framed_read,
                    message_framed_write: write_message_result.message_framed_write,
                    source_address,
                    target_address,
                    proxy_address_string: connect_to_proxy_service_result.connected_proxy_address,
                    connect_response_message_id: message_id,
                });
            }
            error!("Fail to read proxy connect result because of invalid payload type.");
            return Err(CommonError::CodecError);
        })
    }
}
