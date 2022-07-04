use std::{net::SocketAddr, sync::Arc};

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use common::{
    generate_uuid, MessageFramedRead, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest,
    PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use futures::SinkExt;
use tokio::net::TcpStream;
use tracing::{info, instrument};

use crate::config::ProxyConfig;

#[allow(unused)]
pub(crate) struct UdpAssociateFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub source_address: NetAddress,
    pub agent_address: SocketAddr,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
}

#[allow(unused)]
pub(crate) struct UdpAssociateFlowResult<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub source_address: NetAddress,
}

pub(crate) struct UdpAssociateFlow;

impl UdpAssociateFlow {
    #[instrument(skip_all, fields(request.connection_id))]
    pub async fn exec<T>(request: UdpAssociateFlowRequest<T>, _configuration: Arc<ProxyConfig>) -> Result<UdpAssociateFlowResult<T>>
    where
        T: RsaCryptoFetcher,
    {
        let UdpAssociateFlowRequest {
            connection_id,
            message_id,
            user_token,
            message_framed_read,
            message_framed_write,
            source_address,
            ..
        } = request;
        info!("Connection [{}] associate udp success.", connection_id);

        let PayloadEncryptionTypeSelectResult { payload_encryption_type, .. } = PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: user_token.clone(),
        })
        .await?;

        let udp_associate_success_payload = MessagePayload {
            source_address: Some(source_address.clone()),
            target_address: None,
            payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpAssociateSuccess),
            data: Bytes::new(),
        };
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            message_framed_write,
            message_payloads: Some(vec![udp_associate_success_payload]),
            payload_encryption_type,
            user_token: user_token.as_str(),
            ref_id: Some(message_id.as_str()),
            connection_id: Some(connection_id.as_str()),
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
        Ok(UdpAssociateFlowResult {
            connection_id,
            message_framed_read,
            message_framed_write,
            message_id,
            user_token,
            source_address,
        })
    }
}
