use std::sync::Arc;

use std::net::SocketAddr;

use anyhow::anyhow;
use bytes::BytesMut;
use common::{MessageFramedRead, MessageFramedWrite, NetAddress, PpaassError, RsaCryptoFetcher};

use futures::{SinkExt, StreamExt};
use tcp_connect::Socks5TcpConnectFlow;
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::Mutex,
};
use tokio_util::codec::{Framed, FramedParts};

use tracing::{debug, error};

use crate::service::{
    common::ProxyConnectionPool,
    socks5::init::{
        tcp_connect::Socks5TcpConnectFlowResult,
        udp_associate::{Socks5UdpAssociateFlow, Socks5UdpAssociateFlowRequest, Socks5UdpAssociateFlowResult},
    },
};

use crate::{codec::socks5::Socks5InitCommandContentCodec, service::socks5::init::tcp_connect::Socks5TcpConnectFlowRequest};
use crate::{
    config::AgentConfig,
    message::socks5::{Socks5InitCommandResultContent, Socks5InitCommandResultStatus, Socks5InitCommandType},
};
use anyhow::Result;

mod tcp_connect;
mod udp_associate;
mod udp_relay;

pub use udp_relay::*;

pub(crate) type Socks5InitFramed<'a> = Framed<&'a mut TcpStream, Socks5InitCommandContentCodec>;

pub(crate) enum Socks5InitFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    Tcp {
        client_stream: TcpStream,
        message_framed_read: MessageFramedRead<T>,
        message_framed_write: MessageFramedWrite<T>,
        client_address: SocketAddr,
        source_address: NetAddress,
        target_address: NetAddress,
        proxy_address: SocketAddr,
    },
    Udp {
        associated_udp_socket: UdpSocket,
        associated_udp_address: SocketAddr,
        client_stream: TcpStream,
        message_framed_read: MessageFramedRead<T>,
        message_framed_write: MessageFramedWrite<T>,
        client_address: NetAddress,
        proxy_address: SocketAddr,
    },
}
pub(crate) struct Socks5InitFlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

pub(crate) struct Socks5InitFlow;

impl Socks5InitFlow {
    pub async fn exec<T>(
        request: Socks5InitFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>, proxy_connection_pool: Arc<Mutex<ProxyConnectionPool>>,
    ) -> Result<Socks5InitFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let Socks5InitFlowRequest {
            connection_id,
            proxy_addresses,
            mut client_stream,
            client_address,
            buffer,
        } = request;

        let mut framed_parts = FramedParts::new(&mut client_stream, Socks5InitCommandContentCodec);
        framed_parts.read_buf = buffer;
        let mut socks5_init_framed = Framed::from_parts(framed_parts);
        let init_command = match socks5_init_framed.next().await {
            Some(Ok(v)) => v,
            _ => {
                error!(
                    "Connection [{}] fail to handle socks 5 init command from client {}",
                    connection_id, client_address
                );
                send_socks5_init_failure(&mut socks5_init_framed).await?;
                return Err(anyhow!(PpaassError::CodecError));
            },
        };
        debug!(
            "Connection [{}] send socks 5 connect command to client {} : {:#?}",
            connection_id, client_address, init_command
        );
        let dest_address_in_init_command = init_command.dest_address;
        match init_command.request_type {
            Socks5InitCommandType::Connect => {
                match Socks5TcpConnectFlow::exec(
                    Socks5TcpConnectFlowRequest {
                        connection_id: connection_id.clone(),
                        client_address,
                        dest_address: dest_address_in_init_command.clone(),
                    },
                    rsa_crypto_fetcher,
                    configuration,
                    proxy_connection_pool,
                )
                .await
                {
                    Err(e) => {
                        error!(
                            "Connection [{}] fail to handle socks5 init command (CONNECT) because of error: {:#?}",
                            connection_id, e
                        );
                        send_socks5_init_failure(&mut socks5_init_framed).await?;
                        Err(e)
                    },
                    Ok(Socks5TcpConnectFlowResult {
                        message_framed_read,
                        message_framed_write,
                        client_address,
                        source_address,
                        target_address,
                        proxy_address,
                    }) => {
                        //Response for socks5 connect command
                        let init_command_result =
                            Socks5InitCommandResultContent::new(Socks5InitCommandResultStatus::Succeeded, Some(dest_address_in_init_command));
                        socks5_init_framed.send(init_command_result).await?;
                        socks5_init_framed.flush().await?;
                        Ok(Socks5InitFlowResult::Tcp {
                            client_stream,
                            client_address,
                            message_framed_read,
                            message_framed_write,
                            source_address,
                            target_address,
                            proxy_address,
                        })
                    },
                }
            },
            Socks5InitCommandType::UdpAssociate => {
                match Socks5UdpAssociateFlow::exec(
                    Socks5UdpAssociateFlowRequest {
                        connection_id: connection_id.clone(),
                        proxy_addresses,
                        client_address: dest_address_in_init_command.clone(),
                    },
                    rsa_crypto_fetcher,
                    configuration,
                )
                .await
                {
                    Err(e) => {
                        error!(
                            "Connection [{}] fail to handle socks5 init command (UDP ASSOCIATE) because of error: {:#?}",
                            connection_id, e
                        );
                        send_socks5_init_failure(&mut socks5_init_framed).await?;
                        Err(e)
                    },
                    Ok(Socks5UdpAssociateFlowResult {
                        associated_udp_socket,
                        associated_udp_address,
                        message_framed_read,
                        message_framed_write,
                        client_address,
                        proxy_address,
                    }) => {
                        //Response for socks5 udp associate command
                        let init_command_result =
                            Socks5InitCommandResultContent::new(Socks5InitCommandResultStatus::Succeeded, Some(associated_udp_address.clone()));
                        socks5_init_framed.send(init_command_result).await?;
                        socks5_init_framed.flush().await?;
                        Ok(Socks5InitFlowResult::Udp {
                            associated_udp_address: associated_udp_address.try_into()?,
                            associated_udp_socket,
                            client_stream,
                            client_address,
                            message_framed_read,
                            message_framed_write,
                            proxy_address,
                        })
                    },
                }
            },
            Socks5InitCommandType::Bind => {
                todo!()
            },
        }
    }
}

async fn send_socks5_init_failure(socks5_client_framed: &mut Socks5InitFramed<'_>) -> Result<(), PpaassError> {
    let connect_result = Socks5InitCommandResultContent::new(Socks5InitCommandResultStatus::Failure, None);
    if let Err(e) = socks5_client_framed.send(connect_result).await {
        error!("Fail to write socks5 connect fail result to client because of error: {:#?}", e);
        return Err(e);
    };
    socks5_client_framed.flush().await?;
    socks5_client_framed.close().await?;
    Ok(())
}
