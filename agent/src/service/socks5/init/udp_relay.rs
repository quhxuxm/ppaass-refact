use std::{net::SocketAddr, sync::Arc};

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, RsaCryptoFetcher,
    WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use futures::SinkExt;
use tokio::net::{TcpStream, UdpSocket};
use tracing::error;

use crate::{config::AgentConfig, message::socks5::Socks5UdpDataCommandContent};

const BUFFER_SIZE_64KB: usize = 1024 * 64;
pub struct Socks5UdpRelayFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub associated_udp_socket: UdpSocket,
    pub associated_udp_address: SocketAddr,
    pub connection_id: String,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: SocketAddr,
}
pub struct Socks5UdpRelayFlowResult {}
pub struct Socks5UdpRelayFlow;

impl Socks5UdpRelayFlow {
    pub async fn exec<T>(request: Socks5UdpRelayFlowRequest<T>, rsa_crypto_fetcher: Arc<T>, configuration: Arc<AgentConfig>) -> Result<Socks5UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let Socks5UdpRelayFlowRequest {
            associated_udp_socket,
            associated_udp_address,
            connection_id,
            client_stream,
            client_address,
            mut message_framed_write,
            message_framed_read,
            source_address,
            target_address,
            init_data,
            proxy_address,
        } = request;
        let user_token = configuration.user_token().clone().unwrap();
        let user_token_clone = user_token.clone();
        tokio::spawn(async move {
            loop {
                let mut buffer = [0u8; BUFFER_SIZE_64KB];
                match associated_udp_socket.recv(&mut buffer).await {
                    Err(e) => {
                        error!(
                            "Connection [{}] fail to receive udp package from client, because of error: {:#?}",
                            connection_id, e
                        );
                        continue;
                    },
                    Ok(size) => {
                        let received_data = Bytes::copy_from_slice(&buffer[0..size]);
                        let socks5_udp_data: Socks5UdpDataCommandContent = match received_data.try_into() {
                            Err(e) => {
                                error!(
                                    "Connection [{}] fail to convert socks5 udp data packet because of error: {:#?}",
                                    connection_id, e
                                );
                                continue;
                            },
                            Ok(v) => v,
                        };
                        let destination_address = socks5_udp_data.address;
                        let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token_clone.clone(),
                        })
                        .await
                        {
                            Err(e) => {
                                error!("Fail to select payload encryption type because of error: {:#?}", e);
                                continue;
                            },
                            Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
                        };
                        let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                            connection_id: Some(connection_id.clone()),
                            message_framed_write,
                            ref_id: Some(connection_id.clone()),
                            user_token: configuration.user_token().clone().unwrap(),
                            payload_encryption_type,
                            message_payload: Some(MessagePayload::new(
                                client_address.into(),
                                destination_address.into(),
                                PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpData),
                                socks5_udp_data.data,
                            )),
                        })
                        .await;
                        match write_agent_message_result {
                            Err(WriteMessageFramedError {
                                message_framed_write: message_framed_write_from_error,
                                source,
                            }) => {
                                error!("Fail to write agent message to proxy because of error: {:#?}", source);
                                message_framed_write = message_framed_write_from_error;
                                continue;
                            },
                            Ok(WriteMessageFramedResult {
                                message_framed_write: message_framed_write_from_result,
                            }) => {
                                message_framed_write = message_framed_write_from_result;
                                if let Err(e) = message_framed_write.flush().await {
                                    error!("Fail to flush agent message to proxy because of error: {:#?}", e);
                                    continue;
                                };
                            },
                        };
                    },
                };
            }
        });
        Ok(Socks5UdpRelayFlowResult {})
    }
}
