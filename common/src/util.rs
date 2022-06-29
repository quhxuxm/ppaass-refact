use std::str::FromStr;

use chrono::Local;

use tracing::{dispatcher::DefaultGuard, level_filters::LevelFilter};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::{time::FormatTime, Layer};
use tracing_subscriber::{fmt::format::Writer, prelude::__tracing_subscriber_SubscriberExt, Registry};
use uuid::Uuid;

pub struct LogTimer;

impl FormatTime for LogTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

pub fn init_log(directory: &str, file_name_prefix: &str, max_log_level: &str) -> (WorkerGuard, DefaultGuard) {
    let file_appender = tracing_appender::rolling::daily(directory, file_name_prefix);
    let (non_blocking, appender_guard) = tracing_appender::non_blocking(file_appender);
    let log_level_filter = match LevelFilter::from_str(max_log_level) {
        Err(e) => {
            panic!("Fail to initialize log because of error: {:#?}", e);
        },
        Ok(v) => v,
    };
    let subscriber = Registry::default()
        .with(
            Layer::default()
                .with_level(true)
                .with_target(true)
                .with_timer(LogTimer)
                .with_thread_ids(true)
                .with_file(true)
                .with_ansi(false)
                .with_line_number(true)
                .with_writer(non_blocking),
        )
        .with(log_level_filter);
    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    (appender_guard, subscriber_guard)
}

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}
