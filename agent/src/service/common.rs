use std::task::{Context, Poll};
use std::{future::Future, pin::Pin};
use std::{net::SocketAddr, process::Output};

use futures_util::future::BoxFuture;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tower::util::{BoxCloneService, BoxService};
use tower::{Service, ServiceExt};
use tracing::error;

use common::CommonError;

use crate::service::http::flow::HttpFlowService;
use crate::service::socks5::flow::Socks5FlowService;

#[derive(Debug)]
pub(crate) struct ClientConnection {
    pub stream: TcpStream,
    pub client_address: SocketAddr,
}

impl ClientConnection {
    pub(crate) fn new(stream: TcpStream, client_address: SocketAddr) -> Self {
        Self {
            stream,
            client_address,
        }
    }
}

pub(crate) struct HandleClientConnectionService {
    socks5_flow_service: BoxCloneService<ClientConnection, ClientConnection, CommonError>,
    http_flow_service: BoxCloneService<ClientConnection, ClientConnection, CommonError>,
}

impl HandleClientConnectionService {
    pub fn new() -> Self {
        Self {
            socks5_flow_service: BoxCloneService::new(Socks5FlowService::new()),
            http_flow_service: BoxCloneService::new(HttpFlowService::new()),
        }
    }
}

impl Service<ClientConnection> for HandleClientConnectionService {
    type Response = ();
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ClientConnection) -> Self::Future {
        let mut socks5_flow_service = self.socks5_flow_service.clone();
        let mut http_flow_service = self.http_flow_service.clone();
        Box::pin(async move {
            let mut protocol_buf: [u8; 1] = [0];
            let peek_result = req.stream.peek(&mut protocol_buf).await;
            let protocol = match peek_result {
                Err(e) => {
                    error!(
                        "Fail to peek protocol from client stream because of error: {:#?}",
                        e
                    );
                    return Err(CommonError::IoError { source: e });
                }
                Ok(1) => {
                    let protocol = protocol_buf[0];
                    protocol
                }
                Ok(_) => {
                    return Err(CommonError::CodecError);
                }
            };
            if protocol == 4 {
                return Err(CommonError::CodecError);
            }
            if protocol == 5 {
                let mut client_connection = socks5_flow_service.ready().await?.call(req).await?;
                client_connection.stream.shutdown().await?;
                return Ok(());
            }
            let mut client_connection = http_flow_service.ready().await?.call(req).await?;
            client_connection.stream.shutdown().await?;
            return Ok(());
        })
    }
}
