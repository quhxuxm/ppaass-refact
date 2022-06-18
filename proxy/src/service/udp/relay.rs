use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Result;
use common::RsaCryptoFetcher;

use crate::config::ProxyConfig;

pub(crate) struct UdpRelayFlowRequest;
pub(crate) struct UdpRelayFlowResult;
pub(crate) struct UdpRelayFlow;

impl UdpRelayFlow {
    pub async fn exec<T>(request: UdpRelayFlowRequest, rsa_crypto_fetcher: Arc<T>, configuration: Arc<ProxyConfig>) -> Result<UdpRelayFlowResult>
    where
        T: RsaCryptoFetcher,
    {
        Ok(UdpRelayFlowResult)
    }
}
