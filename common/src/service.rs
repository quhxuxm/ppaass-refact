use std::{
    fmt::{Debug, Formatter},
    net::SocketAddr,
};
use std::{sync::Arc, time::Duration};

use bytes::Bytes;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_util::codec::Framed;

use tracing::{debug, error, info, instrument, trace};

use crate::{crypto::RsaCryptoFetcher, generate_uuid, Message, MessageCodec, MessagePayload, MessageStream, PayloadEncryptionType, PpaassError};

pub type MessageFramedRead<T, S> = SplitStream<Framed<S, MessageCodec<T>>>;
pub type MessageFramedWrite<T, S> = SplitSink<Framed<S, MessageCodec<T>>, Message>;

pub struct MessageFramedGenerateResult<T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub message_framed_write: MessageFramedWrite<T, S>,
    pub message_framed_read: MessageFramedRead<T, S>,
}

pub struct MessageFramedGenerator;

impl MessageFramedGenerator {
    #[instrument(skip_all, fields(input_stream))]
    pub async fn generate<T, S>(input_stream: S, buffer_size: usize, compress: bool, rsa_crypto_fetcher: Arc<T>) -> MessageFramedGenerateResult<T, S>
    where
        T: RsaCryptoFetcher + Debug,
        S: AsyncRead + AsyncWrite,
    {
        let framed = Framed::with_capacity(input_stream, MessageCodec::<T>::new(compress, rsa_crypto_fetcher), buffer_size);
        let (message_framed_write, message_framed_read) = framed.split();
        MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
        }
    }
}

pub struct WriteMessageFramedRequest<'a, T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub message_framed_write: MessageFramedWrite<T, S>,
    pub message_payloads: Option<Vec<MessagePayload>>,
    pub ref_id: Option<&'a str>,
    pub connection_id: Option<&'a str>,
    pub user_token: &'a str,
    pub payload_encryption_type: PayloadEncryptionType,
}

impl<'a, T, S> Debug for WriteMessageFramedRequest<'a, T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WriteMessageServiceRequest: ref_id={}, user_token={}, payload_encryption_type={:#?}",
            self.ref_id.unwrap_or(""),
            self.user_token,
            self.payload_encryption_type
        )
    }
}

#[derive(Debug)]
pub struct WriteMessageFramedResult<T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub message_framed_write: MessageFramedWrite<T, S>,
}

pub struct WriteMessageFramedError<T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub message_framed_write: MessageFramedWrite<T, S>,
    pub source: PpaassError,
}

pub struct MessageFramedWriter;

impl MessageFramedWriter {
    #[instrument(fields(request.connection_id))]
    pub async fn write<'a, T, S>(request: WriteMessageFramedRequest<'a, T, S>) -> Result<WriteMessageFramedResult<T, S>, WriteMessageFramedError<T, S>>
    where
        T: RsaCryptoFetcher,
        S: AsyncRead + AsyncWrite,
    {
        let WriteMessageFramedRequest {
            message_framed_write,
            message_payloads,
            ref_id,
            connection_id,
            user_token,
            payload_encryption_type,
        } = request;
        let messages = match message_payloads {
            None => vec![Message::new(
                generate_uuid(),
                ref_id,
                connection_id.map(|v| v.to_owned()),
                user_token,
                payload_encryption_type,
                None::<Bytes>,
            )],
            Some(payloads) => payloads
                .into_iter()
                .map(|item| {
                    Message::new(
                        generate_uuid(),
                        ref_id.clone(),
                        connection_id.map(|v| v.to_owned()),
                        user_token.clone(),
                        payload_encryption_type.clone(),
                        Some(item),
                    )
                })
                .collect::<Vec<_>>(),
        };
        let mut message_framed_write = message_framed_write;
        let mut messages_stream: MessageStream = messages.into();
        if let Err(e) = message_framed_write.send_all(&mut messages_stream).await {
            error!("Connection [{connection_id:?}] fail to write message because of error: {:#?}", e);
            if let Err(e) = message_framed_write.close().await {
                error!("Connection [{connection_id:?}] fail to close message writer because of error: {:#?}", e);
            };
            return Err(WriteMessageFramedError {
                message_framed_write,
                source: e,
            });
        }
        if let Err(e) = message_framed_write.flush().await {
            error!("Connection [{connection_id:?}] fail to flush message because of error: {:#?}", e);
            if let Err(e) = message_framed_write.close().await {
                error!("Connection [{connection_id:?}] fail to close message writer because of error: {e:#?}");
            };
            return Err(WriteMessageFramedError {
                message_framed_write,
                source: e,
            });
        }
        Ok(WriteMessageFramedResult { message_framed_write })
    }
}

#[derive(Debug)]
pub struct ReadMessageFramedRequest<'a, T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub connection_id: &'a str,
    pub message_framed_read: MessageFramedRead<T, S>,
    pub timeout: Option<u64>,
}

#[derive(Debug)]
pub struct ReadMessageFramedResultContent {
    pub message_id: String,
    pub message_payload: Option<MessagePayload>,
    pub user_token: String,
}

#[derive(Debug)]
pub struct ReadMessageFramedResult<T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub message_framed_read: MessageFramedRead<T, S>,
    pub content: Option<ReadMessageFramedResultContent>,
}

pub struct ReadMessageFramedError<T, S>
where
    T: RsaCryptoFetcher,
    S: AsyncRead + AsyncWrite,
{
    pub message_framed_read: MessageFramedRead<T, S>,
    pub source: PpaassError,
}

pub struct MessageFramedReader;

impl MessageFramedReader {
    #[instrument(skip_all, fields(request.connection_id))]
    pub async fn read<'a, T, S>(request: ReadMessageFramedRequest<'a, T, S>) -> Result<ReadMessageFramedResult<T, S>, ReadMessageFramedError<T, S>>
    where
        T: RsaCryptoFetcher + Debug,
        S: AsyncRead + AsyncWrite,
    {
        let ReadMessageFramedRequest {
            connection_id,
            mut message_framed_read,
            timeout,
        } = request;
        let timeout_seconds = timeout.unwrap_or(u64::MAX);
        let result = match tokio::time::timeout(Duration::from_secs(timeout_seconds), message_framed_read.next()).await {
            Err(_elapsed) => {
                error!("Connection [{connection_id}] fail to read message from frame because of timeout:{timeout_seconds}");
                return Err(ReadMessageFramedError {
                    message_framed_read,
                    source: PpaassError::TimeoutError { elapsed: timeout_seconds },
                });
            },
            Ok(None) => {
                return Ok(ReadMessageFramedResult {
                    content: None,
                    message_framed_read,
                });
            },
            Ok(Some(Ok(message))) => ReadMessageFramedResult {
                content: Some(ReadMessageFramedResultContent {
                    message_id: message.id,
                    user_token: message.user_token,
                    message_payload: match message.payload {
                        None => {
                            info!("Connection [{}] no payload in the message.", connection_id);
                            None
                        },
                        Some(payload_bytes) => match payload_bytes.try_into() {
                            Ok(v) => {
                                trace!("Connection [{}] read message payload from remote:\n\n{:#?}\n\n", connection_id, v);
                                Some(v)
                            },
                            Err(e) => {
                                error!("Connection [{connection_id}] fail to decode message payload because of error, error: {e:#?}");
                                return Err(ReadMessageFramedError {
                                    message_framed_read,
                                    source: e,
                                });
                            },
                        },
                    },
                }),
                message_framed_read,
            },
            Ok(Some(Err(e))) => {
                error!("Connection [{connection_id}] fail to decode message because of error, error: {e:#?}");
                return Err(ReadMessageFramedError {
                    message_framed_read,
                    source: e,
                });
            },
        };
        Ok(result)
    }
}

#[derive(Debug)]
pub struct PayloadEncryptionTypeSelectRequest<'a> {
    pub user_token: &'a str,
    pub encryption_token: Bytes,
}

#[derive(Debug)]
pub struct PayloadEncryptionTypeSelectResult {
    pub user_token: String,
    pub encryption_token: Bytes,
    pub payload_encryption_type: PayloadEncryptionType,
}

pub struct PayloadEncryptionTypeSelector;

impl PayloadEncryptionTypeSelector {
    #[instrument(fields(request.user_token))]
    pub async fn select<'a>(request: PayloadEncryptionTypeSelectRequest<'a>) -> Result<PayloadEncryptionTypeSelectResult, PpaassError> {
        let PayloadEncryptionTypeSelectRequest { user_token, encryption_token } = request;
        Ok(PayloadEncryptionTypeSelectResult {
            payload_encryption_type: PayloadEncryptionType::Blowfish(encryption_token.clone()),
            user_token: user_token.to_owned(),
            encryption_token,
        })
    }
}

#[derive(Debug)]
pub struct TcpConnectRequest {
    pub connect_addresses: Vec<SocketAddr>,
    pub connected_stream_so_linger: u64,
}

#[derive(Debug)]
pub struct TcpConnectResult {
    pub connected_stream: TcpStream,
}

pub struct TcpConnector;

impl TcpConnector {
    #[instrument(fields(request.connect_addresses))]
    pub async fn connect(request: TcpConnectRequest) -> Result<TcpConnectResult, PpaassError> {
        let TcpConnectRequest {
            connect_addresses,
            connected_stream_so_linger,
        } = request;
        debug!("Begin connect to target: {connect_addresses:?}");
        let connected_stream = TcpStream::connect(&connect_addresses.as_slice()).await?;
        connected_stream.set_nodelay(true)?;
        connected_stream.set_linger(Some(Duration::from_secs(connected_stream_so_linger)))?;
        debug!("Success connect to target: {connect_addresses:?}");
        Ok(TcpConnectResult { connected_stream })
    }
}
