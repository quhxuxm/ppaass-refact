use std::fmt::{Debug, Formatter};
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use anyhow::anyhow;
use bytes::BytesMut;
use common::PpaassError;
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};
use tower::Service;
use tracing::debug;

use crate::codec::socks5::Socks5AuthCommandContentCodec;
use crate::message::socks5::{Socks5AuthCommandResultContent, Socks5AuthMethod};

#[allow(unused)]
pub(crate) struct Socks5AuthenticateFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

impl Debug for Socks5AuthenticateFlowRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Socks5AuthenticateFlowRequest: client_address={}", self.client_address)
    }
}

#[allow(unused)]
pub(crate) struct Socks5AuthenticateFlowResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub auth_method: Socks5AuthMethod,
    pub buffer: BytesMut,
}

#[derive(Clone, Default)]
pub(crate) struct Socks5AuthCommandService;

impl Debug for Socks5AuthCommandService {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Socks5AuthCommandService")
    }
}

impl Service<Socks5AuthenticateFlowRequest> for Socks5AuthCommandService {
    type Response = Socks5AuthenticateFlowResult;
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5AuthenticateFlowRequest) -> Self::Future {
        Box::pin(async move {
            let mut framed_parts =
                FramedParts::new(&mut request.client_stream, Socks5AuthCommandContentCodec);
            framed_parts.read_buf = request.buffer;
            let mut framed = Framed::from_parts(framed_parts);
            let authenticate_command = match framed.next().await {
                None => {
                    let authentication_result =
                        Socks5AuthCommandResultContent::new(Socks5AuthMethod::NoAcceptableMethods);
                    framed.send(authentication_result).await?;
                    framed.flush().await?;
                    return Err(anyhow!(PpaassError::IoError {
                        source: std::io::Error::new(
                            ErrorKind::InvalidData,
                            "No authenticate frame.",
                        ),
                    }));
                },
                Some(v) => match v {
                    Ok(v) => v,
                    Err(e) => {
                        let authentication_result = Socks5AuthCommandResultContent::new(
                            Socks5AuthMethod::NoAcceptableMethods,
                        );
                        framed.send(authentication_result).await?;
                        framed.flush().await?;
                        return Err(anyhow!(e));
                    },
                },
            };
            debug!(
                "Client {} start socks 5 authenticate: {:#?}",
                request.client_address, authenticate_command
            );
            let authentication_result =
                Socks5AuthCommandResultContent::new(Socks5AuthMethod::NoAuthenticationRequired);
            framed.send(authentication_result).await?;
            framed.flush().await?;
            let FramedParts { read_buf, .. } = framed.into_parts();
            Ok(Socks5AuthenticateFlowResult {
                client_stream: request.client_stream,
                client_address: request.client_address,
                auth_method: Socks5AuthMethod::NoAuthenticationRequired,
                buffer: read_buf,
            })
        })
    }
}
