use std::net::SocketAddr;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::util::BoxCloneService;
use tower::{Service, ServiceExt};
use tracing::{debug, error};

use common::{
    generate_uuid, AgentMessagePayloadTypeValue, CommonError, MessageFramedRead,
    MessageFramedWrite, MessagePayload, NetAddress, PayloadEncryptionType, PayloadType,
    ReadMessageService, ReadMessageServiceRequest, ReadMessageServiceResult, WriteMessageService,
    WriteMessageServiceRequest, WriteMessageServiceResult,
};

use crate::SERVER_CONFIG;

pub(crate) struct Socks5RelayServiceRequest {
    pub client_stream: TcpStream,
    pub client_address: SocketAddr,
    pub proxy_address_string: String,
    pub message_framed_write: MessageFramedWrite,
    pub message_framed_read: MessageFramedRead,
    pub connect_response_message_id: String,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
}

pub(crate) struct Socks5RelayServiceResult {
    pub client_address: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Socks5RelayService {
    write_agent_message_service:
        BoxCloneService<WriteMessageServiceRequest, WriteMessageServiceResult, CommonError>,
    read_proxy_message_service:
        BoxCloneService<ReadMessageServiceRequest, Option<ReadMessageServiceResult>, CommonError>,
}

impl Default for Socks5RelayService {
    fn default() -> Self {
        Self {
            write_agent_message_service: BoxCloneService::new(WriteMessageService),
            read_proxy_message_service: BoxCloneService::new(ReadMessageService),
        }
    }
}

impl Service<Socks5RelayServiceRequest> for Socks5RelayService {
    type Response = Socks5RelayServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Socks5RelayServiceRequest) -> Self::Future {
        let mut write_agent_message_service = self.write_agent_message_service.clone();
        let mut read_proxy_message_service = self.read_proxy_message_service.clone();
        Box::pin(async move {
            let client_stream = request.client_stream;
            let mut message_framed_read = request.message_framed_read;
            let mut message_framed_write = request.message_framed_write;
            let (mut client_stream_read_half, mut client_stream_write_half) =
                client_stream.into_split();
            let source_address_a2t = request.source_address.clone();
            let target_address_a2t = request.target_address.clone();

            let target_address_t2a = request.target_address.clone();

            tokio::spawn(async move {
                loop {
                    let mut buf =
                        BytesMut::with_capacity(SERVER_CONFIG.buffer_size().unwrap_or(1024 * 64));
                    match client_stream_read_half.read_buf(&mut buf).await {
                        Err(e) => {
                            error!(
                            "Client {:#?}, fail to read client data from {:#?} because of error: {:#?}",
                             request.client_address, target_address_a2t, e
                        );
                            return;
                        }
                        Ok(0) => {
                            return;
                        }
                        Ok(_) => {}
                    }
                    let check_ready_result = write_agent_message_service.ready().await;
                    let service_obj = match check_ready_result {
                        Err(e) => {
                            error!(
                            "Client {:#?}, fail to read client data from {:#?} because of error (write service not ready): {:#?}",
                             request.client_address, target_address_a2t, e);
                            return;
                        }
                        Ok(v) => v,
                    };
                    let agent_message_payload = MessagePayload::new(
                        source_address_a2t.clone(),
                        target_address_a2t.clone(),
                        PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData),
                        buf.freeze(),
                    );
                    match service_obj
                        .call(WriteMessageServiceRequest {
                            message_framed_write,
                            ref_id: Some(request.connect_response_message_id.clone()),
                            user_token: SERVER_CONFIG.user_token().clone().unwrap(),
                            payload_encryption_type: PayloadEncryptionType::Blowfish(
                                generate_uuid().into(),
                            ),
                            message_payload: Some(agent_message_payload),
                        })
                        .await
                    {
                        Err(e) => {
                            error!(
                            "Client {:#?}, fail to read client data from {:#?} because of error (write service error): {:#?}",
                             request.client_address, target_address_a2t, e);
                            return;
                        }
                        Ok(agent_message_write_result) => {
                            message_framed_write = agent_message_write_result.message_framed_write;
                        }
                    }
                }
            });
            tokio::spawn(async move {
                loop {
                    let check_ready_result = read_proxy_message_service.ready().await;
                    let service_obj = match check_ready_result {
                        Err(e) => {
                            error!(
                            "Client {:#?}, fail to read proxy data from {:#?} because of error (read service not ready): {:#?}",
                             request.client_address, target_address_t2a, e);
                            return;
                        }
                        Ok(v) => v,
                    };
                    match service_obj
                        .call(ReadMessageServiceRequest {
                            message_framed_read,
                        })
                        .await
                    {
                        Err(e) => {
                            error!(
                            "Client {:#?}, fail to read proxy data from {:#?} because of error (read service error): {:#?}",
                             request.client_address, target_address_t2a, e);
                            return;
                        }
                        Ok(None) => {
                            return;
                        }
                        Ok(Some(proxy_message_read_result)) => {
                            message_framed_read = proxy_message_read_result.message_framed_read;
                            let proxy_message_payload = proxy_message_read_result.message_payload;
                            if let Err(e) = client_stream_write_half
                                .write(proxy_message_payload.data.as_ref())
                                .await
                            {
                                error!(
                                "Client {:#?}, fail to write proxy data from {:#?} to client because of error (read service error): {:#?}",
                                 request.client_address, target_address_t2a, e);
                                return;
                            };
                            if let Err(e) = client_stream_write_half.flush().await {
                                error!(
                                "Client {:#?}, fail to flush proxy data from {:#?} to client because of error (read service error): {:#?}",
                                 request.client_address, target_address_t2a, e);
                                return;
                            };
                        }
                    }
                }
            });
            Ok(Socks5RelayServiceResult {
                client_address: request.client_address,
            })
        })
    }
}
