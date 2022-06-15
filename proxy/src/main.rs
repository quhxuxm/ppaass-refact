extern crate core;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use config::{ProxyArguments, ProxyConfig};
use tracing::log::Level;

use common::init_log;

use crate::server::ProxyServer;

pub(crate) mod config;
pub(crate) mod server;
pub(crate) mod service;

fn init() -> ProxyConfig {
    let arguments = ProxyArguments::parse();
    let configuration_file_content = match arguments.configuration_file {
        None => {
            println!("Starting ppaass-proxy with defaul configuration file:  ppaass-proxy.toml");
            std::fs::read_to_string("ppaass-proxy.toml").expect("Fail to read proxy configuration file.")
        },
        Some(path) => {
            println!("Starting ppaass-proxy with customized configuration file: {}", path.as_str());
            std::fs::read_to_string(path.as_str()).expect("Fail to read proxy configuration file.")
        },
    };
    let mut configuration = toml::from_str::<ProxyConfig>(&configuration_file_content).expect("Fail to parse proxy configuration file");
    if let Some(port) = arguments.port {
        configuration.set_port(port);
    }
    if let Some(compress) = arguments.compress {
        configuration.set_compress(compress);
    }
    if let Some(log_dir) = arguments.log_dir {
        configuration.set_log_dir(log_dir)
    }
    if let Some(log_file) = arguments.log_file {
        configuration.set_log_file(log_file)
    }
    if let Some(target_buffer_size) = arguments.target_buffer_size {
        configuration.set_target_buffer_size(target_buffer_size)
    }
    if let Some(message_framed_buffer_size) = arguments.message_framed_buffer_size {
        configuration.set_message_framed_buffer_size(message_framed_buffer_size)
    }
    if let Some(rsa_root_dir) = arguments.rsa_root_dir {
        configuration.set_rsa_root_dir(rsa_root_dir)
    }
    if let Some(max_log_level) = arguments.max_log_level {
        configuration.set_max_log_level(max_log_level)
    }
    if let Some(rate_limit) = arguments.rate_limit {
        configuration.set_rate_limit(rate_limit)
    }
    if let Some(so_backlog) = arguments.so_backlog {
        configuration.set_so_backlog(so_backlog)
    }
    if let Some(connect_target_timeout_seconds) = arguments.connect_target_timeout_seconds {
        configuration.set_connect_target_timeout_seconds(connect_target_timeout_seconds)
    }
    if let Some(target_connection_retry) = arguments.target_connection_retry {
        configuration.set_target_connection_retry(target_connection_retry)
    }
    configuration
}
fn main() -> Result<()> {
    let configuration = init();
    let _tracing_guard = init_log(
        configuration.log_dir().as_ref().expect("No log directory given."),
        configuration.log_file().as_ref().expect("No log file name given."),
        configuration.max_log_level().as_ref().unwrap_or(&Level::Error.to_string()),
    );
    let proxy_server = ProxyServer::new(Arc::new(configuration))?;
    proxy_server.run()?;
    Ok(())
}
