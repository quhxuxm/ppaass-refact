use std::fmt::{Debug, Formatter};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;
use tracing::{debug, error};

use crate::{
    generate_uuid, CommonError, Message, MessageCodec, MessagePayload, PayloadEncryptionType,
};

pub type MessageFramedRead = SplitStream<Framed<TcpStream, MessageCodec>>;
pub type MessageFramedWrite = SplitSink<Framed<TcpStream, MessageCodec>, Message>;

pub struct PrepareMessageFramedResult {
    pub message_framed_write: MessageFramedWrite,
    pub message_framed_read: MessageFramedRead,
}

#[derive(Clone)]
pub struct PrepareMessageFramedService<'a> {
    public_key: &'a str,
    private_key: &'a str,
    max_frame_size: usize,
    buffer_size: usize,
    compress: bool,
}

impl<'a> PrepareMessageFramedService<'a> {
    pub fn new(
        public_key: &'a str,
        private_key: &'a str,
        max_frame_size: usize,
        buffer_size: usize,
        compress: bool,
    ) -> Self {
        Self {
            public_key,
            private_key,
            max_frame_size,
            buffer_size,
            compress,
        }
    }
}

impl<'a> Service<TcpStream> for PrepareMessageFramedService<'a> {
    type Response = PrepareMessageFramedResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, input_stream: TcpStream) -> Self::Future {
        let framed = Framed::with_capacity(
            input_stream,
            MessageCodec::new(
                &(*self.public_key),
                &(*self.private_key),
                self.max_frame_size,
                self.compress,
            ),
            self.buffer_size,
        );
        let (sink, stream) = framed.split();
        Box::pin(async move {
            Ok(PrepareMessageFramedResult {
                message_framed_write: sink,
                message_framed_read: stream,
            })
        })
    }
}

pub struct WriteMessageServiceRequest {
    pub message_framed_write: MessageFramedWrite,
    pub message_payload: Option<MessagePayload>,
    pub ref_id: Option<String>,
    pub user_token: String,
    pub payload_encryption_type: PayloadEncryptionType,
}

impl Debug for WriteMessageServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WriteMessageServiceRequest: ref_id={}, user_token={}, payload_encryption_type={:#?}",
            self.ref_id.as_ref().unwrap_or(&"".to_string()),
            self.user_token,
            self.payload_encryption_type
        )
    }
}

pub struct WriteMessageServiceResult {
    pub message_framed_write: MessageFramedWrite,
}

#[derive(Clone, Default)]
pub struct WriteMessageService;

impl Service<WriteMessageServiceRequest> for WriteMessageService {
    type Response = WriteMessageServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: WriteMessageServiceRequest) -> Self::Future {
        Box::pin(async move {
            let message = match req.message_payload {
                None => Message::new(
                    generate_uuid(),
                    req.ref_id,
                    req.user_token,
                    req.payload_encryption_type,
                    None,
                ),
                Some(payload) => Message::new(
                    generate_uuid(),
                    req.ref_id,
                    req.user_token,
                    req.payload_encryption_type,
                    Some(payload.into()),
                ),
            };
            let mut message_frame_write = req.message_framed_write;
            if let Err(e) = message_frame_write.send(message).await {
                error!("Fail to write message because of error: {:#?}", e);
                let _ = message_frame_write.close().await;
                return Err(e);
            }
            if let Err(e) = message_frame_write.flush().await {
                error!("Fail to flash message because of error: {:#?}", e);
                let _ = message_frame_write.close().await;
                return Err(e);
            }
            Ok(WriteMessageServiceResult {
                message_framed_write: message_frame_write,
            })
        })
    }
}

pub struct ReadMessageServiceRequest {
    pub message_framed_read: MessageFramedRead,
}

impl Debug for ReadMessageServiceRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReadMessageServiceRequest")
    }
}

pub struct ReadMessageServiceResult {
    pub message_payload: MessagePayload,
    pub message_framed_read: MessageFramedRead,
    pub user_token: String,
    pub message_id: String,
}

#[derive(Clone)]
pub struct ReadMessageService {
    pub read_timeout_seconds: u64,
}

impl ReadMessageService {
    pub fn new(read_timeout_seconds: u64) -> Self {
        Self {
            read_timeout_seconds,
        }
    }
}

impl Service<ReadMessageServiceRequest> for ReadMessageService {
    type Response = Option<ReadMessageServiceResult>;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: ReadMessageServiceRequest) -> Self::Future {
        let read_timeout_seconds = self.read_timeout_seconds;
        Box::pin(async move {
            let message = match tokio::time::timeout(
                Duration::from_secs(read_timeout_seconds),
                req.message_framed_read.next(),
            )
            .await
            {
                Err(e) => {
                    error!("The read timeout in {} seconds.", e);
                    return Err(CommonError::TimeoutError);
                },
                Ok(None) => {
                    debug!("No message any more.");
                    return Ok(None);
                },
                Ok(Some(Ok(v))) => v,
                Ok(Some(Err(e))) => {
                    error!("Fail to decode message because of error: {:#?}", e);
                    return Err(e);
                },
            };
            let payload: MessagePayload = match message.payload {
                None => {
                    debug!("No payload in the message.",);
                    return Ok(None);
                },
                Some(payload_bytes) => match payload_bytes.try_into() {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Fail to decode message payload because of error: {:#?}", e);
                        return Err(e);
                    },
                },
            };
            Ok(Some(ReadMessageServiceResult {
                message_payload: payload,
                message_framed_read: req.message_framed_read,
                user_token: message.user_token,
                message_id: message.id,
            }))
        })
    }
}

#[derive(Debug)]
pub struct PayloadEncryptionTypeSelectServiceRequest {
    pub user_token: String,
    pub encryption_token: Bytes,
}

pub struct PayloadEncryptionTypeSelectServiceResult {
    pub user_token: String,
    pub encryption_token: Bytes,
    pub payload_encryption_type: PayloadEncryptionType,
}

pub struct PayloadEncryptionTypeSelectService;

impl Service<PayloadEncryptionTypeSelectServiceRequest> for PayloadEncryptionTypeSelectService {
    type Response = PayloadEncryptionTypeSelectServiceResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: PayloadEncryptionTypeSelectServiceRequest) -> Self::Future {
        Box::pin(async move {
            Ok(PayloadEncryptionTypeSelectServiceResult {
                payload_encryption_type: PayloadEncryptionType::Blowfish(
                    req.encryption_token.clone(),
                ),
                user_token: req.user_token.clone(),
                encryption_token: req.encryption_token.clone(),
            })
        })
    }
}
