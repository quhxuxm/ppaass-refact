#![allow(unused)]

use std::path::Path;

use lazy_static::lazy_static;
use serde_derive::Deserialize;
use serde_derive::Serialize;

lazy_static! {
    pub(crate) static ref SERVER_CONFIG: Config = {
        let config_file_content = std::fs::read_to_string("ppaass-proxy.toml")
            .expect("Fail to read proxy configuration file.");
        toml::from_str::<Config>(&config_file_content)
            .expect("Fail to parse proxy configuration file")
    };
    pub(crate) static ref AGENT_PUBLIC_KEY: String =
        std::fs::read_to_string(Path::new("AgentPublicKey.pem"))
            .expect("Fail to read agent public key.");
    pub(crate) static ref PROXY_PRIVATE_KEY: String =
        std::fs::read_to_string(Path::new("ProxyPrivateKey.pem"))
            .expect("Fail to read proxy private key.");
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Config {
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
    max_log_level: Option<String>,
    target_connection_retry: Option<u16>,
    buffered_connection_number: Option<usize>,
    concurrent_connection_number: Option<usize>,
    rate_limit: Option<u64>,
}

impl Config {
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

    pub fn max_log_level(&self) -> &Option<String> {
        &self.max_log_level
    }
    pub fn target_connection_retry(&self) -> Option<u16> {
        self.target_connection_retry
    }

    pub fn buffered_connection_number(&self) -> Option<usize> {
        self.buffered_connection_number
    }
    pub fn concurrent_connection_number(&self) -> Option<usize> {
        self.concurrent_connection_number
    }
    pub fn rate_limit(&self) -> Option<u64> {
        self.rate_limit
    }
}
