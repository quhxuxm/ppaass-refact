use std::{
    fmt::Debug,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use anyhow::Result;
use bytes::{Buf, Bytes};
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use pretty_hex::*;
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error, instrument};

use crate::{config::AgentConfig, message::socks5::Socks5UdpDataPacket, service::pool::ProxyConnection};

const SIZE_64KB: usize = 65535;

#[derive(Debug)]
pub struct Socks5UdpRelayFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub associated_udp_socket: UdpSocket,
    pub associated_udp_address: SocketAddr,
    pub client_connection_id: &'a str,
    pub proxy_connection_id: &'a str,
    pub client_stream: TcpStream,
    pub message_framed_write: MessageFramedWrite<T, ProxyConnection>,
    pub message_framed_read: MessageFramedRead<T, ProxyConnection>,
    pub client_address: NetAddress,
    pub target_address: NetAddress,
    pub init_data: Option<Vec<u8>>,
    pub proxy_address: SocketAddr,
}

#[derive(Debug)]
pub struct Socks5UdpRelayFlowResult {}
pub struct Socks5UdpRelayFlow;

impl Socks5UdpRelayFlow {
    #[instrument(skip_all, fields(request.client_connection_id, request.proxy_connection_id))]
    pub async fn exec<'a, T>(request: Socks5UdpRelayFlowRequest<'a, T>, configuration: Arc<AgentConfig>) -> Result<Socks5UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let Socks5UdpRelayFlowRequest {
            associated_udp_socket,
            client_connection_id,
            proxy_connection_id,
            client_address,
            mut message_framed_write,
            mut message_framed_read,
            ..
        } = request;
        let user_token = configuration.user_token().clone().expect("Can not get user token");
        let client_connection_id_a2p = client_connection_id.to_owned();
        let proxy_connection_id = proxy_connection_id.to_owned();
        let associated_udp_socket = Arc::new(associated_udp_socket);
        let associated_udp_socket_a2p = associated_udp_socket.clone();
        let client_address_a2p = client_address.clone();
        tokio::spawn(async move {
            loop {
                let mut buffer = [0u8; SIZE_64KB];
                match associated_udp_socket_a2p.recv(&mut buffer).await {
                    Err(e) => {
                        error!(
                            "Client connection [{}] fail to receive udp package from client, because of error: {:#?}",
                            client_connection_id_a2p, e
                        );
                        return;
                    },
                    Ok(size) => {
                        let received_data = Bytes::copy_from_slice(&buffer[0..size]);
                        debug!(
                            "Client connection [{}] receive client udp packet: \n\n{}\n\n",
                            client_connection_id_a2p,
                            pretty_hex(&received_data)
                        );
                        let socks5_udp_data: Socks5UdpDataPacket = match received_data.try_into() {
                            Err(e) => {
                                error!(
                                    "Client connection [{}] fail to convert socks5 udp data packet because of error: {:#?}",
                                    client_connection_id_a2p, e
                                );
                                return;
                            },
                            Ok(v) => v,
                        };
                        let udp_destination_address = socks5_udp_data.address;
                        let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token.as_str(),
                        })
                        .await
                        {
                            Err(e) => {
                                error!(
                                    "Client connection [{}] fail to select payload encryption type because of error: {:#?}",
                                    client_connection_id_a2p, e
                                );
                                return;
                            },
                            Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
                        };
                        let write_agent_message_result = MessageFramedWriter::write(WriteMessageFramedRequest {
                            connection_id: Some(client_connection_id_a2p.as_str()),
                            message_framed_write,
                            ref_id: Some(client_connection_id_a2p.as_str()),
                            user_token: configuration.user_token().clone().expect("Can not get user token").as_str(),
                            payload_encryption_type,
                            message_payloads: Some(vec![MessagePayload {
                                source_address: Some(client_address_a2p.clone()),
                                target_address: Some(udp_destination_address.into()),
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpData),
                                data: Some(socks5_udp_data.data),
                            }]),
                        })
                        .await;
                        match write_agent_message_result {
                            Err(WriteMessageFramedError { source, .. }) => {
                                error!(
                                    "Client connection [{}] fail to write agent message to proxy because of error: {:#?}",
                                    client_connection_id_a2p, source
                                );
                                return;
                            },
                            Ok(WriteMessageFramedResult {
                                message_framed_write: message_framed_write_from_result,
                            }) => {
                                message_framed_write = message_framed_write_from_result;
                            },
                        };
                    },
                };
            }
        });
        tokio::spawn(async move {
            loop {
                match MessageFramedReader::read(ReadMessageFramedRequest {
                    connection_id: proxy_connection_id.as_str(),
                    message_framed_read,
                    timeout: None,
                })
                .await
                {
                    Err(ReadMessageFramedError { source, .. }) => {
                        error!(
                            "Proxy connection [{}] fail to read data from proxy because of error:{:#?}",
                            proxy_connection_id, source
                        );
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
                                        data: Some(data),
                                    }),
                                ..
                            }),
                    }) => {
                        let send_to_address = match source_address.to_socket_addrs() {
                            Err(e) => {
                                error!(
                                    "Proxy connection [{}] fail to forward proxy udp message to client [{:?}] because of fail to convert socket address, error: {:#?}",
                                    proxy_connection_id, source_address, e
                                );
                                return;
                            },
                            Ok(v) => v,
                        };
                        debug!(
                            "Proxy connection [{}] receive udp data from target, forward udp packet to client [{:?}]:\n\n{}\n\n",
                            proxy_connection_id,
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
                                "Proxy connection [{}] fail to forward proxy udp message to client [{:?}], error: {:#?}",
                                proxy_connection_id, source_address, e
                            );
                            return;
                        };
                        message_framed_read = message_framed_read_from_result;
                    },
                    Ok(ReadMessageFramedResult { .. }) => {
                        error!(
                            "Proxy connection [{}] fail to forward proxy udp message because of invalid proxy message payload",
                            proxy_connection_id
                        );

                        return;
                    },
                };
            }
        });
        Ok(Socks5UdpRelayFlowResult {})
    }
}
