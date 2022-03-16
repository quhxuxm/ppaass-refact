use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::io::AsyncWriteExt;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};

use common::CommonError;

use crate::service::common::ClientConnection;
use crate::service::socks5::authenticate::Socks5AuthCommandService;
use crate::service::socks5::connect::Socks5ConnectCommandService;
use crate::service::socks5::relay::Socks5RelayService;

#[derive(Clone)]
pub(crate) struct Socks5FlowService {
    authenticate_service: BoxCloneService<ClientConnection, ClientConnection, CommonError>,
    connect_service: BoxCloneService<ClientConnection, ClientConnection, CommonError>,
    relay_service: BoxCloneService<ClientConnection, ClientConnection, CommonError>,
}

impl Socks5FlowService {
    pub(crate) fn new() -> Self {
        Self {
            authenticate_service: BoxCloneService::new(Socks5AuthCommandService),
            connect_service: BoxCloneService::new(Socks5ConnectCommandService),
            relay_service: BoxCloneService::new(Socks5RelayService),
        }
    }
}

impl Service<ClientConnection> for Socks5FlowService {
    type Response = ClientConnection;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ClientConnection) -> Self::Future {
        println!("Handle socks 5 flow, client connection: {:#?}", req);
        let mut authenticate_service = self.authenticate_service.clone();
        let mut connect_service = self.connect_service.clone();
        let mut relay_service = self.relay_service.clone();
        Box::pin(async move {
            let client_connection = authenticate_service.ready().await?.call(req).await?;
            let client_connection = connect_service
                .ready()
                .await?
                .call(client_connection)
                .await?;
            let client_connection = relay_service.ready().await?.call(client_connection).await?;
            Ok(client_connection)
        })
    }
}
