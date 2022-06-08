use std::{any::type_name, fmt::Debug};
use std::str::FromStr;

use chrono::Local;
use futures::TryFutureExt;
use tower::{Service, ServiceExt};
use tracing::{debug, Instrument};
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use uuid::Uuid;

pub struct LogTimer;

impl FormatTime for LogTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

pub fn init_log(directory: &str, file_name_prefix: &str, max_log_level: &str) -> WorkerGuard {
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
    _guard
}

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}

pub async fn ready_and_call_service<T, S>(
    service: &mut S, request: T,
) -> Result<S::Response, S::Error>
where
    S: Service<T>,
    S::Error: Debug,
    T: Debug,
{
    let service_type_name = type_name::<S>();
    let service_call_span = tracing::info_span!(
        "CALL_SERVICE=>",
        service_name = service_type_name,
        request = format!("{:#?}", request).as_str()
    );
    match service
        .ready()
        .and_then(|v| v.call(request))
        .instrument(service_call_span)
        .await
    {
        Ok(v) => Ok(v),
        Err(e) => {
            debug!(
                "Fail to invoke service [{}] because of errors: {:#?}",
                service_type_name, e
            );
            Err(e)
        },
    }
}
