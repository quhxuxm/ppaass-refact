use std::{fmt::Debug, net::SocketAddr};
use std::fmt::Formatter;
use std::sync::Arc;
use std::task::Poll;

use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use futures::future::BoxFuture;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};
use tower::Service;
use tower::ServiceBuilder;
use tracing::log::{debug, error};

use common::{
    MessageFramedRead, MessageFramedWrite, NetAddress, PpaassError, ready_and_call_service,
    RsaCryptoFetcher,
};
use tcp_connect::Socks5TcpConnectService;

use crate::{
    codec::socks5::Socks5InitCommandContentCodec,
    service::socks5::init::tcp_connect::Socks5TcpConnectServiceRequest,
};
use crate::message::socks5::{
    Socks5InitCommandResultContent, Socks5InitCommandResultStatus, Socks5InitCommandType,
};
use crate::service::socks5::init::tcp_connect::Socks5TcpConnectServiceResponse;
use crate::service::socks5::init::udp_associate::{
    Socks5UdpAssociateService, Socks5UdpAssociateServiceRequest, Socks5UdpAssociateServiceResponse,
};

mod tcp_connect;
mod udp_associate;
pub(crate) type Socks5InitFramed<'a> = Framed<&'a mut TcpStream, Socks5InitCommandContentCodec>;

pub(crate) struct Socks5InitCommandServiceRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

impl Debug for Socks5InitCommandServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Socks5InitCommandServiceRequest: proxy_addresses={:#?}, client_address={}",
            self.proxy_addresses, self.client_address
        )
    }
}

pub(crate) struct Socks5InitCommandServiceResult<T>
where
    T: RsaCryptoFetcher,
{
    pub client_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub connect_response_message_id: String,
    pub proxy_address: Option<SocketAddr>,
}

#[derive(Clone, Debug)]
pub(crate) struct Socks5InitCommandService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}
impl<T> Socks5InitCommandService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }
}
impl<T> Service<Socks5InitCommandServiceRequest> for Socks5InitCommandService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = Socks5InitCommandServiceResult<T>;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5InitCommandServiceRequest) -> Self::Future {
        let rsa_crypto_fetcher = self.rsa_crypto_fetcher.clone();
        Box::pin(async move {
            let mut client_stream = request.client_stream;
            let client_address = request.client_address;
            let mut framed_parts =
                FramedParts::new(&mut client_stream, Socks5InitCommandContentCodec);
            framed_parts.read_buf = request.buffer;
            let mut socks5_init_framed = Framed::from_parts(framed_parts);
            let init_command = match socks5_init_framed.next().await {
                Some(Ok(v)) => v,
                _ => {
                    send_socks5_init_failure(&mut socks5_init_framed).await?;
                    return Err(PpaassError::CodecError);
                },
            };
            debug!(
                "Client {} send socks 5 connect command: {:#?}",
                client_address, init_command
            );
            let dest_address = init_command.dest_address;
            match init_command.request_type {
                Socks5InitCommandType::Connect => {
                    let mut socks5_tcp_connect_service = ServiceBuilder::new()
                        .service(Socks5TcpConnectService::new(rsa_crypto_fetcher));
                    match ready_and_call_service(
                        &mut socks5_tcp_connect_service,
                        Socks5TcpConnectServiceRequest {
                            proxy_addresses: request.proxy_addresses,
                            client_address,
                            dest_address: dest_address.clone(),
                        },
                    )
                        .await
                    {
                        Err(e) => {
                            error!(
                                "Fail to handle socks5 init command (CONNECT) because of error: {:#?}",
                                e
                            );
                            send_socks5_init_failure(&mut socks5_init_framed).await?;
                            Err(e)
                        },
                        Ok(Socks5TcpConnectServiceResponse {
                            message_framed_read,
                            message_framed_write,
                            client_address,
                            source_address,
                            target_address,
                            connect_response_message_id,
                            proxy_address,
                        }) => {
                            //Response for socks5 connect command
                            let init_command_result = Socks5InitCommandResultContent::new(
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
                                connect_response_message_id,
                                proxy_address,
                            })
                        },
                    }
                },
                Socks5InitCommandType::Bind => {
                    todo!()
                },
                Socks5InitCommandType::UdpAssociate => {
                    let mut socks5_udp_associate_service = ServiceBuilder::new()
                        .service(Socks5UdpAssociateService::new(rsa_crypto_fetcher));
                    match ready_and_call_service(
                        &mut socks5_udp_associate_service,
                        Socks5UdpAssociateServiceRequest {
                            proxy_addresses: request.proxy_addresses,
                            client_address,
                        },
                    )
                        .await
                    {
                        Err(e) => {
                            error!(
                                "Fail to handle socks5 init command (UDP ASSOCIATE) because of error: {:#?}",
                                e
                            );
                            send_socks5_init_failure(&mut socks5_init_framed).await?;
                            Err(e)
                        },
                        Ok(Socks5UdpAssociateServiceResponse {
                            message_framed_read,
                            message_framed_write,
                            source_address,
                            target_address,
                            connect_response_message_id,
                            proxy_address,
                        }) => {
                            //Response for socks5 connect command
                            let init_command_result = Socks5InitCommandResultContent::new(
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
                                connect_response_message_id,
                                proxy_address,
                            })
                        },
                    }
                },
            }
        })
    }
}

async fn send_socks5_init_failure(
    socks5_client_framed: &mut Socks5InitFramed<'_>,
) -> Result<(), PpaassError> {
    let connect_result =
        Socks5InitCommandResultContent::new(Socks5InitCommandResultStatus::Failure, None);
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
        return Err(PpaassError::UnknownError);
    }
    Ok(())
}
