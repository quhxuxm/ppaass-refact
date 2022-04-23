use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::{SinkExt, StreamExt};
use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;
use tracing::debug;

use common::CommonError;

use crate::codec::socks5::Socks5AuthCodec;
use crate::command::socks5::{Socks5AuthCommandResult, Socks5AuthMethod};
use crate::SERVER_CONFIG;
use crate::service::common::DEFAULT_BUFFER_SIZE;

#[allow(unused)]
pub(crate) struct Socks5AuthenticateFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

#[allow(unused)]
pub(crate) struct Socks5AuthenticateFlowResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub auth_method: Socks5AuthMethod,
}

#[derive(Clone, Default, Debug)]
pub(crate) struct Socks5AuthCommandService;

impl Service<Socks5AuthenticateFlowRequest> for Socks5AuthCommandService {
    type Response = Socks5AuthenticateFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5AuthenticateFlowRequest) -> Self::Future {
        Box::pin(async move {
            let mut framed = Framed::with_capacity(
                &mut request.client_stream,
                Socks5AuthCodec,
                SERVER_CONFIG.buffer_size().unwrap_or(DEFAULT_BUFFER_SIZE),
            );
            let authenticate_command = match framed.next().await {
                None => {
                    let authentication_result =
                        Socks5AuthCommandResult::new(Socks5AuthMethod::NoAcceptableMethods);
                    framed.send(authentication_result).await?;
                    framed.flush().await?;
                    return Err(CommonError::IoError {
                        source: std::io::Error::new(
                            ErrorKind::InvalidData,
                            "No authenticate frame.",
                        ),
                    });
                },
                Some(v) => match v {
                    Ok(v) => v,
                    Err(e) => {
                        let authentication_result =
                            Socks5AuthCommandResult::new(Socks5AuthMethod::NoAcceptableMethods);
                        framed.send(authentication_result).await?;
                        framed.flush().await?;
                        return Err(e);
                    },
                },
            };
            debug!(
                "Client {} start socks 5 authenticate: {:#?}",
                request.client_address, authenticate_command
            );
            let authentication_result =
                Socks5AuthCommandResult::new(Socks5AuthMethod::NoAuthenticationRequired);
            framed.send(authentication_result).await?;
            framed.flush().await?;
            Ok(Socks5AuthenticateFlowResult {
                client_stream: request.client_stream,
                client_address: request.client_address,
                auth_method: Socks5AuthMethod::NoAuthenticationRequired,
            })
        })
    }
}
