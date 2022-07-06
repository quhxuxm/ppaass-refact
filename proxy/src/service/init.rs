use anyhow::anyhow;
use anyhow::Result;

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress,
    PayloadEncryptionTypeSelectRequest, PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue,
    ReadMessageFramedError, ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};
use tokio_util::codec::FramedParts;

use std::{fmt::Debug, net::SocketAddr};

use tokio::{io::AsyncWriteExt, net::TcpStream};

use tracing::{debug, error, instrument};

use crate::{
    config::ProxyConfig,
    service::{tcp::connect::TcpConnectFlowError, udp::associate::UdpAssociateFlowError},
};

use super::{
    tcp::connect::{TcpConnectFlow, TcpConnectFlowRequest, TcpConnectFlowResult},
    udp::associate::{UdpAssociateFlow, UdpAssociateFlowRequest, UdpAssociateFlowResult},
};

const DEFAULT_AGENT_CONNECTION_READ_TIMEOUT: u64 = 1200;

#[derive(Debug)]
pub(crate) struct InitFlowRequest<'a, T, TcpStream>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub agent_address: SocketAddr,
}

#[allow(unused)]
pub(crate) enum InitFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    Heartbeat {
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
    },
    Tcp {
        target_stream: TcpStream,
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
        message_id: String,
        source_address: NetAddress,
        target_address: NetAddress,
        user_token: String,
    },
    Udp {
        message_framed_read: MessageFramedRead<T, TcpStream>,
        message_framed_write: MessageFramedWrite<T, TcpStream>,
        message_id: String,
        source_address: NetAddress,
        user_token: String,
    },
}

#[derive(Clone, Default)]
pub(crate) struct InitializeFlow;

impl InitializeFlow {
    async fn shutdown_connection<T>(
        connection_id: &str, message_framed_read: MessageFramedRead<T, TcpStream>, message_framed_write: MessageFramedWrite<T, TcpStream>,
    ) -> Result<()>
    where
        T: RsaCryptoFetcher,
    {
        match message_framed_read.reunite(message_framed_write) {
            Err(e) => {
                return Err(anyhow!(
                    "Connection [{connection_id}] handle agent connection fail and also fail to recover because of error: {e}."
                ))
            },
            Ok(v) => {
                let FramedParts {
                    mut io,
                    mut read_buf,
                    mut write_buf,
                    ..
                } = v.into_parts();
                read_buf.clear();
                write_buf.clear();
                io.shutdown().await?;
            },
        };
        Ok(())
    }
    #[instrument(skip_all, fields(request.connection_id))]
    pub async fn exec<'a, T>(request: InitFlowRequest<'a, T, TcpStream>, configuration: &ProxyConfig) -> Result<InitFlowResult<T>>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let InitFlowRequest {
            connection_id,
            message_framed_read,
            message_framed_write,
            agent_address,
        } = request;
        let read_timeout = configuration.agent_connection_read_timeout().unwrap_or(DEFAULT_AGENT_CONNECTION_READ_TIMEOUT);
        let read_agent_message_result = MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: connection_id.clone(),
            message_framed_read,
            timeout: Some(read_timeout),
        })
        .await;
        match read_agent_message_result {
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        user_token,
                        message_id,
                        message_payload:
                            Some(MessagePayload {
                                source_address,
                                target_address,
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat),
                                ..
                            }),
                        ..
                    }),
                ..
            }) => {
                let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } =
                    PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
                        encryption_token: generate_uuid().into(),
                        user_token: user_token.as_str(),
                    })
                    .await?;
                let heartbeat_success = MessagePayload {
                    source_address: None,
                    target_address: None,
                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::HeartbeatSuccess),
                    data: None,
                };
                let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
                    message_framed_write,
                    message_payloads: Some(vec![heartbeat_success]),
                    payload_encryption_type,
                    user_token: user_token.as_str(),
                    ref_id: Some(message_id.as_str()),
                    connection_id: Some(connection_id),
                })
                .await
                {
                    Err(WriteMessageFramedError { source, .. }) => {
                        error!("Connection [{}] fail to write heartbeat success to agent because of error, source address: {:?}, target address: {:?}, client address: {:?}", connection_id, source_address, target_address, agent_address);
                        return Err(anyhow!(source));
                    },
                    Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
                };
                return Ok(InitFlowResult::Heartbeat {
                    message_framed_write,
                    message_framed_read,
                });
            },
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        user_token,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                                target_address: Some(target_address),
                                source_address: Some(source_address),
                                ..
                            }),
                        ..
                    }),
                ..
            }) => {
                debug!(
                    "Connection [{}] begin tcp connect, source address: {:?}, target address: {:?}, client address: {:?}",
                    connection_id, source_address, target_address, agent_address
                );
                match TcpConnectFlow::exec(
                    TcpConnectFlowRequest {
                        connection_id,
                        message_id: message_id.as_str(),
                        message_framed_read,
                        message_framed_write,
                        agent_address,
                        source_address,
                        target_address,
                        user_token: user_token.as_str(),
                    },
                    configuration,
                )
                .await
                {
                    Err(TcpConnectFlowError {
                        connection_id,
                        message_framed_read,
                        message_framed_write,
                        source,
                        ..
                    }) => {
                        error!("Connection [{connection_id}] handle agent connection fail to do tcp connect because of error: {source:#?}.");
                        Self::shutdown_connection(connection_id.as_str(), message_framed_read, message_framed_write).await?;
                        Err(anyhow!(
                            "Connection [{connection_id}] handle agent connection fail to do tcp connect because of error: {source:#?}."
                        ))
                    },
                    Ok(TcpConnectFlowResult {
                        target_stream,
                        message_framed_read,
                        message_framed_write,
                        source_address,
                        target_address,
                        user_token,
                        message_id,
                        ..
                    }) => {
                        debug!(
                            "Connection [{}] complete tcp connect, source address: {:?}, target address: {:?}, client address: {:?}",
                            connection_id, source_address, target_address, agent_address
                        );
                        Ok(InitFlowResult::Tcp {
                            message_framed_write,
                            message_framed_read,
                            target_stream,
                            message_id,
                            source_address,
                            target_address,
                            user_token,
                        })
                    },
                }
            },
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        user_token,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate),
                                target_address: None,
                                source_address: Some(source_address),
                                ..
                            }),
                        ..
                    }),
                ..
            }) => {
                debug!("Connection [{}] begin udp associate, client address: {:?}", connection_id, source_address);
                match UdpAssociateFlow::exec(
                    UdpAssociateFlowRequest {
                        message_framed_read,
                        message_framed_write,
                        agent_address,
                        connection_id,
                        message_id: message_id.as_str(),
                        source_address,
                        user_token: user_token.as_str(),
                    },
                    configuration,
                )
                .await
                {
                    Err(UdpAssociateFlowError {
                        connection_id,
                        message_framed_read,
                        message_framed_write,
                        source,
                        ..
                    }) => {
                        error!("Connection [{connection_id}] handle agent connection fail to do udp associate because of error: {source:#?}.");
                        Self::shutdown_connection(connection_id.as_str(), message_framed_read, message_framed_write).await?;
                        Err(anyhow!(
                            "Connection [{connection_id}] handle agent connection fail to do udp associate because of error: {source:#?}."
                        ))
                    },
                    Ok(UdpAssociateFlowResult {
                        connection_id,
                        message_id,
                        user_token,
                        message_framed_read,
                        message_framed_write,
                        source_address,
                    }) => {
                        debug!("Connection [{}] complete udp associate, client address: {:?}", connection_id, source_address);
                        Ok(InitFlowResult::Udp {
                            message_framed_write,
                            message_framed_read,
                            message_id,
                            source_address,
                            user_token,
                        })
                    },
                }
            },
            Ok(ReadMessageFramedResult { message_framed_read, .. }) => {
                error!("Connection [{connection_id}] handle agent connection fail because of invalid message content.");
                Self::shutdown_connection(connection_id, message_framed_read, message_framed_write).await?;
                Err(anyhow!(
                    "Connection [{connection_id}] handle agent connection fail because of invalid message content."
                ))
            },
            Err(ReadMessageFramedError { message_framed_read, source }) => {
                error!("Connection [{connection_id}] handle agent connection fail because of error: {source}.");
                Self::shutdown_connection(connection_id, message_framed_read, message_framed_write).await?;
                Err(anyhow!("Connection [{connection_id}] handle agent connection fail because of error: {source}."))
            },
        }
    }
}
