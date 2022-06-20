use std::{
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use anyhow::Result;
use bytes::{Buf, Bytes};
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError, WriteMessageFramedRequest,
    WriteMessageFramedResult,
};
use futures::SinkExt;
use pretty_hex::*;
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error};

use crate::{config::AgentConfig, message::socks5::Socks5UdpDataPacket};

const SIZE_64KB: usize = 65535;
pub struct Socks5UdpRelayFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub associated_udp_socket: UdpSocket,
    pub associated_udp_address: SocketAddr,
    pub connection_id: String,
    pub client_stream: TcpStream,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
    pub client_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: SocketAddr,
}
pub struct Socks5UdpRelayFlowResult {}
pub struct Socks5UdpRelayFlow;

impl Socks5UdpRelayFlow {
    pub async fn exec<T>(request: Socks5UdpRelayFlowRequest<T>, configuration: Arc<AgentConfig>) -> Result<Socks5UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let Socks5UdpRelayFlowRequest {
            associated_udp_socket,
            connection_id,
            client_address,
            mut message_framed_write,
            mut message_framed_read,
            ..
        } = request;
        let user_token = configuration.user_token().clone().unwrap();
        let user_token_clone = user_token.clone();
        let connection_id_a2p = connection_id.clone();
        let associated_udp_socket = Arc::new(associated_udp_socket);
        let associated_udp_socket_a2p = associated_udp_socket.clone();
        let client_address_a2p = client_address.clone();
        tokio::spawn(async move {
            loop {
                let mut buffer = [0u8; SIZE_64KB];
                match associated_udp_socket_a2p.recv(&mut buffer).await {
                    Err(e) => {
                        error!(
                            "Connection [{}] fail to receive udp package from client, because of error: {:#?}",
                            connection_id_a2p, e
                        );
                        return;
                    },
                    Ok(size) => {
                        let received_data = Bytes::copy_from_slice(&buffer[0..size]);
                        debug!(
                            "Connection [{}] receive client udp packet: \n\n{}\n\n",
                            connection_id_a2p,
                            pretty_hex(&received_data)
                        );
                        let socks5_udp_data: Socks5UdpDataPacket = match received_data.try_into() {
                            Err(e) => {
                                error!(
                                    "Connection [{}] fail to convert socks5 udp data packet because of error: {:#?}",
                                    connection_id_a2p, e
                                );
                                return;
                            },
                            Ok(v) => v,
                        };
                        let udp_destination_address = socks5_udp_data.address;
                        let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token_clone.clone(),
                        })
                        .await
                        {
                            Err(e) => {
                                error!(
                                    "Connection [{}] fail to select payload encryption type because of error: {:#?}",
                                    connection_id_a2p, e
                                );
                                return;
                            },
                            Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
                        };
                        let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                            connection_id: Some(connection_id_a2p.clone()),
                            message_framed_write,
                            ref_id: Some(connection_id_a2p.clone()),
                            user_token: configuration.user_token().clone().unwrap(),
                            payload_encryption_type,
                            message_payload: Some(MessagePayload {
                                source_address: Some(client_address_a2p.clone()),
                                target_address: Some(udp_destination_address.into()),
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpData),
                                data: socks5_udp_data.data,
                            }),
                        })
                        .await;
                        match write_agent_message_result {
                            Err(WriteMessageFramedError { source, .. }) => {
                                error!(
                                    "Connection [{}] fail to write agent message to proxy because of error: {:#?}",
                                    connection_id_a2p, source
                                );
                                return;
                            },
                            Ok(WriteMessageFramedResult {
                                message_framed_write: message_framed_write_from_result,
                            }) => {
                                message_framed_write = message_framed_write_from_result;
                                if let Err(e) = message_framed_write.flush().await {
                                    error!(
                                        "Connection [{}] fail to flush agent message to proxy because of error: {:#?}",
                                        connection_id_a2p, e
                                    );
                                    return;
                                };
                            },
                        };
                    },
                };
            }
        });
        tokio::spawn(async move {
            loop {
                match MessageFramedReader::read(common::ReadMessageFramedRequest {
                    connection_id: connection_id.clone(),
                    message_framed_read,
                })
                .await
                {
                    Err(ReadMessageFramedError { source, .. }) => {
                        error!("Connection [{}] fail to read data from proxy because of error:{:#?}", connection_id, source);
                        return;
                    },
                    Ok(ReadMessageFramedResult {
                        message_framed_read: message_framed_read_from_result,
                        content:
                            Some(ReadMessageFramedResultContent {
                                message_payload:
                                    Some(MessagePayload {
                                        source_address: Some(source_address),
                                        target_address: Some(target_address),
                                        payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpData),
                                        data,
                                    }),
                                ..
                            }),
                    }) => {
                        let send_to_address = match source_address.to_socket_addrs() {
                            Err(e) => {
                                error!(
                                    "Connection [{}] fail to forward proxy udp message to client [{:?}] because of fail to convert socket address, error: {:#?}",
                                    connection_id, source_address, e
                                );
                                return;
                            },
                            Ok(v) => v,
                        };
                        debug!(
                            "Connection [{}] receive udp data from target, forward udp packet to client [{:?}]:\n\n{}\n\n",
                            connection_id,
                            client_address,
                            pretty_hex::pretty_hex(&data)
                        );
                        let socks5_udp_packet = Socks5UdpDataPacket {
                            frag: 0,
                            address: target_address.clone().into(),
                            data,
                        };
                        let socks5_udp_packet_bytes: Bytes = socks5_udp_packet.into();
                        if let Err(e) = associated_udp_socket
                            .send_to(socks5_udp_packet_bytes.chunk(), send_to_address.collect::<Vec<_>>().as_slice())
                            .await
                        {
                            error!(
                                "Connection [{}] fail to forward proxy udp message to client [{:?}], error: {:#?}",
                                connection_id, source_address, e
                            );
                            return;
                        };
                        message_framed_read = message_framed_read_from_result;
                    },
                    Ok(ReadMessageFramedResult { .. }) => {
                        error!(
                            "Connection [{}] fail to forward proxy udp message because of invalid proxy message payload",
                            connection_id
                        );

                        return;
                    },
                };
            }
        });
        Ok(Socks5UdpRelayFlowResult {})
    }
}
