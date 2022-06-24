use std::{fs, sync::Arc};

use ::common::{PpaassError, RsaCrypto, RsaCryptoFetcher};

use anyhow::Result;
use tracing::instrument;

use crate::config::AgentConfig;
pub(crate) mod common;
pub(crate) mod http;
pub(crate) mod pool;
pub(crate) mod socks5;

#[derive(Debug)]
pub struct AgentRsaCryptoFetcher {
    rsa_crypto: RsaCrypto,
}

impl AgentRsaCryptoFetcher {
    pub fn new(configuration: Arc<AgentConfig>) -> Result<Self> {
        let public_key = fs::read_to_string(configuration.proxy_public_key_file().as_ref().unwrap_or(&"ProxyPublicKey.pem".to_string()))?;
        let private_key = fs::read_to_string(configuration.agent_private_key_file().as_ref().unwrap_or(&"AgentPrivateKey.pem".to_string()))?;
        let rsa_crypto = RsaCrypto::new(public_key, private_key)?;
        Ok(Self { rsa_crypto })
    }
}
impl RsaCryptoFetcher for AgentRsaCryptoFetcher {
    #[instrument(skip_all, fields(_user_token))]
    fn fetch<Q>(&self, _user_token: Q) -> Result<Option<&RsaCrypto>, PpaassError>
    where
        Q: AsRef<str>,
    {
        Ok(Some(&self.rsa_crypto))
    }
}
