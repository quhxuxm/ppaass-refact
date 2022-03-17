use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{future, TryFutureExt};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tower::retry::{Policy, Retry};
use tower::util::{BoxCloneService, BoxService};
use tower::{service_fn, Service, ServiceExt};
use tracing::error;

use common::CommonError;

use crate::config::{Config, SERVER_CONFIG};
use crate::service::http::flow::{HttpFlowRequest, HttpFlowResult, HttpFlowService};
use crate::service::socks5::flow::{Socks5FlowRequest, Socks5FlowResult, Socks5FlowService};

const SOCKS5_PROTOCOL_FLAG: u8 = 5;
const SOCKS4_PROTOCOL_FLAG: u8 = 4;
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
            if protocol == SOCKS4_PROTOCOL_FLAG {
                return Err(CommonError::CodecError);
            }
            if protocol == SOCKS5_PROTOCOL_FLAG {
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

#[derive(Clone)]
struct DoProxyConnectRequest {
    proxy_address: String,
}

pub(crate) struct ConnectToProxyResponse {
    pub proxy_stream: TcpStream,
    pub connected_proxy_address: String,
}

#[derive(Clone)]
struct ConnectToProxyAttempts {
    retry: u16,
}

pub(crate) struct ConnectToProxyService {
    concrete_service: BoxCloneService<DoProxyConnectRequest, ConnectToProxyResponse, CommonError>,
}

impl ConnectToProxyService {
    pub(crate) fn new(retry: u16) -> Self {
        let concrete_service = Retry::new(
            ConnectToProxyAttempts { retry },
            service_fn(|request: DoProxyConnectRequest| async move {
                println!("Begin connect to proxy: {}", request.proxy_address);
                let proxy_stream = TcpStream::connect(&request.proxy_address)
                    .await
                    .map_err(|e| CommonError::IoError { source: e })?;
                Ok(ConnectToProxyResponse {
                    proxy_stream,
                    connected_proxy_address: request.proxy_address,
                })
            }),
        );
        Self {
            concrete_service: BoxCloneService::new(concrete_service),
        }
    }
}

impl Policy<DoProxyConnectRequest, ConnectToProxyResponse, CommonError> for ConnectToProxyAttempts {
    type Future = futures_util::future::Ready<Self>;

    fn retry(
        &self,
        req: &DoProxyConnectRequest,
        result: Result<&ConnectToProxyResponse, &CommonError>,
    ) -> Option<Self::Future> {
        match result {
            Ok(_) => {
                // Treat all `Response`s as success,
                // so don't retry...
                None
            }
            Err(_) => {
                // Treat all errors as failures...
                // But we limit the number of attempts...
                if self.retry > 0 {
                    // Try again!
                    return Some(future::ready(ConnectToProxyAttempts {
                        retry: self.retry - 1,
                    }));
                }
                // Used all our attempts, no retry...
                None
            }
        }
    }

    fn clone_request(&self, req: &DoProxyConnectRequest) -> Option<DoProxyConnectRequest> {
        Some(req.clone())
    }
}

impl Service<()> for ConnectToProxyService {
    type Response = ConnectToProxyResponse;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<ConnectToProxyResponse, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ()) -> Self::Future {
        let proxy_addresses = SERVER_CONFIG
            .proxy_addresses()
            .as_ref()
            .expect("No proxy addresses configuration item")
            .clone();
        let mut concrete_connect_service = self.concrete_service.clone();
        let connect_future = async move {
            for address in proxy_addresses.into_iter() {
                let concrete_connect_result = concrete_connect_service
                    .ready()
                    .await?
                    .call(DoProxyConnectRequest {
                        proxy_address: address.clone(),
                    })
                    .await;
                match concrete_connect_result {
                    Ok(r) => return Ok(r),
                    Err(e) => {
                        error!(
                            "Fail to connect to proxy address: {} because of error: {:#?}",
                            address, e
                        );
                        continue;
                    }
                }
            }
            return Err(CommonError::IoError {
                source: std::io::Error::new(
                    ErrorKind::NotConnected,
                    "No proxy address is connnectable.",
                ),
            });
        };
        Box::pin(connect_future)
    }
}
