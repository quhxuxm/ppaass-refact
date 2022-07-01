#![allow(unused)]
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;

use clap::Parser;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use tracing::error;

#[derive(Serialize, Deserialize, Debug)]
pub struct AgentLogConfig {
    log_dir: Option<String>,
    log_file: Option<String>,
    max_log_level: Option<String>,
}

impl AgentLogConfig {
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
#[derive(Serialize, Deserialize, Debug)]
pub struct AgentConfig {
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
    compress: Option<bool>,
    client_stream_so_linger: Option<u64>,
    proxy_stream_so_linger: Option<u64>,
    so_backlog: Option<u32>,
    agent_private_key_file: Option<String>,
    proxy_public_key_file: Option<String>,
    init_proxy_connection_number: Option<usize>,
    min_proxy_connection_number: Option<usize>,
    proxy_connection_number_incremental: Option<usize>,
    proxy_connection_check_interval_seconds: Option<u64>,
    proxy_connection_check_timeout: Option<u64>,
}

impl AgentConfig {
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn set_port(&mut self, port: u16) {
        self.port = Some(port);
    }
    pub fn user_token(&self) -> &Option<String> {
        &self.user_token
    }
    pub fn set_user_token(&mut self, user_token: String) {
        self.user_token = Some(user_token);
    }
    pub fn proxy_addresses(&self) -> &Option<Vec<String>> {
        &self.proxy_addresses
    }
    pub fn set_proxy_addresses(&mut self, proxy_addresses: Vec<String>) {
        self.proxy_addresses = Some(proxy_addresses);
    }
    pub fn client_buffer_size(&self) -> Option<usize> {
        self.client_buffer_size
    }
    pub fn set_client_buffer_size(&mut self, client_buffer_size: usize) {
        self.client_buffer_size = Some(client_buffer_size)
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
    pub fn max_blocking_threads(&self) -> Option<usize> {
        self.max_blocking_threads
    }
    pub fn thread_timeout(&self) -> Option<u64> {
        self.thread_timeout
    }
    pub fn compress(&self) -> Option<bool> {
        self.compress
    }
    pub fn set_compress(&mut self, compress: bool) {
        self.compress = Some(compress);
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
    pub fn set_so_backlog(&mut self, so_backlog: u32) {
        self.so_backlog = Some(so_backlog)
    }
    pub fn so_recv_buffer_size(&self) -> Option<u32> {
        self.so_recv_buffer_size
    }
    pub fn so_send_buffer_size(&self) -> Option<u32> {
        self.so_send_buffer_size
    }
    pub fn agent_private_key_file(&self) -> &Option<String> {
        &self.agent_private_key_file
    }
    pub fn set_agent_private_key_file(&mut self, agent_private_key_file: String) {
        self.agent_private_key_file = Some(agent_private_key_file)
    }
    pub fn proxy_public_key_file(&self) -> &Option<String> {
        &self.proxy_public_key_file
    }
    pub fn set_proxy_public_key_file(&mut self, proxy_public_key_file: String) {
        self.proxy_public_key_file = Some(proxy_public_key_file)
    }
    pub fn init_proxy_connection_number(&self) -> Option<usize> {
        self.init_proxy_connection_number
    }
    pub fn min_proxy_connection_number(&self) -> Option<usize> {
        self.min_proxy_connection_number
    }
    pub fn proxy_connection_check_interval_seconds(&self) -> Option<u64> {
        self.proxy_connection_check_interval_seconds
    }
    pub fn proxy_connection_number_increasement(&self) -> Option<usize> {
        self.proxy_connection_number_incremental
    }
    pub fn proxy_connection_check_timeout(&self) -> Option<u64> {
        self.proxy_connection_check_timeout
    }
}

#[derive(Parser, Debug)]
#[clap(name="ppaass-agent", author="Qu Hao", version="1.0", about, long_about = None)]
/// The agent side of the ppaass, which will run as a http or sock5 agent
/// in client side and transfer the connection to the ppaass proxy side.
pub(crate) struct AgentArguments {
    /// Configuration file path
    #[clap(short = 'c', long, value_parser)]
    pub configuration_file: Option<String>,
    /// Port of the ppaass proxy
    #[clap(short = 'p', long, value_parser)]
    pub port: Option<u16>,
    /// The user token used to authenticate ppaass proxy
    #[clap(short = 'u', long, value_parser)]
    pub user_token: Option<String>,
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
    /// The client buffer size
    #[clap(long, value_parser)]
    pub client_buffer_size: Option<usize>,
    /// The message framed buffer size
    #[clap(long, value_parser)]
    pub message_framed_buffer_size: Option<usize>,
    /// The proxy addresses
    #[clap(long, value_parser)]
    pub proxy_addresses: Option<Vec<String>>,
    /// The agent private key file path
    #[clap(long, value_parser)]
    pub agent_private_key_file: Option<String>,
    /// The proxy public key file path
    #[clap(long, value_parser)]
    pub proxy_public_key_file: Option<String>,
}
