use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;

use common::CommonError;

use crate::codec::socks5::Socks5AuthCodec;
use crate::command::socks5::{Socks5AuthCommandResult, Socks5AuthMethod};

pub(crate) struct Socks5AuthenticateFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

pub(crate) struct Socks5AuthenticateFlowResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub auth_method: Option<Socks5AuthMethod>,
}
#[derive(Clone)]
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
            let mut framed =
                Framed::with_capacity(&mut request.client_stream, Socks5AuthCodec, 2048);
            let authenticate_command = match framed.next().await {
                None => {
                    return Ok(Socks5AuthenticateFlowResult {
                        client_stream: request.client_stream,
                        auth_method: None,
                        client_address: request.client_address,
                    })
                }
                Some(v) => v?,
            };
            println!("Socks 5 authenticate: {:#?}", authenticate_command);
            let authentication_result =
                Socks5AuthCommandResult::new(Socks5AuthMethod::NoAuthenticationRequired);
            framed.send(authentication_result).await?;
            framed.flush().await?;
            Ok(Socks5AuthenticateFlowResult {
                client_stream: request.client_stream,
                client_address: request.client_address,
                auth_method: Some(Socks5AuthMethod::NoAuthenticationRequired),
            })
        })
    }
}
