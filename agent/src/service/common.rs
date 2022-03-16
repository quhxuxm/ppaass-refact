use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::TryFutureExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::error;

use common::CommonError;

use crate::service::http::flow::{HttpFlowRequest, HttpFlowResult, HttpFlowService};
use crate::service::socks5::flow::{Socks5FlowRequest, Socks5FlowResult, Socks5FlowService};

#[derive(Debug)]
pub(crate) struct ClientConnectionInfo {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}

pub(crate) struct HandleClientConnectionService {
    socks5_flow_service: BoxCloneService<Socks5FlowRequest, Socks5FlowResult, CommonError>,
    http_flow_service: BoxCloneService<HttpFlowRequest, HttpFlowResult, CommonError>,
}

impl HandleClientConnectionService {
    pub fn new() -> Self {
        Self {
            socks5_flow_service: BoxCloneService::new(Socks5FlowService::new()),
            http_flow_service: BoxCloneService::new(HttpFlowService::new()),
        }
    }
}

impl Service<ClientConnectionInfo> for HandleClientConnectionService {
    type Response = ();
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ClientConnectionInfo) -> Self::Future {
        let mut socks5_flow_service = self.socks5_flow_service.clone();
        let mut http_flow_service = self.http_flow_service.clone();
        Box::pin(async move {
            let mut protocol_buf: [u8; 1] = [0];
            let peek_result = req.client_stream.peek(&mut protocol_buf).await;
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
                let mut flow_result = socks5_flow_service
                    .ready()
                    .await?
                    .call(Socks5FlowRequest {
                        client_stream: req.client_stream,
                        client_address: req.client_address,
                    })
                    .await?;
                flow_result.client_stream.shutdown().await?;
                return Ok(());
            }
            let mut client_connection = http_flow_service
                .ready()
                .await?
                .call(HttpFlowRequest {
                    client_stream: req.client_stream,
                    client_address: req.client_address,
                })
                .await?;
            client_connection.client_stream.shutdown().await?;
            return Ok(());
        })
    }
}

pub(crate) struct ConnectToProxyRequest;

pub(crate) struct ConnectToProxyService;

impl Service<ConnectToProxyRequest> for ConnectToProxyService {
    type Response = TcpStream;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<TcpStream, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ConnectToProxyRequest) -> Self::Future {
        Box::pin(TcpStream::connect("").map_err(|e| CommonError::IoError { source: e }))
    }
}
