use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;

use common::CommonError;

use crate::codec::socks5::Socks5ConnectCodec;
use crate::command::socks5::{
    Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus, Socks5ConnectCommandType,
};

#[derive(Debug)]
pub(crate) struct Socks5ConnectFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

#[derive(Debug)]
pub(crate) struct Socks5ConnectFlowResult {
    pub client_stream: TcpStream,
    pub proxy_stream: Option<TcpStream>,
    pub client_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Socks5ConnectCommandService;

impl Service<Socks5ConnectFlowRequest> for Socks5ConnectCommandService {
    type Response = Socks5ConnectFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Socks5ConnectFlowRequest) -> Self::Future {
        Box::pin(async move {
            let mut framed =
                Framed::with_capacity(&mut request.client_stream, Socks5ConnectCodec, 2048);
            let connect_command = match framed.next().await {
                None => {
                    return Ok(Socks5ConnectFlowResult {
                        client_stream: request.client_stream,
                        client_address: request.client_address,
                        proxy_stream: None,
                    })
                }
                Some(v) => v?,
            };
            println!("Socks 5 connect: {:#?}", connect_command);
            let connect_command_type = connect_command.request_type;

            let proxy_stream = match connect_command_type {
                Socks5ConnectCommandType::Connect => {
                    TcpStream::connect(connect_command.dest_address.to_string()).await?
                }
                Socks5ConnectCommandType::Bind => {
                    todo!()
                }
                Socks5ConnectCommandType::UdpAssociate => {
                    todo!()
                }
            };
            let connect_result =
                Socks5ConnectCommandResult::new(Socks5ConnectCommandResultStatus::Succeeded);
            framed.send(connect_result).await?;
            Ok(Socks5ConnectFlowResult {
                client_stream: request.client_stream,
                client_address: request.client_address,
            })
        })
    }
}
