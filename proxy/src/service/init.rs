use anyhow::anyhow;
use anyhow::Result;

use common::{
    AgentMessagePayloadTypeValue, MessageFramedRead, MessageFramedReader, MessageFramedWrite, MessagePayload, NetAddress, PayloadType, PpaassError,
    ReadMessageFramedRequest, ReadMessageFramedResult, ReadMessageFramedResultContent, RsaCryptoFetcher,
};

use std::net::SocketAddr;

use std::sync::Arc;
use tokio::net::TcpStream;

use tracing::debug;
use tracing::error;

use crate::config::ProxyConfig;

use super::{
    tcp::connect::{TcpConnectFlow, TcpConnectFlowRequest, TcpConnectFlowResult},
    udp::associate::{UdpAssociateFlow, UdpAssociateFlowRequest, UdpAssociateFlowResult},
};

pub(crate) enum RelayType {
    Tcp,
    Udp,
}
pub(crate) struct InitFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub agent_address: SocketAddr,
}

pub(crate) struct InitFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
    pub relay_type: RelayType,
}

#[derive(Clone, Default)]
pub(crate) struct InitializeFlow;

impl InitializeFlow {
    pub async fn exec<T>(request: InitFlowRequest<T>, configuration: Arc<ProxyConfig>) -> Result<InitFlowResult<T>>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let InitFlowRequest {
            connection_id,
            message_framed_read,
            message_framed_write,
            agent_address,
        } = request;
        let read_agent_message_result = MessageFramedReader::read(ReadMessageFramedRequest {
            connection_id: connection_id.clone(),
            message_framed_read,
        })
        .await;
        match read_agent_message_result {
            Ok(ReadMessageFramedResult {
                message_framed_read,
                content:
                    Some(ReadMessageFramedResultContent {
                        message_id,
                        user_token,
                        message_payload:
                            Some(MessagePayload {
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect),
                                target_address,
                                source_address,
                                ..
                            }),
                        ..
                    }),
                ..
            }) => {
                let TcpConnectFlowResult {
                    target_stream,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    target_address,
                    user_token,
                    message_id,
                    ..
                } = TcpConnectFlow::exec(
                    TcpConnectFlowRequest {
                        connection_id,
                        message_id,
                        message_framed_read,
                        message_framed_write,
                        agent_address,
                        source_address,
                        target_address,
                        user_token,
                    },
                    configuration,
                )
                .await?;
                return Ok(InitFlowResult {
                    message_framed_write,
                    message_framed_read,
                    target_stream,
                    message_id,
                    source_address,
                    target_address,
                    user_token,
                    relay_type: RelayType::Tcp,
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
                                payload_type: PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate),
                                target_address,
                                source_address,
                                ..
                            }),
                        ..
                    }),
                ..
            }) => {
                let UdpAssociateFlowResult { .. } = UdpAssociateFlow::exec(UdpAssociateFlowRequest {}, configuration).await?;
                todo!()
            },
            _ => Err(anyhow!(PpaassError::CodecError)),
        }
    }
}
