use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tower::Service;

use common::CommonError;

pub(crate) struct ConnectServiceRequest;
pub(crate) struct ConnectServiceResult;

pub(crate) struct ConnectService;

impl ConnectService {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Service<ConnectServiceRequest> for ConnectService {
    type Response = ConnectServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: ConnectServiceRequest) -> Self::Future {
        todo!()
    }
}
