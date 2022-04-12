use std::path::Path;
use std::str::FromStr;
use std::{any::type_name, fmt::Debug};

use chrono::Local;
use futures_util::TryFutureExt;
use tower::{Service, ServiceExt};
use tracing::error;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use uuid::Uuid;

pub struct LogTimer;

impl FormatTime for LogTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

pub fn init_log(
    directory: impl AsRef<Path>,
    file_name_prefix: impl AsRef<Path>,
    max_log_level: &str,
) {
    let file_appender = tracing_appender::rolling::daily(directory, file_name_prefix);
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let log_level_filter = match LevelFilter::from_str(max_log_level) {
        Err(e) => {
            panic!("Fail to initialize log because of error: {:#?}", e);
        },
        Ok(v) => v,
    };
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
        .init();
}

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}

pub async fn ready_and_call_service<T, S>(
    service: &mut S,
    request: T,
) -> Result<S::Response, S::Error>
where
    S: Service<T>,
    S::Error: Debug,
{
    let service_type_name = type_name::<S>();
    match service.ready().and_then(|v| v.call(request)).await {
        Ok(v) => Ok(v),
        Err(e) => {
            error!(
                "Fail to invoke service [{}] because of errors: {:#?}",
                service_type_name, e
            );
            Err(e)
        },
    }
}
