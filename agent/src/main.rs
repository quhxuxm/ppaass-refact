use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;

use anyhow::Result;
use chrono::Local;
use tracing::level_filters::LevelFilter;
use tracing::log::Level;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

use crate::config::SERVER_CONFIG;
use crate::server::AgentServer;

pub(crate) mod command {
    pub(crate) mod socks5;
}

pub(crate) mod server;

pub(crate) mod codec {
    pub(crate) mod socks5;
}

pub(crate) mod service;

pub(crate) mod config;

pub(crate) struct LogTimer;

impl FormatTime for LogTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

fn main() -> Result<()> {
    let file_appender = tracing_appender::rolling::daily(
        SERVER_CONFIG
            .log_dir()
            .as_ref()
            .expect("No log directory given."),
        SERVER_CONFIG
            .log_file()
            .as_ref()
            .expect("No log file name given."),
    );
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let log_level_filter = LevelFilter::from_str(
        SERVER_CONFIG
            .max_log_level()
            .as_ref()
            .unwrap_or(&Level::Error.to_string()),
    )?;
    tracing_subscriber::fmt()
        .with_max_level(log_level_filter)
        .with_level(true)
        .with_target(true)
        .with_timer(LogTimer)
        .with_thread_ids(true)
        .with_file(true)
        .with_ansi(false)
        .with_line_number(true)
        .with_writer(non_blocking)
        .with_ansi(true)
        .init();
    let agent_server = AgentServer::new();
    agent_server.run();
    Ok(())
}
