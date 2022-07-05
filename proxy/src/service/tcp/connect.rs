use anyhow::anyhow;
use anyhow::Result;

use common::{
    generate_uuid, MessageFramedRead, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest,
    PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue, RsaCryptoFetcher, TcpConnectRequest,
    TcpConnectResult, TcpConnector, WriteMessageFramedError, WriteMessageFramedRequest, WriteMessageFramedResult,
};

use std::net::SocketAddr;
use tokio::net::TcpStream;
use tracing::{debug, error, instrument};

use crate::config::{ProxyConfig, DEFAULT_TARGET_STREAM_SO_LINGER};

pub(crate) struct TcpConnectFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_id: &'a str,
    pub user_token: &'a str,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub agent_address: SocketAddr,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
}

pub(crate) struct TcpConnectFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub target_stream: TcpStream,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub user_token: String,
    pub message_id: String,
}

pub(crate) struct TcpConnectFlow;

impl TcpConnectFlow {
    #[instrument(skip_all, fields(request.connection_id))]
    pub async fn exec<'a, T>(request: TcpConnectFlowRequest<'a, T>, configuration: &ProxyConfig) -> Result<TcpConnectFlowResult<T>>
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
            user_token,
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
                let error_message = format!("Connection [{connection_id}] fail connect to target {target_address:#?} because of error: {e:#?}");
                error!("{error_message}");
                let connect_fail_payload = MessagePayload {
                    source_address: Some(source_address),
                    target_address: Some(target_address),
                    payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail),
                    data: None,
                };
                if let Err(WriteMessageFramedError { source, .. }) = MessageFramedWriter::write(WriteMessageFramedRequest {
                    message_framed_write,
                    message_payloads: Some(vec![connect_fail_payload]),
                    payload_encryption_type,
                    user_token,
                    ref_id: Some(message_id),
                    connection_id: Some(connection_id),
                })
                .await
                {
                    error!("Connection [{connection_id}] fail to write connect fail result to agent because of error: {source:#?}");
                    return Err(anyhow!(source));
                };
                return Err(anyhow!(error_message));
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
            data: None,
        };
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            message_framed_write,
            message_payloads: Some(vec![connect_success_payload]),
            payload_encryption_type,
            user_token,
            ref_id: Some(message_id),
            connection_id: Some(connection_id),
        })
        .await
        {
            Err(WriteMessageFramedError { source, .. }) => return Err(anyhow!(source)),
            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
        };
        Ok(TcpConnectFlowResult {
            target_stream,
            message_framed_read,
            message_framed_write,
            source_address,
            target_address,
            user_token: user_token.to_owned(),
            message_id: message_id.to_owned(),
        })
    }
}
