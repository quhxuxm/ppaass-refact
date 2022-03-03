use std::path::Path;

use lazy_static::lazy_static;
use serde_derive::Deserialize;
use serde_derive::Serialize;

pub const DEFAULT_TCP_BUFFER_SIZE: usize = 128 * 1024;
pub const DEFAULT_TCP_MAX_FRAME_SIZE: usize = DEFAULT_TCP_BUFFER_SIZE * 2;
pub const DEFAULT_UDP_BUFFER_SIZE: usize = 65536;

lazy_static! {
    pub(crate) static ref PROXY_SERVER_CONFIG: ProxyConfiguration = {
        let config_file_content = std::fs::read_to_string("ppaass-proxy.toml")
            .expect("Fail to read proxy configuration file.");
        toml::from_str::<ProxyConfiguration>(&config_file_content)
            .expect("Fail to parse proxy configuration file")
    };
    pub(crate) static ref AGENT_PUBLIC_KEY: String =
        std::fs::read_to_string(Path::new("AgentPublicKey.pem"))
            .expect("Fail to read agent public key.");
    pub(crate) static ref PROXY_PRIVATE_KEY: String =
        std::fs::read_to_string(Path::new("ProxyPrivateKey.pem"))
            .expect("Fail to read proxy private key.");
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProxyConfiguration {
    port: Option<u16>,
    buffer_size: Option<usize>,
    max_frame_size: Option<usize>,
    thread_number: Option<usize>,
    max_blocking_threads: Option<usize>,
    thread_timeout: Option<u64>,
    target_connect_timeout: Option<u64>,
    log_dir: Option<String>,
    log_file: Option<String>,
    compress: Option<bool>,
}

impl ProxyConfiguration {
    pub fn port(&self) -> Option<u16> {
        self.port
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

    pub fn thread_timeout(&self) -> Option<u64> {
        self.thread_timeout
    }

    pub fn target_connect_timeout(&self) -> Option<u64> {
        self.target_connect_timeout
    }

    pub fn log_dir(&self) -> &Option<String> {
        &self.log_dir
    }

    pub fn log_file(&self) -> &Option<String> {
        &self.log_file
    }

    pub fn max_blocking_threads(&self) -> Option<usize> {
        self.max_blocking_threads
    }

    pub fn compress(&self) -> Option<bool> {
        self.compress
    }
}
