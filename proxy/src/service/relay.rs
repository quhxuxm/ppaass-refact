use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tower::Service;

use common::CommonError;

pub(crate) struct RelayServiceRequest;
pub(crate) struct RelayServiceResult;

pub(crate) struct RelayService;

impl RelayService {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Service<RelayServiceRequest> for RelayService {
    type Response = RelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: RelayServiceRequest) -> Self::Future {
        todo!()
    }
}
