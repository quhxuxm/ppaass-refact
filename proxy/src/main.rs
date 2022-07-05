extern crate core;

use std::{io::Read, path::Path, str::FromStr, sync::mpsc::channel};

use crate::server::ProxyServer;
use anyhow::anyhow;
use anyhow::Result;
use clap::Parser;
use common::LogTimer;
use config::{ProxyArguments, ProxyConfig, ProxyLogConfig};
use hotwatch::{Event, Hotwatch};
use server::ProxyServerSignal;

use tracing::Level;
use tracing::{info, metadata::LevelFilter};
use tracing_subscriber::Registry;
use tracing_subscriber::{fmt::Layer, prelude::__tracing_subscriber_SubscriberExt};
pub(crate) mod config;
pub(crate) mod server;
pub(crate) mod service;

const PROXY_LOG_CONFIG_FILE: &str = "ppaass-proxy-log.toml";

fn merge_arguments_and_log_config(arguments: &ProxyArguments, log_config: &mut ProxyLogConfig) {
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

fn merge_arguments_and_config(arguments: &ProxyArguments, config: &mut ProxyConfig) {
    if let Some(port) = arguments.port {
        config.set_port(port);
    }
    if let Some(compress) = arguments.compress {
        config.set_compress(compress);
    }

    if let Some(target_buffer_size) = arguments.target_buffer_size {
        config.set_target_buffer_size(target_buffer_size)
    }
    if let Some(message_framed_buffer_size) = arguments.message_framed_buffer_size {
        config.set_message_framed_buffer_size(message_framed_buffer_size)
    }
    if let Some(ref rsa_root_dir) = arguments.rsa_root_dir {
        config.set_rsa_root_dir(rsa_root_dir.to_string())
    }

    if let Some(so_backlog) = arguments.so_backlog {
        config.set_so_backlog(so_backlog)
    }
}

fn main() -> Result<()> {
    let arguments = ProxyArguments::parse();
    let mut log_configuration_file = std::fs::File::open(PROXY_LOG_CONFIG_FILE).expect("Fail to read proxy log configuration file.");
    let mut log_configuration_file_content = String::new();
    if let Err(e) = log_configuration_file.read_to_string(&mut log_configuration_file_content) {
        eprintln!("Fail to read proxy server log configuration file because of error: {:#?}", e);
        return Err(anyhow!(e));
    };
    let mut log_configuration = toml::from_str::<ProxyLogConfig>(&log_configuration_file_content).expect("Fail to parse proxy log configuration file");

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

    loop {
        let (proxy_server_signal_sender, proxy_server_signal_receiver) = channel();
        let proxy_server_signal_sender_for_watch_configuration = proxy_server_signal_sender.clone();
        let proxy_server_signal_sender_for_watch_rsa = proxy_server_signal_sender.clone();

        let mut configuration_file_watch = match Hotwatch::new() {
            Err(e) => {
                eprintln!("Fail to start proxy server configuration file watch because of error: {:#?}", e);

                return Err(anyhow!(e));
            },
            Ok(v) => v,
        };
        let configuration_file_path = match arguments.configuration_file {
            None => {
                println!("Starting ppaass-proxy with default configuration file:  ppaass-proxy.toml");
                Path::new("ppaass-proxy.toml")
            },
            Some(ref path) => {
                println!("Starting ppaass-proxy with customized configuration file: {}", path.as_str());
                Path::new(path)
            },
        };
        if let Err(e) = configuration_file_watch.watch(configuration_file_path, move |event| {
            info!("Event happen on watching file: {:?}", event);
            if let Event::NoticeWrite(_) = event {
                return;
            }
            if let Event::Remove(_) = event {
                return;
            }
            if let Err(e) = proxy_server_signal_sender_for_watch_configuration.send(ProxyServerSignal::Shutdown) {
                eprintln!("Fail to notice proxy server shutdown because of error: {:#?}", e);
            };
        }) {
            eprintln!("Fail to start proxy server configuration file watch because of error: {:#?}", e);
            return Err(anyhow!(e));
        }
        let configuration_file_content = std::fs::read_to_string(configuration_file_path).expect("Fail to read proxy configuration file.");
        let mut configuration = toml::from_str::<ProxyConfig>(&configuration_file_content).expect("Fail to parse proxy configuration file");
        merge_arguments_and_config(&arguments, &mut configuration);
        let rsa_dir_path = configuration.rsa_root_dir().as_ref().expect("Fail to read rsa root directory.");
        let mut rsa_folder_watch = match Hotwatch::new() {
            Err(e) => {
                eprintln!("Fail to start proxy server rsa folder watch because of error: {:#?}", e);
                return Err(anyhow!(e));
            },
            Ok(v) => v,
        };
        if let Err(e) = rsa_folder_watch.watch(rsa_dir_path, move |event| {
            info!("Event happen on watching dir:{:?}", event);
            if let Err(e) = proxy_server_signal_sender_for_watch_rsa.send(ProxyServerSignal::Shutdown) {
                eprintln!("Fail to notice proxy server shutdown because of error: {:#?}", e);
            };
        }) {
            eprintln!("Fail to start proxy server rsa folder watch because of error: {:#?}", e);
            return Err(anyhow!(e));
        }

        if let Err(e) = proxy_server_signal_sender.send(ProxyServerSignal::Startup { configuration }) {
            eprintln!("Fail to send startup single to proxy server because of error: {:#?}", e);
            return Err(anyhow!(e));
        };
        let proxy_server = ProxyServer::new(proxy_server_signal_receiver)?;
        let proxy_server_guard = proxy_server.run()?;
        proxy_server_guard.join().map_err(|_e| anyhow!("Error happen in proxy server."))?;
    }
}
