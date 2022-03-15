use std::task::{Context, Poll};

use tower::Service;

use crate::command::socks5::{
    Socks5AuthCommand, Socks5AuthCommandResult, Socks5AuthMethod, Socks5ConnectCommand,
    Socks5ConnectCommandResult, Socks5ConnectCommandResultStatus,
};

pub(crate) struct Socks5AuthCommandService;

impl Service<Socks5AuthCommand> for Socks5AuthCommandService {
    type Response = Socks5AuthCommandResult;
    type Error = ();
    type Future = futures_util::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5AuthCommand) -> Self::Future {
        println!("Socks5 auth command: {:#?}", request);
        futures_util::future::ok(Socks5AuthCommandResult::new(
            Socks5AuthMethod::NoAuthenticationRequired,
        ))
    }
}

pub(crate) struct Socks5ConnectCommandService;

impl Service<Socks5ConnectCommand> for Socks5ConnectCommandService {
    type Response = Socks5ConnectCommandResult;
    type Error = ();
    type Future = futures_util::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5ConnectCommand) -> Self::Future {
        println!("Socks5 connect command: {:#?}", request);
        futures_util::future::ok(Socks5ConnectCommandResult::new(
            Socks5ConnectCommandResultStatus::Succeeded,
            None,
        ))
    }
}
