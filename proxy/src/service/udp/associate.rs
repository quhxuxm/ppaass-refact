#![allow(unused)]

use std::task::{Context, Poll};

use futures::future::BoxFuture;
use tower::Service;

use common::PpaassError;

pub(crate) struct UdpAssociateServiceRequest;
pub(crate) struct UdpAssociateServiceResult;
#[derive(Clone)]
pub(crate) struct UdpAssociateService;

impl Service<UdpAssociateServiceRequest> for UdpAssociateService {
    type Response = UdpAssociateServiceResult;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: UdpAssociateServiceRequest) -> Self::Future {
        todo!()
    }
}
