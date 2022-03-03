mod config;
mod server;

use chrono::Local;
use config::PROXY_SERVER_CONFIG;
use server::Server;
use tracing::{debug, error, info};
use tracing_appender::non_blocking;
use tracing_appender::rolling::hourly;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

struct LogTimer;

impl FormatTime for LogTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

fn main() {
    let file_appender = hourly(
        PROXY_SERVER_CONFIG
            .log_dir()
            .as_ref()
            .expect("No log directory given."),
        PROXY_SERVER_CONFIG
            .log_file()
            .as_ref()
            .expect("No log file name given."),
    );
    let (non_blocking, _guard) = non_blocking(file_appender);
    tracing_subscriber::fmt()
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
    let server = match Server::new() {
        Ok(v) => v,
        Err(e) => {
            error!("Fail to create server instance because of error: {:#?}", e);
            return;
        }
    };
    match server.run() {
        Err(e) => {
            error!("Server fail to start because of error: {:#?}", e);
        }
        Ok(_) => {
            info!("Server graceful shutdown");
        }
    };
    server.shutdown();
}
