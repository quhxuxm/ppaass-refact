use anyhow::Result;
use tracing::log::Level;

use common::init_log;

use crate::config::SERVER_CONFIG;
use crate::server::AgentServer;

pub(crate) mod command {
    pub(crate) mod socks5;
}

pub(crate) mod server;

pub(crate) mod codec {
    pub(crate) mod http;
    pub(crate) mod socks5;
}

pub(crate) mod service;

pub(crate) mod config;

fn main() -> Result<()> {
    init_log(SERVER_CONFIG
        .log_dir()
        .as_ref()
        .expect("No log directory given."), SERVER_CONFIG
        .log_file()
        .as_ref()
        .expect("No log file name given."), SERVER_CONFIG
        .max_log_level()
        .as_ref()
        .unwrap_or(&Level::Error.to_string()));
    let agent_server = AgentServer::new();
    agent_server.run();
    Ok(())
}
