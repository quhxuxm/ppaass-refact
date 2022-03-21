use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::log::{debug, error};

use common::{
    generate_uuid, CommonError, MessageFramedRead, MessageFramedWrite, MessagePayload,
    PayloadEncryptionType, PrepareMessageFramedResult, PrepareMessageFramedService,
    WriteMessageService, WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::codec::socks5::Socks5ConnectCodec;
use crate::command::socks5::{
    Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus, Socks5ConnectCommandType,
};
use crate::config::{AGENT_PRIVATE_KEY, PROXY_PUBLIC_KEY};
use crate::service::common::{ConnectToProxyRequest, ConnectToProxyService};
use crate::SERVER_CONFIG;

const DEFAULT_RETRY_TIMES: u16 = 3;
const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;
#[derive(Debug)]
pub(crate) struct Socks5ConnectFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

pub(crate) struct Socks5ConnectFlowResult {
    pub client_stream: TcpStream,
    pub message_framed_read: MessageFramedRead,
    pub message_framed_write: MessageFramedWrite,
    pub client_address: SocketAddr,
}

#[derive(Clone)]
pub(crate) struct Socks5ConnectCommandService {
    prepare_message_framed_service:
        BoxCloneService<TcpStream, PrepareMessageFramedResult, CommonError>,
    write_proxy_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
}

impl Socks5ConnectCommandService {
    pub(crate) fn new() -> Self {
        Socks5ConnectCommandService {
            prepare_message_framed_service: BoxCloneService::new(PrepareMessageFramedService::new(
                &(*PROXY_PUBLIC_KEY),
                &(*AGENT_PRIVATE_KEY),
                SERVER_CONFIG.buffer_size().unwrap_or(1026 * 64),
                SERVER_CONFIG.compress().unwrap_or(true),
            )),
            write_proxy_message_service: BoxCloneService::new(WriteMessageService),
        }
    }
}

impl Service<Socks5ConnectFlowRequest> for Socks5ConnectCommandService {
    type Response = Socks5ConnectFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5ConnectFlowRequest) -> Self::Future {
        let mut prepare_message_framed_service = self.prepare_message_framed_service.clone();
        let mut write_proxy_message_service = self.write_proxy_message_service.clone();
        Box::pin(async move {
            let mut framed = Framed::with_capacity(
                &mut request.client_stream,
                Socks5ConnectCodec,
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );
            let connect_command = match framed.next().await {
                None => {
                    error!(
                        "No connect command from client: {:#?}",
                        request.client_address
                    );
                    let connect_result = Socks5ConnectCommandResult::new(
                        Socks5ConnectCommandResultStatus::Failure,
                        None,
                    );
                    framed.send(connect_result).await?;
                    return Err(CommonError::IoError {
                        source: std::io::Error::new(
                            ErrorKind::InvalidData,
                            "No connect command from client.",
                        ),
                    });
                }
                Some(v) => v?,
            };
            debug!(
                "Client {} send socks 5 connect command: {:#?}",
                request.client_address, connect_command
            );
            let connect_command_type = connect_command.request_type;

            let connect_to_proxy_response = match connect_command_type {
                Socks5ConnectCommandType::Connect => {
                    let mut connect_to_proxy_service = ConnectToProxyService::new(
                        SERVER_CONFIG
                            .proxy_connection_retry()
                            .unwrap_or(DEFAULT_RETRY_TIMES),
                    );
                    let connect_to_proxy_service_ready = match connect_to_proxy_service
                        .ready()
                        .await
                    {
                        Err(e) => {
                            error!(
                                    "Fail to invoke connect proxy service because of error(ready): {:#?}",
                                    e
                                );
                            let connect_result = Socks5ConnectCommandResult::new(
                                Socks5ConnectCommandResultStatus::Failure,
                                None,
                            );
                            framed.send(connect_result).await?;
                            return Err(e);
                        }
                        Ok(r) => r,
                    };

                    match connect_to_proxy_service_ready
                        .call(ConnectToProxyRequest {
                            proxy_address: None,
                            client_address: request.client_address,
                        })
                        .await
                    {
                        Err(e) => {
                            error!(
                                "Fail to invoke connect proxy service because of error(call): {:#?}",
                                e
                            );
                            let connect_result = Socks5ConnectCommandResult::new(
                                Socks5ConnectCommandResultStatus::Failure,
                                None,
                            );
                            framed.send(connect_result).await?;
                            return Err(e);
                        }
                        Ok(r) => r,
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
                .call(connect_to_proxy_response.proxy_stream)
                .await?;

            let mut message_framed_write = framed_result.message_framed_write;

            write_proxy_message_service
                .ready()
                .await?
                .call(WriteMessageServiceRequest {
                    message_framed_write,
                    payload_encryption_type: PayloadEncryptionType::Blowfish(
                        generate_uuid().into(),
                    ),
                    user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                    ref_id: None,
                    message_payload: MessagePayload::new(),
                })
                .await?;

            let mut message_framed_read = framed_result.message_framed_read;

            let connect_result = Socks5ConnectCommandResult::new(
                Socks5ConnectCommandResultStatus::Succeeded,
                Some(connect_command.dest_address),
            );
            framed.send(connect_result).await?;
            Ok(Socks5ConnectFlowResult {
                client_stream: request.client_stream,
                client_address: request.client_address,
                message_framed_read,
                message_framed_write,
            })
        })
    }
}
