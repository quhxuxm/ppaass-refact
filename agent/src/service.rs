use std::fs;

use ::common::{PpaassError, RsaCrypto, RsaCryptoFetcher};

use anyhow::Result;
pub(crate) mod common;
pub(crate) mod http;
pub(crate) mod socks5;

pub struct AgentRsaCryptoFetcher {
    rsa_crypto: RsaCrypto,
}

impl AgentRsaCryptoFetcher {
    pub fn new() -> Result<Self> {
        let public_key = fs::read_to_string("ProxyPublicKey.pem")?;
        let private_key = fs::read_to_string("AgentPrivateKey.pem")?;
        let rsa_crypto = RsaCrypto::new(public_key, private_key)?;
        Ok(Self { rsa_crypto })
    }
}
impl RsaCryptoFetcher for AgentRsaCryptoFetcher {
    fn fetch<Q>(&self, _user_token: Q) -> Result<Option<&RsaCrypto>, PpaassError>
    where
        Q: AsRef<str>,
    {
        Ok(Some(&self.rsa_crypto))
    }
}
