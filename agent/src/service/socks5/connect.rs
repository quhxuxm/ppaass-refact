use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tower::Service;

use common::CommonError;

use crate::command::socks5::{
    Socks5ConnectCommand, Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus,
};
use crate::service::common::ClientConnection;

#[derive(Clone)]
pub(crate) struct Socks5ConnectCommandService;

impl Service<ClientConnection> for Socks5ConnectCommandService {
    type Response = ClientConnection;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: ClientConnection) -> Self::Future {
        println!("Socks5 connect command: {:#?}", request);
        todo!()
    }
}
