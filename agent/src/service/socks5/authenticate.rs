use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt};
use tokio_util::codec::Framed;
use tower::Service;

use common::CommonError;

use crate::codec::socks5::Socks5AuthCodec;
use crate::command::socks5::{Socks5AuthCommand, Socks5AuthCommandResult, Socks5AuthMethod};
use crate::service::common::ClientConnection;

#[derive(Clone)]
pub(crate) struct Socks5AuthCommandService;

impl Service<ClientConnection> for Socks5AuthCommandService {
    type Response = ClientConnection;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: ClientConnection) -> Self::Future {
        println!("Socks5 auth command: {:#?}", request);
        let ClientConnection {
            mut stream,
            client_address,
        } = request;
        let mut framed = Framed::with_capacity(&mut stream, Socks5AuthCodec, 2048);

        let runtime = tokio::runtime::Handle::current();
        let current = runtime.enter();
        let authenticate_command = framed.poll_next(current);
        todo!()
    }
}
