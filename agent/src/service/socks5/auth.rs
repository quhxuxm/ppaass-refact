use std::io::ErrorKind;
use std::net::SocketAddr;

use anyhow::anyhow;
use anyhow::Result;
use bytes::BytesMut;
use common::PpaassError;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, FramedParts};
use tracing::{debug, instrument};

use crate::codec::socks5::Socks5AuthCommandContentCodec;
use crate::message::socks5::{Socks5AuthCommandResultContent, Socks5AuthMethod};

#[allow(unused)]
#[derive(Debug)]
pub(crate) struct Socks5AuthenticateFlowRequest<'a> {
    pub client_connection_id: &'a str,
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub buffer: BytesMut,
}

#[allow(unused)]
pub(crate) struct Socks5AuthenticateFlowResult {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub auth_method: Socks5AuthMethod,
    pub buffer: BytesMut,
}

pub(crate) struct Socks5AuthenticateFlow;

impl Socks5AuthenticateFlow {
    #[instrument(skip_all, fields(request.client_connection_id))]
    pub async fn exec<'a>(request: Socks5AuthenticateFlowRequest<'a>) -> Result<Socks5AuthenticateFlowResult> {
        let Socks5AuthenticateFlowRequest {
            client_connection_id,
            mut client_stream,
            client_address,
            buffer,
        } = request;
        let mut framed_parts = FramedParts::new(&mut client_stream, Socks5AuthCommandContentCodec);
        framed_parts.read_buf = buffer;
        let mut framed = Framed::from_parts(framed_parts);
        let authenticate_command = match framed.next().await {
            None => {
                let authentication_result = Socks5AuthCommandResultContent::new(Socks5AuthMethod::NoAcceptableMethods);
                framed.send(authentication_result).await?;
                framed.flush().await?;
                return Err(anyhow!(PpaassError::IoError {
                    source: std::io::Error::new(ErrorKind::InvalidData, "No authenticate frame.",),
                }));
            },
            Some(v) => match v {
                Ok(v) => v,
                Err(e) => {
                    let authentication_result = Socks5AuthCommandResultContent::new(Socks5AuthMethod::NoAcceptableMethods);
                    framed.send(authentication_result).await?;
                    framed.flush().await?;
                    return Err(anyhow!(e));
                },
            },
        };
        debug!(
            "Client connection [{}] start socks 5 authenticate for client[{}]: {:#?}",
            client_connection_id, request.client_address, authenticate_command
        );
        let authentication_result = Socks5AuthCommandResultContent::new(Socks5AuthMethod::NoAuthenticationRequired);
        framed.send(authentication_result).await?;
        framed.flush().await?;
        let FramedParts { read_buf, .. } = framed.into_parts();
        Ok(Socks5AuthenticateFlowResult {
            client_stream,
            client_address,
            auth_method: Socks5AuthMethod::NoAuthenticationRequired,
            buffer: read_buf,
        })
    }
}
