#![allow(unused)]
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;

use lazy_static::lazy_static;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use tracing::error;

lazy_static! {
    pub(crate) static ref SERVER_CONFIG: Config = {
        let config_file_content = std::fs::read_to_string("ppaass-agent.toml").expect("Fail to read agent configuration file.");
        toml::from_str::<Config>(&config_file_content).expect("Fail to parse agent configuration file")
    };
    pub(crate) static ref AGENT_PRIVATE_KEY: String = std::fs::read_to_string(Path::new("AgentPrivateKey.pem")).expect("Fail to read agent private key.");
    pub(crate) static ref PROXY_PUBLIC_KEY: String = std::fs::read_to_string(Path::new("ProxyPublicKey.pem")).expect("Fail to read proxy public key.");
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Config {
    port: Option<u16>,
    so_recv_buffer_size: Option<u32>,
    so_send_buffer_size: Option<u32>,
    user_token: Option<String>,
    proxy_addresses: Option<Vec<String>>,
    client_buffer_size: Option<usize>,
    message_framed_buffer_size: Option<usize>,
    thread_number: Option<usize>,
    max_blocking_threads: Option<usize>,
    thread_timeout: Option<u64>,
    log_dir: Option<String>,
    log_file: Option<String>,
    max_log_level: Option<String>,
    proxy_connection_retry: Option<u16>,
    compress: Option<bool>,
    buffered_connection_number: Option<usize>,
    concurrent_connection_number: Option<usize>,
    rate_limit: Option<u64>,
    read_proxy_timeout_seconds: Option<u64>,
    read_client_timeout_seconds: Option<u64>,
    connect_proxy_timeout_seconds: Option<u64>,
    client_stream_so_linger: Option<u64>,
    proxy_stream_so_linger: Option<u64>,
    so_backlog: Option<u32>,
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
    pub fn client_buffer_size(&self) -> Option<usize> {
        self.client_buffer_size
    }
    pub fn message_framed_buffer_size(&self) -> Option<usize> {
        self.message_framed_buffer_size
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
    pub fn max_log_level(&self) -> &Option<String> {
        &self.max_log_level
    }
    pub fn proxy_connection_retry(&self) -> Option<u16> {
        self.proxy_connection_retry
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
    pub fn read_proxy_timeout_seconds(&self) -> Option<u64> {
        self.read_proxy_timeout_seconds
    }
    pub fn read_client_timeout_seconds(&self) -> Option<u64> {
        self.read_client_timeout_seconds
    }
    pub fn connect_proxy_timeout_seconds(&self) -> Option<u64> {
        self.connect_proxy_timeout_seconds
    }
    pub fn client_stream_so_linger(&self) -> Option<u64> {
        self.client_stream_so_linger
    }
    pub fn proxy_stream_so_linger(&self) -> Option<u64> {
        self.proxy_stream_so_linger
    }
    pub fn so_backlog(&self) -> Option<u32> {
        self.so_backlog
    }
    pub fn so_recv_buffer_size(&self) -> Option<u32> {
        self.so_recv_buffer_size
    }
    pub fn so_send_buffer_size(&self) -> Option<u32> {
        self.so_send_buffer_size
    }
}
