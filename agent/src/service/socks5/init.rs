use std::task::Poll;
use std::{fmt::Debug, net::SocketAddr};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;
use tower::ServiceBuilder;
use tracing::log::{debug, error};

use common::{
    ready_and_call_service, CommonError, MessageFramedRead, MessageFramedWrite, NetAddress,
};

use crate::service::common::DEFAULT_BUFFER_SIZE;
use crate::SERVER_CONFIG;
use crate::{
    codec::socks5::Socks5InitCodec,
    service::socks5::init::tcp_connect::Socks5TcpConnectServiceRequest,
};
use crate::{
    command::socks5::{
        Socks5InitCommandResult, Socks5InitCommandResultStatus, Socks5InitCommandType,
    },
    service::socks5::init::tcp_connect::Socks5TcpConnectServiceResponse,
};

mod tcp_connect;
mod udp_associate;
use tcp_connect::Socks5TcpConnectService;

pub(crate) type Socks5InitFramed<'a> = Framed<&'a mut TcpStream, Socks5InitCodec>;

#[derive(Debug)]
pub(crate) struct Socks5InitCommandServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

pub(crate) struct Socks5InitCommandServiceResult {
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
pub(crate) struct Socks5InitCommandService;

impl Service<Socks5InitCommandServiceRequest> for Socks5InitCommandService {
    type Response = Socks5InitCommandServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5InitCommandServiceRequest) -> Self::Future {
        Box::pin(async move {
            let mut client_stream = request.client_stream;
            let client_address = request.client_address;
            let mut socks5_init_framed = Framed::with_capacity(
                &mut client_stream,
                Socks5InitCodec,
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );

            let init_command = match socks5_init_framed.next().await {
                Some(Ok(v)) => v,
                _ => {
                    send_socks5_init_failure(&mut socks5_init_framed).await?;
                    return Err(CommonError::CodecError);
                }
            };
            debug!(
                "Client {} send socks 5 connect command: {:#?}",
                client_address, init_command
            );
            let dest_address = init_command.dest_address;
            match init_command.request_type {
                Socks5InitCommandType::Connect => {
                    let mut socks5_tcp_connect_service =
                        ServiceBuilder::new().service(Socks5TcpConnectService::default());

                    match ready_and_call_service(
                        &mut socks5_tcp_connect_service,
                        Socks5TcpConnectServiceRequest {
                            client_address,
                            dest_address: dest_address.clone(),
                        },
                    )
                    .await
                    {
                        Err(e) => {
                            error!("Fail to handle socks5 init command (CONNECT) because of error: {:#?}", e);
                            send_socks5_init_failure(&mut socks5_init_framed).await?;
                            Err(e)
                        }
                        Ok(Socks5TcpConnectServiceResponse {
                            message_framed_read,
                            message_framed_write,
                            client_address,
                            source_address,
                            target_address,
                            proxy_address_string,
                            connect_response_message_id,
                        }) => {
                            //Response for socks5 connect command
                            let init_command_result = Socks5InitCommandResult::new(
                                Socks5InitCommandResultStatus::Succeeded,
                                Some(dest_address),
                            );
                            socks5_init_framed.send(init_command_result).await?;
                            socks5_init_framed.flush().await?;
                            Ok(Socks5InitCommandServiceResult {
                                client_stream,
                                client_address,
                                message_framed_read,
                                message_framed_write,
                                source_address,
                                target_address,
                                proxy_address_string,
                                connect_response_message_id,
                            })
                        }
                    }
                }
                Socks5InitCommandType::Bind => {
                    todo!()
                }
                Socks5InitCommandType::UdpAssociate => {
                    todo!()
                }
            }
        })
    }
}

async fn send_socks5_init_failure<'a>(
    socks5_client_framed: &mut Socks5InitFramed<'a>,
) -> Result<(), CommonError> {
    let connect_result = Socks5InitCommandResult::new(Socks5InitCommandResultStatus::Failure, None);
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
    Ok(())
}
