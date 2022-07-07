use std::net::SocketAddr;

use anyhow::anyhow;
use anyhow::Result;

use common::{
    generate_uuid, MessageFramedRead, MessageFramedWrite, MessageFramedWriter, MessagePayload, NetAddress, PayloadEncryptionTypeSelectRequest,
    PayloadEncryptionTypeSelectResult, PayloadEncryptionTypeSelector, PayloadType, ProxyMessagePayloadTypeValue, RsaCryptoFetcher, WriteMessageFramedError,
    WriteMessageFramedRequest, WriteMessageFramedResult,
};

use tokio::net::TcpStream;
use tracing::info;

use crate::config::ProxyConfig;

#[allow(unused)]
pub(crate) struct UdpAssociateFlowRequest<'a, T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: &'a str,
    pub message_id: &'a str,
    pub user_token: &'a str,
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

#[allow(unused)]
pub(crate) struct UdpAssociateFlowError<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_id: String,
    pub user_token: String,
    pub message_framed_read: MessageFramedRead<T, TcpStream>,
    pub message_framed_write: MessageFramedWrite<T, TcpStream>,
    pub source_address: NetAddress,
    pub source: anyhow::Error,
}

pub(crate) struct UdpAssociateFlow;

impl UdpAssociateFlow {
    pub async fn exec<'a, T>(
        UdpAssociateFlowRequest {
            connection_id,
            message_id,
            user_token,
            message_framed_read,
            message_framed_write,
            source_address,
            ..
        }: UdpAssociateFlowRequest<'a, T>,
        _configuration: &ProxyConfig,
    ) -> Result<UdpAssociateFlowResult<T>, UdpAssociateFlowError<T>>
    where
        T: RsaCryptoFetcher,
    {
        info!("Connection [{}] associate udp success.", connection_id);
        let payload_encryption_type = match PayloadEncryptionTypeSelector::select(PayloadEncryptionTypeSelectRequest {
            encryption_token: generate_uuid().into(),
            user_token: user_token.clone(),
        })
        .await
        {
            Err(e) => {
                return Err(UdpAssociateFlowError {
                    connection_id: connection_id.to_owned(),
                    message_id: message_id.to_owned(),
                    user_token: user_token.to_owned(),
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    source: anyhow!(e),
                })
            },
            Ok(PayloadEncryptionTypeSelectResult { payload_encryption_type, .. }) => payload_encryption_type,
        };

        let udp_associate_success_payload = MessagePayload {
            source_address: Some(source_address.clone()),
            target_address: None,
            payload_type: PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpAssociateSuccess),
            data: None,
        };
        let message_framed_write = match MessageFramedWriter::write(WriteMessageFramedRequest {
            message_framed_write,
            message_payloads: Some(vec![udp_associate_success_payload]),
            payload_encryption_type,
            user_token,
            ref_id: Some(message_id),
            connection_id: Some(connection_id),
        })
        .await
        {
            Err(WriteMessageFramedError {
                source, message_framed_write, ..
            }) => {
                return Err(UdpAssociateFlowError {
                    connection_id: connection_id.to_owned(),
                    message_id: message_id.to_owned(),
                    user_token: user_token.to_owned(),
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    source: anyhow!(source),
                })
            },
            Ok(WriteMessageFramedResult { message_framed_write }) => message_framed_write,
        };
        Ok(UdpAssociateFlowResult {
            connection_id: connection_id.to_string(),
            message_framed_read,
            message_framed_write,
            message_id: message_id.to_string(),
            user_token: user_token.to_string(),
            source_address,
        })
    }
}
