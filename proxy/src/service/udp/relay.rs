use std::{
    fmt::Debug,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs},
};

use anyhow::Result;
use bytes::Bytes;
use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};
use pretty_hex;
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error, instrument};

use pretty_hex::*;
const SIZE_64KB: usize = 65535;

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct UdpRelayFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_id: &'a str,
    pub user_token: &'a str,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
}

#[derive(Debug)]
pub(crate) struct UdpRelayFlowResult;
pub(crate) struct UdpRelayFlow;

impl UdpRelayFlow {
    #[instrument(skip_all, fields(request.connection_id))]
    pub async fn exec<'a, T>(
        UdpRelayFlowRequest {
            connection_id,
            message_id,
            user_token,
            mut message_framed_read,
            mut message_framed_write,
            ..
        }: UdpRelayFlowRequest<'a, T>,
    ) -> Result<UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let connection_id = connection_id.to_owned();
        let message_id = message_id.to_owned();
        let user_token = user_token.to_owned();
        tokio::spawn(async move {
            loop {
                match MessageFramedReader::read(ReadMessageFramedRequest {
                    connection_id: connection_id.as_str(),
                    message_framed_read,
                    timeout: None,
                })
                .await
                {
                    Err(ReadMessageFramedError { source, .. }) => {
                        error!("Connection [{}] has a error when read from agent, error: {:#?}.", connection_id, source);
                        return;
                    },
                    Ok(ReadMessageFramedResult { content: None, .. }) => {
                        debug!(
                            "Connection [{}] nothing to read, message id:{}, user token:{}",
                            connection_id, message_id, user_token
                        );
                        return;
                    },
                    Ok(ReadMessageFramedResult {
                        message_framed_read: message_framed_read_return_back,
                        content:
                            Some(ReadMessageFramedResultContent {
                                message_id,
                                message_payload:
                                    Some(MessagePayload {
                                        source_address,
                                        target_address: Some(target_address),
                                        payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpData),
                                        data: Some(data),
                                    }),
                                user_token,
                            }),
                    }) => {
                        let udp_socket = match UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0))).await {
                            Err(e) => {
                                error!("Connection [{}] fail to create udp socket because of error : {:#?}", connection_id, e);
                                return;
                            },
                            Ok(v) => v,
                        };
                        let udp_target_addresses = match target_address.clone().to_socket_addrs() {
                            Err(e) => {
                                error!("Connection [{}] fail to convert addresses because of error : {:#?}", connection_id, e);
                                return;
                            },
                            Ok(v) => v,
                        };
                        if let Err(e) = udp_socket.connect(udp_target_addresses.collect::<Vec<_>>().as_slice()).await {
                            error!(
                                "Connection [{}] fail to connect target address [{:?}] because of error : {:#?}",
                                connection_id, target_address, e
                            );
                            return;
                        };
                        debug!(
                            "Connection [{}] begin to send udp data from agent to target:\n{}\n",
                            connection_id,
                            pretty_hex::pretty_hex(&data)
                        );
                        if let Err(e) = udp_socket.send(&data).await {
                            error!("Connection [{}] fail to send udp packet to target because of error:{:#?}", connection_id, e);
                            return;
                        };
                        let mut receive_buffer = [0u8; SIZE_64KB];
                        let received_data_size = match udp_socket.recv(&mut receive_buffer).await {
                            Err(e) => {
                                error!(
                                    "Connection [{}] fail to receive udp packet from target because of error:{:#?}",
                                    connection_id, e
                                );
                                return;
                            },
                            Ok(v) => v,
                        };
                        let received_data = &receive_buffer[0..received_data_size];
                        debug!(
                            "Connection [{}] receive udp data from target:\n\n{}\n\n",
                            connection_id,
                            pretty_hex(&received_data)
                        );
                        let PayloadEncryptionTypeSelectResult {
                            user_token,
                            payload_encryption_type,
                            ..
                        } = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token.as_str(),
                        })
                        .await
                        {
                            Err(e) => {
                                error!("Connection [{}] fail to select payload encryption because of error:{:#?}", connection_id, e);
                                return;
                            },
                            Ok(v) => v,
                        };

                        message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                            connection_id: Some(connection_id.as_str()),
                            message_framed_write,
                            message_payloads: Some(vec![MessagePayload {
                                data: Some(Bytes::copy_from_slice(received_data)),
                                payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpData),
                                source_address,
                                target_address: Some(target_address),
                            }]),
                            payload_encryption_type,
                            ref_id: Some(message_id.as_str()),
                            user_token: user_token.as_str(),
                        })
                        .await
                        {
                            Err(WriteMessageFramedError { message_framed_write, source }) => {
                                error!("Connection [{}] fail to write udp data because of error:{:#?}", connection_id, source);
                                message_framed_write
                            },
                            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                        };
                        message_framed_read = message_framed_read_return_back;
                    },
                    Ok(ReadMessageFramedResult { .. }) => {
                        error!(
                            "Connection [{}] has a invalid payload when read from agent, message id:{}, user token:{}.",
                            connection_id, message_id, user_token
                        );
                        return;
                    },
                };
            }
        });
        Ok(UdpRelayFlowResult)
    }
}
