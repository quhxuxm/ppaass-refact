use std::sync::Arc;

use std::net::SocketAddr;

use anyhow::anyhow;
use bytes::BytesMut;
use common::{MessageFramedRead, MessageFramedWrite, NetAddress, PpaassError, RsaCryptoFetcher};

use futures::{SinkExt, StreamExt};
use tcp_connect::Socks5TcpConnectFlow;
use tokio::net::{TcpStream, UdpSocket};
use tokio_util::codec::{Framed, FramedParts};

use tracing::{debug, error};

use crate::service::socks5::init::{
    tcp_connect::Socks5TcpConnectFlowResult,
    udp_associate::{Socks5UdpAssociateFlow, Socks5UdpAssociateFlowRequest, Socks5UdpAssociateFlowResult},
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

pub(crate) enum Socks5InitFlowResultRelayType {
    Tcp,
    Udp(UdpSocket),
}
pub(crate) struct Socks5InitFlowRequest {
    pub connection_id: String,
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

pub(crate) struct Socks5InitFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub client_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub client_address: SocketAddr,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub proxy_address: SocketAddr,
    pub relay_type: Socks5InitFlowResultRelayType,
}

pub(crate) struct Socks5InitFlow;

impl Socks5InitFlow {
    pub async fn exec<T>(request: Socks5InitFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>) -> Result<Socks5InitFlowResult<T>>
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
                send_socks5_init_failure(&mut socks5_init_framed).await?;
                return Err(anyhow!(PpaassError::CodecError));
            },
        };
        debug!("Client {} send socks 5 connect command: {:#?}", client_address, init_command);
        let dest_address = init_command.dest_address;
        match init_command.request_type {
            Socks5InitCommandType::Connect => {
                match Socks5TcpConnectFlow::exec(
                    Socks5TcpConnectFlowRequest {
                        connection_id,
                        proxy_addresses,
                        client_address,
                        dest_address: dest_address.clone(),
                    },
                    rsa_crypto_fetcher,
                    configuration,
                )
                .await
                {
                    Err(e) => {
                        error!("Fail to handle socks5 init command (CONNECT) because of error: {:#?}", e);
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
                        let init_command_result = Socks5InitCommandResultContent::new(Socks5InitCommandResultStatus::Succeeded, Some(dest_address));
                        socks5_init_framed.send(init_command_result).await?;
                        socks5_init_framed.flush().await?;
                        Ok(Socks5InitFlowResult {
                            relay_type: Socks5InitFlowResultRelayType::Tcp,
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
                        connection_id,
                        proxy_addresses,
                        client_address,
                        dest_address: dest_address.clone(),
                    },
                    rsa_crypto_fetcher,
                    configuration,
                )
                .await
                {
                    Err(e) => {
                        error!("Fail to handle socks5 init command (UDP ASSOCIATE) because of error: {:#?}", e);
                        send_socks5_init_failure(&mut socks5_init_framed).await?;
                        Err(e)
                    },
                    Ok(Socks5UdpAssociateFlowResult {
                        associated_udp_socket,
                        message_framed_read,
                        message_framed_write,
                        client_address,
                        proxy_address,
                        source_address,
                        target_address,
                    }) => {
                        //Response for socks5 udp associate command
                        let init_command_result = Socks5InitCommandResultContent::new(Socks5InitCommandResultStatus::Succeeded, Some(dest_address));
                        socks5_init_framed.send(init_command_result).await?;
                        socks5_init_framed.flush().await?;
                        Ok(Socks5InitFlowResult {
                            relay_type: Socks5InitFlowResultRelayType::Udp(associated_udp_socket),
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
