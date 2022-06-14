#![allow(unused)]

use std::path::Path;

use clap::Parser;
use lazy_static::lazy_static;
use serde_derive::Deserialize;
use serde_derive::Serialize;
lazy_static! {
    pub(crate) static ref SERVER_CONFIG: Config = {
        let config_file_content = std::fs::read_to_string("ppaass-proxy.toml").expect("Fail to read proxy configuration file.");
        toml::from_str::<Config>(&config_file_content).expect("Fail to parse proxy configuration file")
    };
    pub(crate) static ref AGENT_PUBLIC_KEY: String = std::fs::read_to_string(Path::new("AgentPublicKey.pem")).expect("Fail to read agent public key.");
    pub(crate) static ref PROXY_PRIVATE_KEY: String = std::fs::read_to_string(Path::new("ProxyPrivateKey.pem")).expect("Fail to read proxy private key.");
}

pub const DEFAULT_READ_AGENT_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_CONNECT_TARGET_RETRY: u16 = 2;
pub const DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS: u64 = 20;

#[derive(Serialize, Deserialize)]
pub(crate) struct Config {
    /// Port of the ppaass proxy
    port: Option<u16>,
    /// The root directory used to store the rsa
    /// files for each user
    rsa_root_dir: Option<String>,
    /// The so_recv_buf size
    so_recv_buffer_size: Option<u32>,
    /// The so_snd_buf size
    so_send_buffer_size: Option<u32>,
    /// The target buffer size
    target_buffer_size: Option<usize>,
    /// The message framed buffer size
    message_framed_buffer_size: Option<usize>,
    /// The threads number
    thread_number: Option<usize>,
    /// The max blocking threads number
    max_blocking_threads: Option<usize>,
    /// The thread timeout
    thread_timeout: Option<u64>,
    /// The log directory
    log_dir: Option<String>,
    /// The log file name prefix
    log_file: Option<String>,
    /// Whether enable compressing
    compress: Option<bool>,
    /// The max log level
    max_log_level: Option<String>,
    /// The retry for target connection
    target_connection_retry: Option<u16>,
    /// The buffered connection number
    buffered_connection_number: Option<usize>,
    /// The concurrent connection number
    concurrent_connection_number: Option<usize>,
    /// The rate limit
    rate_limit: Option<u64>,
    connect_target_timeout_seconds: Option<u64>,
    agent_stream_so_linger: Option<u64>,
    target_stream_so_linger: Option<u64>,
    so_backlog: Option<u32>,
}

impl Config {
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn target_buffer_size(&self) -> Option<usize> {
        self.target_buffer_size
    }
    pub fn message_framed_buffer_size(&self) -> Option<usize> {
        self.message_framed_buffer_size
    }
    pub fn thread_number(&self) -> Option<usize> {
        self.thread_number
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
    pub fn rsa_root_dir(&self) -> &Option<String> {
        &self.rsa_root_dir
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
    pub fn connect_target_timeout_seconds(&self) -> Option<u64> {
        self.connect_target_timeout_seconds
    }
    pub fn target_stream_so_linger(&self) -> Option<u64> {
        self.target_stream_so_linger
    }
    pub fn agent_stream_so_linger(&self) -> Option<u64> {
        self.agent_stream_so_linger
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

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct ArgumentConfig {
    /// Port of the ppaass proxy
    #[clap(short, long, value_parser)]
    port: Option<u16>,
    /// The root directory used to store the rsa
    /// files for each user
    #[clap(short, long, value_parser)]
    rsa_root_dir: Option<String>,
    /// The log directory
    #[clap(short, long, value_parser)]
    log_dir: Option<String>,
    /// The log file name prefix
    #[clap(short, long, value_parser)]
    log_file: Option<String>,
    /// Whether enable compressing
    #[clap(short, long, value_parser)]
    compress: Option<bool>,
    /// The max log level
    #[clap(short, long, value_parser)]
    max_log_level: Option<String>,
    /// The retry for target connection
    #[clap(short, long, value_parser)]
    target_connection_retry: Option<u16>,
    /// The rate limit
    #[clap(short, long, value_parser)]
    rate_limit: Option<u64>,
    /// The connect to target timeout seconds
    #[clap(short, long, value_parser)]
    connect_target_timeout_seconds: Option<u64>,
    /// The so_backlog
    #[clap(short, long, value_parser)]
    so_backlog: Option<u32>,
}
