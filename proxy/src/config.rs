#![allow(unused)]

use std::path::Path;

use clap::Parser;
use lazy_static::lazy_static;
use serde_derive::Deserialize;
use serde_derive::Serialize;
lazy_static! {
    pub(crate) static ref AGENT_PUBLIC_KEY: String = std::fs::read_to_string(Path::new("AgentPublicKey.pem")).expect("Fail to read agent public key.");
    pub(crate) static ref PROXY_PRIVATE_KEY: String = std::fs::read_to_string(Path::new("ProxyPrivateKey.pem")).expect("Fail to read proxy private key.");
}

pub const DEFAULT_READ_AGENT_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_CONNECT_TARGET_RETRY: u16 = 2;
pub const DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_TARGET_STREAM_SO_LINGER: u64 = 20;
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct ProxyConfig {
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
    /// The connect to target timeout in seconds
    connect_target_timeout_seconds: Option<u64>,
    agent_stream_so_linger: Option<u64>,
    target_stream_so_linger: Option<u64>,
    so_backlog: Option<u32>,
}

impl ProxyConfig {
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn set_port(&mut self, port: u16) {
        self.port = Some(port);
    }
    pub fn target_buffer_size(&self) -> Option<usize> {
        self.target_buffer_size
    }
    pub fn set_target_buffer_size(&mut self, target_buffer_size: usize) {
        self.target_buffer_size = Some(target_buffer_size)
    }
    pub fn message_framed_buffer_size(&self) -> Option<usize> {
        self.message_framed_buffer_size
    }
    pub fn set_message_framed_buffer_size(&mut self, message_framed_buffer_size: usize) {
        self.message_framed_buffer_size = Some(message_framed_buffer_size)
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
    pub fn set_log_dir(&mut self, log_dir: String) {
        self.log_dir = Some(log_dir);
    }
    pub fn log_file(&self) -> &Option<String> {
        &self.log_file
    }
    pub fn set_log_file(&mut self, log_file: String) {
        self.log_file = Some(log_file)
    }
    pub fn max_blocking_threads(&self) -> Option<usize> {
        self.max_blocking_threads
    }
    pub fn compress(&self) -> Option<bool> {
        self.compress
    }
    pub fn set_compress(&mut self, compress: bool) {
        self.compress = Some(compress);
    }
    pub fn max_log_level(&self) -> &Option<String> {
        &self.max_log_level
    }
    pub fn set_max_log_level(&mut self, max_log_level: String) {
        self.max_log_level = Some(max_log_level)
    }
    pub fn target_connection_retry(&self) -> Option<u16> {
        self.target_connection_retry
    }
    pub fn set_target_connection_retry(&mut self, target_connection_retry: u16) {
        self.target_connection_retry = Some(target_connection_retry)
    }
    pub fn rsa_root_dir(&self) -> &Option<String> {
        &self.rsa_root_dir
    }
    pub fn set_rsa_root_dir(&mut self, rsa_root_dir: String) {
        self.rsa_root_dir = Some(rsa_root_dir)
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
    pub fn set_rate_limit(&mut self, rate_limit: u64) {
        self.rate_limit = Some(rate_limit)
    }
    pub fn connect_target_timeout_seconds(&self) -> Option<u64> {
        self.connect_target_timeout_seconds
    }
    pub fn set_connect_target_timeout_seconds(&mut self, connect_target_timeout_seconds: u64) {
        self.connect_target_timeout_seconds = Some(connect_target_timeout_seconds)
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
    pub fn set_so_backlog(&mut self, so_backlog: u32) {
        self.so_backlog = Some(so_backlog)
    }
    pub fn so_recv_buffer_size(&self) -> Option<u32> {
        self.so_recv_buffer_size
    }
    pub fn so_send_buffer_size(&self) -> Option<u32> {
        self.so_send_buffer_size
    }
}

#[derive(Parser, Debug)]
#[clap(name="ppaass-proxy", author="Qu Hao", version="1.0", about, long_about = None)]
/// The proxy side of the ppaass, which will proxy
/// the agent connection to the target server
pub(crate) struct ProxyArguments {
    /// Configuration file path
    #[clap(short = 'c', long, value_parser)]
    pub configuration_file: Option<String>,
    /// Port of the ppaass proxy
    #[clap(short = 'p', long, value_parser)]
    pub port: Option<u16>,
    /// The root directory used to store the rsa
    /// files for each user
    #[clap(short = 'r', long, value_parser)]
    pub rsa_root_dir: Option<String>,
    /// The log directory
    #[clap(long, value_parser)]
    pub log_dir: Option<String>,
    /// The log file name prefix
    #[clap(long, value_parser)]
    pub log_file: Option<String>,
    /// Whether enable compressing
    #[clap(short = 'z', long, value_parser)]
    pub compress: Option<bool>,
    /// The max log level
    #[clap(long, value_parser)]
    pub max_log_level: Option<String>,
    /// The retry for target connection
    #[clap(long, value_parser)]
    pub target_connection_retry: Option<u16>,
    /// The rate limit
    #[clap(long, value_parser)]
    pub rate_limit: Option<u64>,
    /// The connect to target timeout seconds
    #[clap(long, value_parser)]
    pub connect_target_timeout_seconds: Option<u64>,
    /// The so_backlog
    #[clap(long, value_parser)]
    pub so_backlog: Option<u32>,
    /// The target buffer size
    #[clap(long, value_parser)]
    pub target_buffer_size: Option<usize>,
    /// The message framed buffer size
    #[clap(long, value_parser)]
    pub message_framed_buffer_size: Option<usize>,
}
