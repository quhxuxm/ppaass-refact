use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use tokio::net::{TcpListener, TcpStream};
use tower::{service_fn, BoxError, Service, ServiceExt};
use tracing::{error, info};

use common::CommonError;
use common::CommonError::CodecError;

use crate::protocol::socks::command::{Socks5AuthMethod, Socks5AuthRequest, Socks5AuthResponse};

pub(crate) mod protocol {
    pub(crate) mod http;
    pub(crate) mod socks;
}

pub(crate) mod codec {
    pub(crate) mod socks;
}

const SOCKS5_PROTOCOL_FLAG: u8 = 5;
const SOCKS4_PROTOCOL_FLAG: u8 = 4;

async fn handle_client_accept(
    client: (TcpStream, SocketAddr),
) -> Result<(TcpStream, SocketAddr), BoxError> {
    println!("Call handle client accept for: {}", client.1);
    Ok(client)
}

#[tokio::main]
async fn main() {
    let listener =
        match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 10081)).await {
            Err(e) => {
                panic!("Fail to bind agent because of error: {:#?}", e);
            }
            Ok(listener) => {
                info!("Success to bind agent, start listening ... ");
                listener
            }
        };
    let mut client_accept_service = service_fn(handle_client_accept);
    let client_accept_service = client_accept_service.ready();
    let client_accept_service = match client_accept_service.await {
        Err(e) => {
            panic!(
                "Fail to create client accept service because of error: {:#?}",
                e
            );
        }
        Ok(s) => s,
    };
    loop {
        let (client_stream, client_address) = match listener.accept().await {
            Err(e) => {
                error!(
                    "Fail to accept client connection because of error: {:#?}",
                    e
                );
                continue;
            }
            Ok((client_stream, client_address)) => (client_stream, client_address),
        };
        let (client_stream, client_address) = match client_accept_service
            .call((client_stream, client_address))
            .await
        {
            Err(e) => {
                error!(
                    "Fail to accept client connection [{}] because of error: {:#?}.",
                    client_address, e
                );
                continue;
            }
            Ok(client) => client,
        };

        let mut protocol_switch_buf: [u8; 1] = [0];
        let protocol_flag = match client_stream.peek(&mut protocol_switch_buf).await {
            Err(e) => {
                error!(
                    "Fail to switch client connection [{}] protocol because of error: {:#?}",
                    client_address, e
                );
                continue;
            }
            Ok(1) => protocol_switch_buf[0],
            Ok(_) => {
                error!("Fail to switch client connection protocol because of no enough data");
                continue;
            }
        };
        match protocol_flag {
            SOCKS4_PROTOCOL_FLAG => {
                error!("Do not support socks 4 protocol.");
                continue;
            }
            SOCKS5_PROTOCOL_FLAG => {
                println!(
                    "Handle socks 5 request for client connection [{}].",
                    client_address
                );

                continue;
            }
            _ => {
                println!(
                    "Handle http request for client connection [{}].",
                    client_address
                );
                continue;
            }
        }
    }
}
