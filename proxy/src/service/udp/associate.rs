use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Result;
use common::{MessageFramedRead, MessageFramedWrite, RsaCryptoFetcher};

use crate::config::ProxyConfig;

pub(crate) struct UdpAssociateFlowRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
}
pub(crate) struct UdpAssociateFlowResult;
pub(crate) struct UdpAssociateFlow;

impl UdpAssociateFlow {
    pub async fn exec<T>(request: UdpAssociateFlowRequest<T>, configuration: Arc<ProxyConfig>) -> Result<UdpAssociateFlowResult>
    where
        T: RsaCryptoFetcher,
    {
        Ok(UdpAssociateFlowResult)
    }
}
