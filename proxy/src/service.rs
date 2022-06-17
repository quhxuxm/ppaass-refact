use std::{collections::HashMap, net::SocketAddr};
use std::{fmt::Debug, sync::Arc};
use std::{fs, path::Path};

use tokio::net::TcpStream;

use common::{generate_uuid, MessageFramedGenerator, PpaassError, RsaCrypto, RsaCryptoFetcher};

use tracing::{debug, error};

use crate::service::tcp::relay::{TcpRelayProcess, TcpRelayRequest};
use crate::{
    config::ProxyConfig,
    service::tcp::connect::{TcpConnectProcess, TcpConnectProcessRequest},
};

use anyhow::Result;
mod tcp;

const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;

pub(crate) struct ProxyRsaCryptoFetcher {
    cache: HashMap<String, RsaCrypto>,
}

impl ProxyRsaCryptoFetcher {
    pub fn new(configuration: Arc<ProxyConfig>) -> Result<Self> {
        let mut result = Self { cache: HashMap::new() };
        let rsa_dir_path = configuration.rsa_root_dir().as_ref().expect("Fail to read rsa root directory.");
        let rsa_dir = fs::read_dir(rsa_dir_path)?;
        rsa_dir.for_each(|entry| {
            let entry = match entry {
                Err(e) => {
                    error!("Fail to read {} directory because of error: {:#?}", rsa_dir_path, e);
                    return;
                },
                Ok(v) => v,
            };
            let user_token = entry.file_name();
            let user_token = user_token.to_str();
            let user_token = match user_token {
                None => {
                    error!("Fail to read {}{:?} directory because of user token not exist", rsa_dir_path, entry.file_name());
                    return;
                },
                Some(v) => v,
            };
            let public_key = match fs::read_to_string(Path::new(format!("{}{}/AgentPublicKey.pem", rsa_dir_path, user_token).as_str())) {
                Err(e) => {
                    error!("Fail to read {}{}/AgentPublicKey.pem because of error: {:#?}", rsa_dir_path, user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            let private_key = match fs::read_to_string(Path::new(format!("{}{}/ProxyPrivateKey.pem", rsa_dir_path, user_token).as_str())) {
                Err(e) => {
                    error!("Fail to read {}{}/ProxyPrivateKey.pem because of error: {:#?}", rsa_dir_path, user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            let rsa_crypto = match RsaCrypto::new(public_key, private_key) {
                Err(e) => {
                    error!("Fail to create rsa crypto for user: {} because of error: {:#?}", user_token, e);
                    return;
                },
                Ok(v) => v,
            };
            result.cache.insert(user_token.to_string(), rsa_crypto);
        });
        Ok(result)
    }
}

impl RsaCryptoFetcher for ProxyRsaCryptoFetcher {
    fn fetch<Q>(&self, user_token: Q) -> Result<Option<&RsaCrypto>, PpaassError>
    where
        Q: AsRef<str>,
    {
        Ok(self.cache.get(user_token.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct AgentConnection {
    id: String,
    agent_stream: TcpStream,
    agent_address: SocketAddr,
}
impl AgentConnection {
    pub fn new(agent_stream: TcpStream, agent_address: SocketAddr) -> Self {
        Self {
            id: generate_uuid(),
            agent_stream,
            agent_address,
        }
    }
    pub fn get_id(&self) -> &str {
        self.id.as_str()
    }
    pub async fn exec<T>(self, rsa_crypto_fetcher: Arc<T>, configuration: Arc<ProxyConfig>) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + 'static,
    {
        let connection_id = self.id.clone();
        debug!("Begin to handle agent connection: {}", connection_id);

        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);
        let agent_stream = self.agent_stream;
        let agent_address_clone = self.agent_address.clone();
        let agent_address = self.agent_address;
        let tcp_connect_process = TcpConnectProcess;
        let tcp_relay_process = TcpRelayProcess;
        let framed_result = MessageFramedGenerator::generate(agent_stream, message_framed_buffer_size, compress, rsa_crypto_fetcher.clone()).await?;
        debug!("Connection [{}] is going to handle tcp connect.", connection_id);
        let tcp_connect_result = tcp_connect_process
            .exec(
                TcpConnectProcessRequest {
                    connection_id: connection_id.clone(),
                    message_framed_read: framed_result.message_framed_read,
                    message_framed_write: framed_result.message_framed_write,
                    agent_address: agent_address_clone,
                },
                configuration.clone(),
            )
            .await?;
        debug!("Connection [{}] is going to handle tcp relay.", connection_id);
        tcp_relay_process
            .exec(
                TcpRelayRequest {
                    connection_id: connection_id.clone(),
                    message_framed_read: tcp_connect_result.message_framed_read,
                    message_framed_write: tcp_connect_result.message_framed_write,
                    agent_address,
                    target_stream: tcp_connect_result.target_stream,
                    source_address: tcp_connect_result.source_address,
                    target_address: tcp_connect_result.target_address,
                    user_token: tcp_connect_result.user_token,
                    agent_tcp_connect_message_id: tcp_connect_result.agent_tcp_connect_message_id,
                },
                configuration,
            )
            .await?;
        debug!("Connection [{}] is finish relay.", connection_id);
        Ok(())
    }
}
