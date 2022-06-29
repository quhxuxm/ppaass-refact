#![allow(unused)]

use std::path::Path;

use clap::Parser;
use serde_derive::Deserialize;
use serde_derive::Serialize;

pub const DEFAULT_READ_AGENT_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_CONNECT_TARGET_RETRY: u16 = 2;
pub const DEFAULT_CONNECT_TARGET_TIMEOUT_SECONDS: u64 = 20;
pub const DEFAULT_TARGET_STREAM_SO_LINGER: u64 = 20;

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct ProxyLogConfig {
    /// The log directory
    log_dir: Option<String>,
    /// The log file name prefix
    log_file: Option<String>,
    /// The max log level
    max_log_level: Option<String>,
}

impl ProxyLogConfig {
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
    pub fn max_log_level(&self) -> &Option<String> {
        &self.max_log_level
    }
    pub fn set_max_log_level(&mut self, max_log_level: String) {
        self.max_log_level = Some(max_log_level)
    }
}
#[derive(Serialize, Deserialize, Debug, Default)]
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
    /// Whether enable compressing
    compress: Option<bool>,
    agent_stream_so_linger: Option<u64>,
    target_stream_so_linger: Option<u64>,
    agent_connection_read_timeout: Option<u64>,
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

    pub fn max_blocking_threads(&self) -> Option<usize> {
        self.max_blocking_threads
    }
    pub fn compress(&self) -> Option<bool> {
        self.compress
    }
    pub fn set_compress(&mut self, compress: bool) {
        self.compress = Some(compress);
    }

    pub fn rsa_root_dir(&self) -> &Option<String> {
        &self.rsa_root_dir
    }
    pub fn set_rsa_root_dir(&mut self, rsa_root_dir: String) {
        self.rsa_root_dir = Some(rsa_root_dir)
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
    pub fn agent_connection_read_timeout(&self) -> Option<u64> {
        self.agent_connection_read_timeout
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
