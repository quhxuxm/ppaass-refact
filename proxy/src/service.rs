use std::{collections::HashMap, net::SocketAddr};
use std::{fmt::Debug, sync::Arc};
use std::{fs, path::Path};

use tokio::net::TcpStream;

use common::{generate_uuid, MessageFramedGenerateResult, MessageFramedGenerator, PpaassError, RsaCrypto, RsaCryptoFetcher};

use tracing::{debug, error, instrument};

use crate::service::{
    init::{InitFlowRequest, InitFlowResult},
    tcp::relay::{TcpRelayFlow, TcpRelayFlowRequest},
    udp::relay::{UdpRelayFlow, UdpRelayFlowRequest},
};
use crate::{config::ProxyConfig, service::init::InitializeFlow};

use anyhow::Result;
mod init;
mod tcp;
mod udp;

const DEFAULT_BUFFER_SIZE: usize = 1024 * 64;

#[derive(Debug)]
pub(crate) struct ProxyRsaCryptoFetcher {
    cache: HashMap<String, RsaCrypto>,
}

impl ProxyRsaCryptoFetcher {
    #[instrument(skip_all)]
    pub fn new(configuration: &ProxyConfig) -> Result<Self> {
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
    #[instrument(skip_all, fields(user_token))]
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

    #[instrument(skip_all)]
    pub async fn exec<T>(self, rsa_crypto_fetcher: Arc<T>, configuration: Arc<ProxyConfig>) -> Result<()>
    where
        T: RsaCryptoFetcher + Send + Sync + Debug + 'static,
    {
        let connection_id = self.id.clone();
        debug!("Begin to handle agent connection: {}", connection_id);
        let message_framed_buffer_size = configuration.message_framed_buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE);
        let compress = configuration.compress().unwrap_or(true);
        let agent_stream = self.agent_stream;
        let agent_address_clone = self.agent_address.clone();
        let agent_address = self.agent_address;
        let MessageFramedGenerateResult {
            mut message_framed_write,
            mut message_framed_read,
        } = MessageFramedGenerator::generate(agent_stream, message_framed_buffer_size, compress, rsa_crypto_fetcher).await;
        debug!("Connection [{}] is going to handle tcp connect.", connection_id);

        loop {
            let init_flow_result = InitializeFlow::exec(
                InitFlowRequest {
                    connection_id: connection_id.as_str(),
                    message_framed_read,
                    message_framed_write,
                    agent_address: agent_address_clone,
                },
                &configuration,
            )
            .await?;
            match init_flow_result {
                InitFlowResult::Heartbeat {
                    message_framed_read: message_framed_read_pass_back,
                    message_framed_write: message_framed_write_pass_back,
                } => {
                    message_framed_read = message_framed_read_pass_back;
                    message_framed_write = message_framed_write_pass_back;
                    continue;
                },
                InitFlowResult::Tcp {
                    target_stream,
                    message_framed_read,
                    message_framed_write,
                    source_address,
                    target_address,
                    user_token,
                    ..
                } => {
                    debug!("Connection [{}] is going to handle tcp relay.", connection_id);
                    TcpRelayFlow::exec(
                        TcpRelayFlowRequest {
                            connection_id: &connection_id,
                            message_framed_read,
                            message_framed_write,
                            agent_address,
                            target_stream,
                            source_address,
                            target_address,
                            user_token: &user_token,
                        },
                        &configuration,
                    )
                    .await?;
                    debug!("Connection [{}] is finish tcp relay.", connection_id);
                    break;
                },
                InitFlowResult::Udp {
                    message_framed_read,
                    message_framed_write,
                    message_id,
                    user_token,
                    ..
                } => {
                    debug!("Connection [{}] is going to handle udp relay.", connection_id);
                    UdpRelayFlow::exec(UdpRelayFlowRequest {
                        connection_id: connection_id.as_str(),
                        message_framed_read,
                        message_framed_write,
                        message_id: message_id.as_str(),
                        user_token: user_token.as_str(),
                    })
                    .await?;
                    debug!("Connection [{}] is finish udp relay.", connection_id);
                    break;
                },
            }
        }

        Ok(())
    }
}
