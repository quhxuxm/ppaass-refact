use std::fmt::{Debug, Formatter};
use std::time::Duration;
use std::{
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpStream, time::timeout};
use tokio_util::codec::Framed;
use tower::Service;
use tracing::{debug, error};

use crate::{
    crypto::RsaCryptoFetcher, generate_uuid, Message, MessageCodec, MessagePayload,
    PayloadEncryptionType, PpaassError,
};

pub type MessageFramedRead<T> = SplitStream<Framed<TcpStream, MessageCodec<T>>>;
pub type MessageFramedWrite<T> = SplitSink<Framed<TcpStream, MessageCodec<T>>, Message>;

pub struct PrepareMessageFramedResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
}

#[derive(Clone)]
pub struct PrepareMessageFramedService<T>
where
    T: RsaCryptoFetcher,
{
    max_frame_size: usize,
    buffer_size: usize,
    compress: bool,
    rsa_crypto_fetcher: Arc<T>,
}

impl<T> PrepareMessageFramedService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(
        max_frame_size: usize, buffer_size: usize, compress: bool, rsa_crypto_fetcher: Arc<T>,
    ) -> Self {
        Self {
            max_frame_size,
            buffer_size,
            compress,
            rsa_crypto_fetcher,
        }
    }
}

impl<T> Service<TcpStream> for PrepareMessageFramedService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = PrepareMessageFramedResult<T>;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, input_stream: TcpStream) -> Self::Future {
        let framed = Framed::with_capacity(
            input_stream,
            MessageCodec::<T>::new(
                self.max_frame_size,
                self.compress,
                self.rsa_crypto_fetcher.clone(),
            ),
            self.buffer_size,
        );
        Box::pin(async move {
            let (message_framed_write, message_framed_read) = framed.split();
            Ok(PrepareMessageFramedResult {
                message_framed_write,
                message_framed_read,
            })
        })
    }
}

pub struct WriteMessageServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_payload: Option<MessagePayload>,
    pub ref_id: Option<String>,
    pub user_token: String,
    pub payload_encryption_type: PayloadEncryptionType,
}

impl<T> Debug for WriteMessageServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
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

pub struct WriteMessageServiceResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
}

#[derive(Clone, Default)]
pub struct WriteMessageService;

impl<T> Service<WriteMessageServiceRequest<T>> for WriteMessageService
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = WriteMessageServiceResult<T>;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: WriteMessageServiceRequest<T>) -> Self::Future {
        Box::pin(async move {
            let message = match req.message_payload {
                None => Message::new(
                    generate_uuid(),
                    req.ref_id,
                    req.user_token,
                    req.payload_encryption_type,
                    None::<Bytes>,
                ),
                Some(payload) => Message::new(
                    generate_uuid(),
                    req.ref_id,
                    req.user_token,
                    req.payload_encryption_type,
                    Some(payload),
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

pub struct ReadMessageServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
}

impl<T> Debug for ReadMessageServiceRequest<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReadMessageServiceRequest")
    }
}

pub struct ReadMessageServiceResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_payload: MessagePayload,
    pub message_framed_read: MessageFramedRead<T>,
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

impl<T> Service<ReadMessageServiceRequest<T>> for ReadMessageService
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = Option<ReadMessageServiceResult<T>>;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: ReadMessageServiceRequest<T>) -> Self::Future {
        let read_timeout_seconds = self.read_timeout_seconds;
        Box::pin(async move {
            let message = match timeout(
                Duration::from_secs(read_timeout_seconds),
                req.message_framed_read.next(),
            )
            .await
            {
                Err(_e) => {
                    error!("The read timeout in {} seconds.", read_timeout_seconds);
                    return Err(PpaassError::TimeoutError);
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
    type Error = PpaassError;
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
