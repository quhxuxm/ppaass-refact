use std::path::Path;

use lazy_static::lazy_static;
use serde_derive::Deserialize;
use serde_derive::Serialize;

lazy_static! {
    pub(crate) static ref SERVER_CONFIG: Config = {
        let config_file_content = std::fs::read_to_string("ppaass-agent.toml")
            .expect("Fail to read agent configuration file.");
        toml::from_str::<Config>(&config_file_content)
            .expect("Fail to parse agent configuration file")
    };
    pub(crate) static ref AGENT_PRIVATE_KEY: String =
        std::fs::read_to_string(Path::new("AgentPrivateKey.pem"))
            .expect("Fail to read agent private key.");
    pub(crate) static ref PROXY_PUBLIC_KEY: String =
        std::fs::read_to_string(Path::new("ProxyPublicKey.pem"))
            .expect("Fail to read proxy public key.");
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Config {
    port: Option<u16>,
    user_token: Option<String>,
    proxy_addresses: Option<Vec<String>>,
    buffer_size: Option<usize>,
    max_frame_size: Option<usize>,
    thread_number: Option<usize>,
    max_blocking_threads: Option<usize>,
    thread_timeout: Option<u64>,
    log_dir: Option<String>,
    log_file: Option<String>,
    compress: Option<bool>,
}

impl Config {
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn user_token(&self) -> &Option<String> {
        &self.user_token
    }
    pub fn proxy_addresses(&self) -> &Option<Vec<String>> {
        &self.proxy_addresses
    }
    pub fn buffer_size(&self) -> Option<usize> {
        self.buffer_size
    }
    pub fn max_frame_size(&self) -> Option<usize> {
        self.max_frame_size
    }
    pub fn thread_number(&self) -> Option<usize> {
        self.thread_number
    }
    pub fn max_blocking_threads(&self) -> Option<usize> {
        self.max_blocking_threads
    }
    pub fn thread_timeout(&self) -> Option<u64> {
        self.thread_timeout
    }
    pub fn log_dir(&self) -> &Option<String> {
        &self.log_dir
    }
    pub fn log_file(&self) -> &Option<String> {
        &self.log_file
    }
    pub fn compress(&self) -> Option<bool> {
        self.compress
    }
}
