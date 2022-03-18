use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::{Service, ServiceExt};
use tracing::log::{debug, error};

use common::CommonError;

use crate::codec::socks5::Socks5ConnectCodec;
use crate::command::socks5::{
    Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus, Socks5ConnectCommandType,
};
use crate::service::common::{ConnectToProxyRequest, ConnectToProxyService};
use crate::SERVER_CONFIG;

const DEFAULT_RETRY_TIMES: u16 = 3;

#[derive(Debug)]
pub(crate) struct Socks5ConnectFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

#[derive(Debug)]
pub(crate) struct Socks5ConnectFlowResult {
    pub client_stream: TcpStream,
    pub proxy_stream: TcpStream,
    pub client_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Socks5ConnectCommandService;

impl Service<Socks5ConnectFlowRequest> for Socks5ConnectCommandService {
    type Response = Socks5ConnectFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5ConnectFlowRequest) -> Self::Future {
        Box::pin(async move {
            let mut framed =
                Framed::with_capacity(&mut request.client_stream, Socks5ConnectCodec, 2048);
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
                            //TODO need change
                            proxy_address: Some(connect_command.dest_address.to_string()),
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
            let connect_result = Socks5ConnectCommandResult::new(
                Socks5ConnectCommandResultStatus::Succeeded,
                Some(connect_command.dest_address),
            );
            framed.send(connect_result).await?;
            Ok(Socks5ConnectFlowResult {
                client_stream: request.client_stream,
                client_address: request.client_address,
                proxy_stream: connect_to_proxy_response.proxy_stream,
            })
        })
    }
}
