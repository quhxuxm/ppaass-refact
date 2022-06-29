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

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}
