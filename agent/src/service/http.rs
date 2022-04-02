use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio_tfo::TfoStream;
use tower::{Service, ServiceBuilder};

use common::{ready_and_call_service, CommonError};

use crate::service::http::connect::{HttpConnectService, HttpConnectServiceRequest};
use crate::service::http::relay::{HttpRelayService, HttpRelayServiceRequest};

mod connect;
mod relay;

pub(crate) struct HttpFlowRequest {
    pub client_stream: TfoStream,
    pub client_address: SocketAddr,
}
#[derive(Debug)]
#[allow(unused)]
pub(crate) struct HttpFlowResult {
    pub client_address: SocketAddr,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HttpFlowService;

impl Service<HttpFlowRequest> for HttpFlowService {
    type Response = HttpFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpFlowRequest) -> Self::Future {
        Box::pin(async move {
            let mut connect_service = ServiceBuilder::new().service(HttpConnectService::default());
            let mut relay_service = ServiceBuilder::new().service(HttpRelayService::default());
            let connect_result = ready_and_call_service(
                &mut connect_service,
                HttpConnectServiceRequest {
                    client_address: req.client_address,
                    client_stream: req.client_stream,
                },
            )
            .await?;
            let relay_result = ready_and_call_service(
                &mut relay_service,
                HttpRelayServiceRequest {
                    client_address: connect_result.client_address,
                    client_stream: connect_result.client_stream,
                    message_framed_write: connect_result.message_framed_write,
                    message_framed_read: connect_result.message_framed_read,
                    target_address: connect_result.target_address,
                    source_address: connect_result.source_address,
                    init_data: connect_result.init_data,
                    connect_response_message_id: connect_result.message_id,
                },
            )
            .await?;
            Ok(HttpFlowResult {
                client_address: relay_result.client_address,
            })
        })
    }
}
