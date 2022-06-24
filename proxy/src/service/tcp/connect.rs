use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use common::{
    generate_uuid, MessageFramedRead, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest,
    PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue, RsaCryptoFetcher, TcpConnectRequest,
    TcpConnectResult, TcpConnector, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};
use futures::SinkExt;
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpStream;
use tracing::{debug, error, instrument};

use crate::config::{ProxyConfig, DEFAULT_TARGET_STREAM_SO_LINGER};

pub(crate) struct TcpConnectFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub agent_address: SocketAddr,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
}

pub(crate) struct TcpConnectFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
    pub message_id: String,
}

pub(crate) struct TcpConnectFlow;

impl TcpConnectFlow {
    #[instrument(skip_all, fields(request.connection_id))]
    pub async fn exec<T>(request: TcpConnectFlowRequest<T>, configuration: Arc<ProxyConfig>) -> Result<TcpConnectFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let target_stream_so_linger = configuration.target_stream_so_linger().unwrap_or(DEFAULT_TARGET_STREAM_SO_LINGER);
        let TcpConnectFlowRequest {
            connection_id,
            message_id,
            user_token,
            source_address,
            target_address,
            message_framed_write,
            message_framed_read,
            agent_address,
            ..
        } = request;
        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: user_token.clone(),
        })
        .await?;
        let connect_to_target_result = TcpConnector::connect(TcpConnectRequest {
            connect_addresses: target_address.clone().try_into()?,
            connected_stream_so_linger: target_stream_so_linger,
        })
        .await;
        let TcpConnectResult {
            connected_stream: target_stream,
        } = match connect_to_target_result {
            Err(e) => {
                error!(
                    "Connection [{}] fail connect to target {:#?} because of error: {:#?}",
                    connection_id, target_address, e
                );
                let connect_fail_payload = MessagePayload {
                    source_address: Some(source_address),
                    target_address: Some(target_address),
                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                    data: Bytes::new(),
                };
                match MessageFramedWriter::write(WriteMessageFramedRequest {
                    message_framed_write,
                    message_payload: Some(connect_fail_payload),
                    payload_encryption_type,
                    user_token: user_token.clone(),
                    ref_id: Some(message_id),
                    connection_id: Some(connection_id),
                })
                .await
                {
                    Err(WriteMessageFramedError { source, .. }) => {
                        return Err(anyhow!(source));
                    },
                    Ok(WriteMessageFramedResult { mut message_framed_write }) => {
                        if let Err(e) = message_framed_write.flush().await {
                            return Err(anyhow!(e));
                        }
                    },
                };
                return Err(anyhow!(e));
            },
            Ok(v) => v,
        };
        debug!(
            "Connection [{}] agent address: {}, success connect to target {:#?}",
            connection_id, agent_address, target_address
        );

        let connect_success_payload = MessagePayload {
            source_address: Some(source_address.clone()),
            target_address: Some(target_address.clone()),
            payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess),
            data: Bytes::new(),
        };
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            message_framed_write,
            message_payload: Some(connect_success_payload),
            payload_encryption_type,
            user_token: user_token.clone(),
            ref_id: Some(message_id.clone()),
            connection_id: Some(connection_id),
        })
        .await
        {
            Err(WriteMessageFramedError { source, .. }) => return Err(anyhow!(source)),
            Ok(WriteMessageFramedResult { mut message_framed_write }) => {
                if let Err(e) = message_framed_write.flush().await {
                    return Err(anyhow!(e));
                }
                message_framed_write
            },
        };
        Ok(TcpConnectFlowResult {
            target_stream,
            message_framed_read,
            message_framed_write,
            source_address,
            target_address,
            user_token,
            message_id,
        })
    }
}
