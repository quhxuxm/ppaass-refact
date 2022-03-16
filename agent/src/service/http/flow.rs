use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tower::Service;

use common::CommonError;

use crate::service::common::ClientConnection;

#[derive(Clone)]
pub(crate) struct HttpFlowService;

impl HttpFlowService {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Service<ClientConnection> for HttpFlowService {
    type Response = ClientConnection;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: ClientConnection) -> Self::Future {
        todo!()
    }
}
