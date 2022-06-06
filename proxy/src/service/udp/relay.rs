#![allow(unused)]

use std::task::{Context, Poll};

use futures::future::BoxFuture;
use tower::Service;

use common::PpaassError;

pub(crate) struct UdpRelayServiceRequest;
pub(crate) struct UdpRelayServiceResult;
#[derive(Clone)]
pub(crate) struct UdpRelayService;
impl Service<UdpRelayServiceRequest> for UdpRelayService {
    type Response = UdpRelayServiceResult;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: UdpRelayServiceRequest) -> Self::Future {
        todo!()
    }
}
