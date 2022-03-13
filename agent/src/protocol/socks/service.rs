use std::task::{Context, Poll};

use tower::Service;

use crate::protocol::socks::command::{
    Socks5ConnectRequest, Socks5ConnectResponse, Socks5ConnectResponseStatus,
};
use crate::{Socks5AuthMethod, Socks5AuthRequest, Socks5AuthResponse};

pub(crate) struct Socks5AuthCommandService;

impl Service<Socks5AuthRequest> for Socks5AuthCommandService {
    type Response = Socks5AuthResponse;
    type Error = ();
    type Future = futures_util::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5AuthRequest) -> Self::Future {
        println!("Socks5 auth command: {:#?}", request);
        futures_util::future::ok(Socks5AuthResponse::new(
            Socks5AuthMethod::NoAuthenticationRequired,
        ))
    }
}

pub(crate) struct Socks5ConnectCommandService;

impl Service<Socks5ConnectRequest> for Socks5ConnectCommandService {
    type Response = Socks5ConnectResponse;
    type Error = ();
    type Future = futures_util::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5ConnectRequest) -> Self::Future {
        println!("Socks5 connect command: {:#?}", request);
        futures_util::future::ok(Socks5ConnectResponse::new(
            Socks5ConnectResponseStatus::Succeeded,
            None,
        ))
    }
}
