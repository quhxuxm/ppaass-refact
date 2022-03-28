use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::Service;

use common::{ready_and_call_service, CommonError};
use tracing::debug;

use crate::service::http::connect::{
    HttpConnectService, HttpConnectServiceRequest, HttpConnectServiceResult,
};
use crate::service::http::relay::{
    HttpRelayService, HttpRelayServiceRequest, HttpRelayServiceResult,
};

mod connect;
mod relay;

#[derive(Debug)]
pub(crate) struct HttpFlowRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
}
#[derive(Debug)]
pub(crate) struct HttpFlowResult {
    pub client_address: SocketAddr,
}

#[derive(Clone, Debug)]
pub(crate) struct HttpFlowService {
    connect_service:
        BoxCloneService<HttpConnectServiceRequest, HttpConnectServiceResult, CommonError>,
    relay_service: BoxCloneService<HttpRelayServiceRequest, HttpRelayServiceResult, CommonError>,
}

impl HttpFlowService {
    pub(crate) fn new() -> Self {
        Self {
            connect_service: BoxCloneService::new::<HttpConnectService>(Default::default()),
            relay_service: BoxCloneService::new::<HttpRelayService>(Default::default()),
        }
    }
}

impl Service<HttpFlowRequest> for HttpFlowService {
    type Response = HttpFlowResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let connect_service_ready = self.connect_service.poll_ready(cx)?;
        if connect_service_ready.is_ready() {
            debug!("Ready to handle client http connection.");
            return Poll::Ready(Ok(()));
        }
        debug!("Not ready to handle client http connection.");
        Poll::Pending
    }

    fn call(&mut self, req: HttpFlowRequest) -> Self::Future {
        let mut connect_service = self.connect_service.clone();
        let mut relay_service = self.relay_service.clone();
        Box::pin(async move {
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
