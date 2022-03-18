use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tower::Service;

use common::CommonError;

pub(crate) struct TcpCloseServiceRequest;
pub(crate) struct TcpCloseServiceResult;
#[derive(Clone)]
pub(crate) struct TcpCloseService;
impl Service<TcpCloseServiceRequest> for TcpCloseService {
    type Response = TcpCloseServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: TcpCloseServiceRequest) -> Self::Future {
        todo!()
    }
}
