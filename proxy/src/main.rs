extern crate core;

use std::{io::Read, path::Path, sync::mpsc::channel};

use crate::server::ProxyServer;
use anyhow::anyhow;
use anyhow::Result;
use clap::Parser;
use common::init_log;
use config::{ProxyArguments, ProxyConfig};
use hotwatch::{Event, Hotwatch};
use server::ProxyServerSignal;

use tracing::Level;

pub(crate) mod config;
pub(crate) mod server;
pub(crate) mod service;

fn merge_arguments_and_connfig(arguments: &ProxyArguments, config: &mut ProxyConfig) {
    if let Some(port) = arguments.port {
        config.set_port(port);
    }
    if let Some(compress) = arguments.compress {
        config.set_compress(compress);
    }
    if let Some(ref log_dir) = arguments.log_dir {
        config.set_log_dir(log_dir.to_string())
    }
    if let Some(ref log_file) = arguments.log_file {
        config.set_log_file(log_file.to_string())
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
    if let Some(ref max_log_level) = arguments.max_log_level {
        config.set_max_log_level(max_log_level.to_string())
    }
    if let Some(so_backlog) = arguments.so_backlog {
        config.set_so_backlog(so_backlog)
    }
}

fn main() -> Result<()> {
    loop {
        let (proxy_server_signal_sender, proxy_server_signal_receiver) = channel();
        let proxy_server_signal_sender_for_watch_configuration = proxy_server_signal_sender.clone();
        let proxy_server_signal_sender_for_watch_rsa = proxy_server_signal_sender.clone();
        let arguments = ProxyArguments::parse();
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
            if let Event::Write(_) = event {
                if let Err(e) = proxy_server_signal_sender_for_watch_configuration.send(ProxyServerSignal::Shutdown) {
                    eprintln!("Fail to notice proxy server shutdown because of error: {:#?}", e);
                };
            }
        }) {
            eprintln!("Fail to start proxy server configuration file watch because of error: {:#?}", e);

            return Err(anyhow!(e));
        }

        let mut configuration_file = std::fs::File::open(configuration_file_path).expect("Fail to read proxy configuration file.");
        let mut configuration_file_content = String::new();
        if let Err(e) = configuration_file.read_to_string(&mut configuration_file_content) {
            eprintln!("Fail to read proxy server configuration file because of error: {:#?}", e);

            return Err(anyhow!(e));
        };
        let mut configuration = toml::from_str::<ProxyConfig>(&configuration_file_content).expect("Fail to parse proxy configuration file");
        merge_arguments_and_connfig(&arguments, &mut configuration);

        let rsa_dir_path = configuration.rsa_root_dir().as_ref().expect("Fail to read rsa root directory.");
        let mut rsa_folder_watch = match Hotwatch::new() {
            Err(e) => {
                eprintln!("Fail to start proxy server rsa folder watch because of error: {:#?}", e);

                return Err(anyhow!(e));
            },
            Ok(v) => v,
        };
        if let Err(e) = rsa_folder_watch.watch(rsa_dir_path, move |_| {
            if let Err(e) = proxy_server_signal_sender_for_watch_rsa.send(ProxyServerSignal::Shutdown) {
                eprintln!("Fail to notice proxy server shutdown because of error: {:#?}", e);
            };
        }) {
            eprintln!("Fail to start proxy server rsa folder watch because of error: {:#?}", e);
            return Err(anyhow!(e));
        }
        let (_appender_guard, _subscriber_guard) = init_log(
            configuration.log_dir().as_ref().expect("No log directory given."),
            configuration.log_file().as_ref().expect("No log file name given."),
            configuration.max_log_level().as_ref().unwrap_or(&Level::ERROR.to_string()),
        );
        if let Err(e) = proxy_server_signal_sender.send(ProxyServerSignal::Startup { configuration }) {
            eprintln!("Fail to send startup single to proxy server because of error: {:#?}", e);
            return Err(anyhow!(e));
        };
        let proxy_server = ProxyServer::new(proxy_server_signal_receiver)?;
        let proxy_server_guard = proxy_server.run()?;
        proxy_server_guard.join().map_err(|_e| anyhow!("Error happen in proxy server."))?;
    }
}
