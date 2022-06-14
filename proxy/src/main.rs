extern crate core;

use anyhow::Result;
use tracing::log::Level;

use common::init_log;

use crate::config::SERVER_CONFIG;
use crate::server::ProxyServer;

pub(crate) mod config;
pub(crate) mod server;
pub(crate) mod service;

fn main() -> Result<()> {
    let _tracing_guard = init_log(
        SERVER_CONFIG.log_dir().as_ref().expect("No log directory given."),
        SERVER_CONFIG.log_file().as_ref().expect("No log file name given."),
        SERVER_CONFIG.max_log_level().as_ref().unwrap_or(&Level::Error.to_string()),
    );
    let proxy_server = ProxyServer::new()?;
    proxy_server.run()?;
    Ok(())
}
