use ::common::{CommonError, RsaCrypto, RsaCryptoFetcher};

pub(crate) mod common;
pub(crate) mod http;
pub(crate) mod socks5;

pub struct AgentRsaCryptoFetcher;

impl RsaCryptoFetcher for AgentRsaCryptoFetcher {
    fn fetch(&self, user_token: &str) -> Result<Option<&RsaCrypto>, CommonError> {
        todo!()
    }
}
