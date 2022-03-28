use std::task::Poll;
use std::{fmt::Debug, net::SocketAddr};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;
use tower::ServiceBuilder;
use tracing::log::{debug, error};

use common::{
    generate_uuid, ready_and_call_service, AgentMessagePayloadTypeValue, CommonError,
    MessageFramedRead, MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType,
    PayloadType, ProxyMessagePayloadTypeValue, ReadMessageService, ReadMessageServiceRequest,
    ReadMessageServiceResult, WriteMessageService, WriteMessageServiceRequest,
};

use crate::codec::socks5::Socks5ConnectCodec;
use crate::command::socks5::{
    Socks5ConnectCommand, Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus,
    Socks5ConnectCommandType,
};
use crate::service::common::{
    generate_prepare_message_framed_service, ConnectToProxyService, ConnectToProxyServiceRequest,
    DEFAULT_BUFFER_SIZE, DEFAULT_RETRY_TIMES,
};
use crate::SERVER_CONFIG;

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

#[derive(Clone, Debug, Default)]
pub(crate) struct Socks5ConnectCommandService;

impl Socks5ConnectCommandService {
    async fn send_socks5_failure(
        socks5_client_framed: &mut Framed<&mut TcpStream, Socks5ConnectCodec>,
    ) -> Result<Option<Socks5ConnectCommand>, CommonError> {
        let connect_result =
            Socks5ConnectCommandResult::new(Socks5ConnectCommandResultStatus::Failure, None);
        if let Err(e) = socks5_client_framed.send(connect_result).await {
            error!(
                "Fail to write socks5 connect fail result to client because of error: {:#?}",
                e
            );
            return Err(e);
        };
        if let Err(e) = socks5_client_framed.flush().await {
            error!(
                "Fail to flush socks5 connect fail result to client because of error: {:#?}",
                e
            );
            return Err(CommonError::UnknownError);
        }
        Ok(None)
    }

    async fn call_service<'a, S, T, U>(
        service: &'a mut S,
        request: T,
        socks5_client_framed: &mut Framed<&'a mut TcpStream, Socks5ConnectCodec>,
    ) -> Result<U, CommonError>
    where
        S: Service<T, Response = U, Error = CommonError>,
    {
        return match ready_and_call_service(service, request).await {
            Ok(v) => Ok(v),
            Err(e) => {
                Self::send_socks5_failure(socks5_client_framed).await?;
                Err(e)
            }
        };
    }
}

impl Service<Socks5ConnectCommandServiceRequest> for Socks5ConnectCommandService {
    type Response = Socks5ConnectCommandServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5ConnectCommandServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut write_agent_message_service =
                ServiceBuilder::new().service(WriteMessageService::default());
            let mut read_proxy_message_service =
                ServiceBuilder::new().service(ReadMessageService::default());
            let mut connect_to_proxy_service =
                ServiceBuilder::new().service(ConnectToProxyService::new(
                    SERVER_CONFIG
                        .proxy_connection_retry()
                        .unwrap_or(DEFAULT_RETRY_TIMES),
                ));
            let mut prepare_message_framed_service = generate_prepare_message_framed_service();
            let mut socks5_client_framed = Framed::with_capacity(
                &mut request.client_stream,
                Socks5ConnectCodec,
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );
            let connect_command = match socks5_client_framed.next().await {
                Some(Ok(v)) => v,
                _ => {
                    Self::send_socks5_failure(&mut socks5_client_framed).await?;
                    return Err(CommonError::CodecError);
                }
            };
            debug!(
                "Client {} send socks 5 connect command: {:#?}",
                request.client_address, connect_command
            );
            let connect_to_proxy_service_result = match connect_command.request_type {
                Socks5ConnectCommandType::Connect => {
                    Self::call_service(
                        &mut connect_to_proxy_service,
                        ConnectToProxyServiceRequest {
                            proxy_address: None,
                            client_address: request.client_address,
                        },
                        &mut socks5_client_framed,
                    )
                    .await?
                }
                Socks5ConnectCommandType::Bind => {
                    todo!()
                }
                Socks5ConnectCommandType::UdpAssociate => {
                    todo!()
                }
            };
            let framed_result = Self::call_service(
                &mut prepare_message_framed_service,
                connect_to_proxy_service_result.proxy_stream,
                &mut socks5_client_framed,
            )
            .await?;
            let write_message_result = Self::call_service(
                &mut write_agent_message_service,
                WriteMessageServiceRequest {
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
                },
                &mut socks5_client_framed,
            )
            .await?;
            let read_proxy_message_result = Self::call_service(
                &mut read_proxy_message_service,
                ReadMessageServiceRequest {
                    message_framed_read: framed_result.message_framed_read,
                },
                &mut socks5_client_framed,
            )
            .await?;
            let read_proxy_message_result =
                read_proxy_message_result.ok_or(CommonError::CodecError)?;
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
            Err(CommonError::CodecError)
        })
    }
}
