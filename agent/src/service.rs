use ::common::{CommonError, RsaCrypto, RsaCryptoFetcher};
use std::fs;
use tracing::error;

pub(crate) mod common;
pub(crate) mod http;
pub(crate) mod socks5;

pub struct AgentRsaCryptoFetcher {
    rsa_crypto: RsaCrypto,
}

impl AgentRsaCryptoFetcher {
    pub fn new() -> Result<Self, CommonError> {
        let public_key = match fs::read_to_string("ProxyPublicKey.pem") {
            Err(e) => {
                error!("Fail to read AgentPublicKey.pem because of error: {:#?}", e);
                return Err(CommonError::IoError { source: e });
            },
            Ok(v) => v,
        };
        let private_key = match fs::read_to_string("AgentPrivateKey.pem") {
            Err(e) => {
                error!(
                    "Fail to read ProxyPrivateKey.pem because of error: {:#?}",
                    e
                );
                return Err(CommonError::IoError { source: e });
            },
            Ok(v) => v,
        };
        let rsa_crypto = match RsaCrypto::new(public_key, private_key) {
            Err(e) => {
                error!("Fail to create rsa crypto because of error: {:#?}", e);
                return Err(e);
            },
            Ok(v) => v,
        };
        Ok(Self { rsa_crypto })
    }
}
impl RsaCryptoFetcher for AgentRsaCryptoFetcher {
    fn fetch(&self, _user_token: &str) -> Result<Option<&RsaCrypto>, CommonError> {
        Ok(Some(&self.rsa_crypto))
    }
}
