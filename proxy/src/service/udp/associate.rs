use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Result;
use common::RsaCryptoFetcher;

use crate::config::ProxyConfig;

pub(crate) struct UdpAssociateFlowRequest;
pub(crate) struct UdpAssociateFlowResult;
pub(crate) struct UdpAssociateFlow;

impl UdpAssociateFlow {
    pub async fn exec<T>(request: UdpAssociateFlowRequest, configuration: Arc<ProxyConfig>) -> Result<UdpAssociateFlowResult>
    where
        T: RsaCryptoFetcher,
    {
        Ok(UdpAssociateFlowResult)
    }
}
