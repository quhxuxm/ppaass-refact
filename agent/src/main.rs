use std::{io::Read, str::FromStr, sync::Arc};

use anyhow::Result;
use clap::Parser;
use common::LogTimer;
use config::{AgentArguments, AgentConfig, AgentLogConfig};
use tracing::{metadata::LevelFilter, Level};
use tracing_subscriber::{fmt::Layer, prelude::__tracing_subscriber_SubscriberExt, Registry};

use crate::server::AgentServer;

pub(crate) mod message {
    pub(crate) mod socks5;
}

pub(crate) mod codec;
pub(crate) mod server;

pub(crate) mod service;

pub(crate) mod config;

fn merge_arguments_and_log_config(arguments: &AgentArguments, log_config: &mut AgentLogConfig) {
    if let Some(ref log_dir) = arguments.log_dir {
        log_config.set_log_dir(log_dir.to_string())
    }
    if let Some(ref log_file) = arguments.log_file {
        log_config.set_log_file(log_file.to_string())
    }
    if let Some(ref max_log_level) = arguments.max_log_level {
        log_config.set_max_log_level(max_log_level.to_string())
    }
}

fn init() -> AgentConfig {
    let arguments = AgentArguments::parse();
    let mut log_configuration_file = std::fs::File::open("ppaass-agent-log.toml").expect("Fail to read agnet log configuration file.");
    let mut log_configuration_file_content = String::new();
    log_configuration_file
        .read_to_string(&mut log_configuration_file_content)
        .expect("Fail to read agnet log configuration file");

    let mut log_configuration = toml::from_str::<AgentLogConfig>(&log_configuration_file_content).expect("Fail to parse agnet log configuration file");

    merge_arguments_and_log_config(&arguments, &mut log_configuration);
    let log_directory = log_configuration.log_dir().as_ref().expect("No log directory given.");
    let log_file = log_configuration.log_file().as_ref().expect("No log file name given.");
    let default_log_level = &Level::ERROR.to_string();
    let log_max_level = log_configuration.max_log_level().as_ref().unwrap_or(default_log_level);

    let file_appender = tracing_appender::rolling::daily(log_directory, log_file);
    let (non_blocking, _appender_guard) = tracing_appender::non_blocking(file_appender);
    let log_level_filter = match LevelFilter::from_str(log_max_level) {
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
    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        panic!("Fail to initialize tracing subscriber because of error: {:#?}", e);
    };

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

    if let Some(client_buffer_size) = arguments.client_buffer_size {
        configuration.set_client_buffer_size(client_buffer_size)
    }
    if let Some(message_framed_buffer_size) = arguments.message_framed_buffer_size {
        configuration.set_message_framed_buffer_size(message_framed_buffer_size)
    }

    if let Some(so_backlog) = arguments.so_backlog {
        configuration.set_so_backlog(so_backlog)
    }
    configuration
}

fn main() -> Result<()> {
    let configuration = init();
    let agent_server = AgentServer::new(Arc::new(configuration))?;
    agent_server.run()?;
    Ok(())
}
