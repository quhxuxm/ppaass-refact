use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use config::{AgentArguments, AgentConfig};
use tracing::Level;

use common::init_log;

use crate::server::AgentServer;

pub(crate) mod message {
    pub(crate) mod socks5;
}

pub(crate) mod codec;
pub(crate) mod server;

pub(crate) mod service;

pub(crate) mod config;

fn init() -> AgentConfig {
    let arguments = AgentArguments::parse();
    let configuration_file_content = match arguments.configuration_file {
        None => {
            println!("Starting ppaass-agent with default configuration file:  ppaass-agent.toml");
            std::fs::read_to_string("ppaass-agent.toml").expect("Fail to read agent configuration file.")
        },
        Some(path) => {
            println!("Starting ppaass-agent with customized configuration file: {}", path.as_str());
            std::fs::read_to_string(path.as_str()).expect("Fail to read agent configuration file.")
        },
    };
    let mut configuration = toml::from_str::<AgentConfig>(&configuration_file_content).expect("Fail to parse agent configuration file");
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
    if let Some(client_buffer_size) = arguments.client_buffer_size {
        configuration.set_client_buffer_size(client_buffer_size)
    }
    if let Some(message_framed_buffer_size) = arguments.message_framed_buffer_size {
        configuration.set_message_framed_buffer_size(message_framed_buffer_size)
    }
    if let Some(max_log_level) = arguments.max_log_level {
        configuration.set_max_log_level(max_log_level)
    }
    if let Some(so_backlog) = arguments.so_backlog {
        configuration.set_so_backlog(so_backlog)
    }
    configuration
}
fn main() -> Result<()> {
    let confiugration = init();
    let _tracing_guard = init_log(
        confiugration.log_dir().as_ref().expect("No log directory given."),
        confiugration.log_file().as_ref().expect("No log file name given."),
        confiugration.max_log_level().as_ref().unwrap_or(&Level::ERROR.to_string()),
    );
    let agent_server = AgentServer::new(Arc::new(confiugration))?;
    agent_server.run()?;
    Ok(())
}
