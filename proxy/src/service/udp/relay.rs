use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs},
    sync::Arc,
};

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use common::{
    generate_uuid, Message, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, ReadMessageFramedError, ReadMessageFramedRequest,
    ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use futures::StreamExt;
use tokio::net::UdpSocket;
use tracing::{debug, error};

use crate::config::ProxyConfig;

const SIZE_64KB: usize = 65536;
#[allow(unused)]
pub(crate) struct UdpRelayFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
}
pub(crate) struct UdpRelayFlowResult;
pub(crate) struct UdpRelayFlow;

impl UdpRelayFlow {
    pub async fn exec<T>(request: UdpRelayFlowRequest<T>, rsa_crypto_fetcher: Arc<T>, configuration: Arc<ProxyConfig>) -> Result<UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let UdpRelayFlowRequest {
            connection_id,
            message_id,
            user_token,
            mut message_framed_read,
            mut message_framed_write,
            ..
        } = request;
        tokio::spawn(async move {
            loop {
                match MessageFramedReader::read(ReadMessageFramedRequest {
                    connection_id: connection_id.clone(),
                    message_framed_read,
                })
                .await
                {
                    Err(ReadMessageFramedError {
                        message_framed_read: message_framed_read_return_back,
                        source,
                    }) => {
                        error!("Connection [{}] has a error when read from agent, error: {:#?}.", connection_id, source);
                        message_framed_read = message_framed_read_return_back;
                        continue;
                    },
                    Ok(ReadMessageFramedResult {
                        message_framed_read: message_framed_read_return_back,
                        content: None,
                    }) => {
                        debug!(
                            "Connection [{}] nothing to read, message id:{}, user token:{}",
                            connection_id, message_id, user_token
                        );
                        message_framed_read = message_framed_read_return_back;
                        continue;
                    },
                    Ok(ReadMessageFramedResult {
                        message_framed_read: message_framed_read_return_back,
                        content:
                            Some(ReadMessageFramedResultContent {
                                message_id,
                                message_payload: None,
                                user_token,
                            }),
                    }) => {
                        error!(
                            "Connection [{}] has a invalid payload when read from agent, message id:{}, user token:{}.",
                            connection_id, message_id, user_token
                        );
                        message_framed_read = message_framed_read_return_back;
                        continue;
                    },
                    Ok(ReadMessageFramedResult {
                        message_framed_read: message_framed_read_return_back,
                        content:
                            Some(ReadMessageFramedResultContent {
                                message_id,
                                message_payload:
                                    Some(MessagePayload {
                                        source_address,
                                        target_address,
                                        payload_type,
                                        data,
                                    }),
                                user_token,
                            }),
                    }) => {
                        let udp_socket = match UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0))).await {
                            Err(e) => {
                                error!("Connection [{}] fail to create udp socket because of error : {:#?}", connection_id, e);
                                message_framed_read = message_framed_read_return_back;
                                continue;
                            },
                            Ok(v) => v,
                        };
                        let udp_target_addresses = match target_address.clone().to_socket_addrs() {
                            Err(e) => {
                                error!("Connection [{}] fail to convert addresses because of error : {:#?}", connection_id, e);
                                message_framed_read = message_framed_read_return_back;
                                continue;
                            },
                            Ok(v) => v,
                        };
                        if let Err(e) = udp_socket.connect(udp_target_addresses.collect::<Vec<_>>().as_slice()).await {
                            error!(
                                "Connection [{}] fail to connect target address [{:?}] because of error : {:#?}",
                                connection_id, target_address, e
                            );
                            message_framed_read = message_framed_read_return_back;
                            continue;
                        };
                        if let Err(e) = udp_socket.send(&data).await {
                            message_framed_read = message_framed_read_return_back;
                            continue;
                        };
                        let mut receive_buffer = [0u8; SIZE_64KB];
                        let received_data_size = match udp_socket.recv(&mut receive_buffer).await {
                            Err(e) => {
                                message_framed_read = message_framed_read_return_back;
                                continue;
                            },
                            Ok(v) => v,
                        };
                        let received_data = &receive_buffer[0..received_data_size];

                        let PayloadEncryptionTypeSelectResult {
                            user_token,
                            encryption_token,
                            payload_encryption_type,
                        } = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                            encryption_token: generate_uuid().into(),
                            user_token: user_token.clone(),
                        })
                        .await
                        {
                            Err(e) => {
                                message_framed_read = message_framed_read_return_back;
                                continue;
                            },
                            Ok(v) => v,
                        };

                        message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                            connection_id: Some(connection_id.clone()),
                            message_framed_write,
                            message_payload: Some(MessagePayload {
                                data: Bytes::copy_from_slice(received_data),
                                payload_type,
                                source_address,
                                target_address,
                            }),
                            payload_encryption_type,
                            ref_id: Some(message_id),
                            user_token,
                        })
                        .await
                        {
                            Err(WriteMessageFramedError { message_framed_write, source }) => message_framed_write,
                            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                        };

                        message_framed_read = message_framed_read_return_back;
                    },
                };
            }
        });
        Ok(UdpRelayFlowResult)
    }
}
