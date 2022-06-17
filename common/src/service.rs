use std::{
    fmt::{Debug, Formatter},
    net::SocketAddr,
};
use std::{sync::Arc, time::Duration};

use bytes::Bytes;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use tracing::{debug, error, info, trace};

use crate::{crypto::RsaCryptoFetcher, generate_uuid, Message, MessageCodec, MessagePayload, PayloadEncryptionType, PpaassError};

pub type MessageFramedRead<T> = SplitStream<Framed<TcpStream, MessageCodec<T>>>;
pub type MessageFramedWrite<T> = SplitSink<Framed<TcpStream, MessageCodec<T>>, Message>;

pub struct MessageFramedGenerateResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_framed_read: MessageFramedRead<T>,
    pub framed_address: Option<SocketAddr>,
}

pub struct MessageFramedGenerator;

impl MessageFramedGenerator {
    pub async fn generate<T>(
        input_stream: TcpStream, buffer_size: usize, compress: bool, rsa_crypto_fetcher: Arc<T>,
    ) -> Result<MessageFramedGenerateResult<T>, PpaassError>
    where
        T: RsaCryptoFetcher,
    {
        let framed_address = input_stream.peer_addr().ok();
        let framed = Framed::with_capacity(input_stream, MessageCodec::<T>::new(compress, rsa_crypto_fetcher), buffer_size);
        let (message_framed_write, message_framed_read) = framed.split();
        Ok(MessageFramedGenerateResult {
            message_framed_write,
            message_framed_read,
            framed_address,
        })
    }
}

pub struct WriteMessageFramedRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
    pub message_payload: Option<MessagePayload>,
    pub ref_id: Option<String>,
    pub connection_id: Option<String>,
    pub user_token: String,
    pub payload_encryption_type: PayloadEncryptionType,
}

impl<T> Debug for WriteMessageFramedRequest<T>
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

pub struct WriteMessageFramedResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
}

pub struct WriteMessageFramedError<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_write: MessageFramedWrite<T>,
    pub source: PpaassError,
}

impl<T> Debug for WriteMessageFramedError<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteMessageServiceError")
            .field("message_framed_write", &self.message_framed_write)
            .field("source", &self.source)
            .finish()
    }
}

pub struct MessageFramedWriter;

impl MessageFramedWriter {
    pub async fn write<T>(request: WriteMessageFramedRequest<T>) -> Result<WriteMessageFramedResult<T>, WriteMessageFramedError<T>>
    where
        T: RsaCryptoFetcher,
    {
        let WriteMessageFramedRequest {
            message_framed_write,
            message_payload,
            ref_id,
            connection_id,
            user_token,
            payload_encryption_type,
        } = request;
        let message = match message_payload {
            None => Message::new(generate_uuid(), ref_id, connection_id, user_token, payload_encryption_type, None::<Bytes>),
            Some(payload) => {
                let message = Message::new(generate_uuid(), ref_id, connection_id, user_token, payload_encryption_type, Some(payload));
                debug!("Write message to remote:\n\n{:?}\n\n", message);
                trace!("Write message payload to remote:\n\n{:#?}\n\n", message.payload);
                message
            },
        };
        let mut message_framed_write = message_framed_write;
        if let Err(e) = message_framed_write.send(message).await {
            error!("Fail to write message because of error: {:#?}", e);
            if let Err(e) = message_framed_write.flush().await {
                error!("Fail to flush message because of error: {:#?}", e);
            }
            if let Err(e) = message_framed_write.close().await {
                error!("Fail to close message writer because of error: {:#?}", e);
            };
            return Err(WriteMessageFramedError {
                message_framed_write,
                source: e,
            });
        }
        Ok(WriteMessageFramedResult { message_framed_write })
    }
}

pub struct ReadMessageFramedRequest<T>
where
    T: RsaCryptoFetcher,
{
    pub connection_id: String,
    pub message_framed_read: MessageFramedRead<T>,
    pub read_from_address: Option<SocketAddr>,
}

impl<T> Debug for ReadMessageFramedRequest<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadMessageServiceRequest")
            .field("connection_id", &self.connection_id)
            .field("message_framed_read", &self.message_framed_read)
            .field("read_from_address", &self.read_from_address)
            .finish()
    }
}

pub struct ReadMessageFramedResultContent {
    pub message_id: String,
    pub message_payload: Option<MessagePayload>,
    pub user_token: String,
}

pub struct ReadMessageFramedResult<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub content: Option<ReadMessageFramedResultContent>,
}

pub struct ReadMessageFramedError<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub source: PpaassError,
}

impl<T> Debug for ReadMessageFramedError<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadMessageServiceError")
            .field("message_framed_read", &self.message_framed_read)
            .field("source", &self.source)
            .finish()
    }
}

pub struct MessageFramedReader;

impl MessageFramedReader {
    pub async fn read<T>(request: ReadMessageFramedRequest<T>) -> Result<ReadMessageFramedResult<T>, ReadMessageFramedError<T>>
    where
        T: RsaCryptoFetcher,
    {
        let ReadMessageFramedRequest {
            connection_id,
            mut message_framed_read,
            read_from_address,
        } = request;
        let result = match message_framed_read.next().await {
            None => {
                return Ok(ReadMessageFramedResult {
                    content: None,
                    message_framed_read,
                });
            },
            Some(Ok(message)) => ReadMessageFramedResult {
                content: Some(ReadMessageFramedResultContent {
                    message_id: message.id,
                    user_token: message.user_token,
                    message_payload: match message.payload {
                        None => {
                            info!("Connection [{}] no payload in the message, read from: {:?}.", connection_id, read_from_address);
                            None
                        },
                        Some(payload_bytes) => match payload_bytes.try_into() {
                            Ok(v) => {
                                trace!("Connection [{}] read message payload from remote:\n\n{:#?}\n\n", connection_id, v);
                                Some(v)
                            },
                            Err(e) => {
                                error!(
                                    "Connection [{}] fail to decode message payload because of error, read from:{:?}, error: {:#?}",
                                    connection_id, read_from_address, e
                                );
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
            Some(Err(e)) => {
                error!(
                    "Connection [{}] fail to decode message because of error, read from {:?}, error: {:#?}",
                    connection_id, read_from_address, e
                );
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
pub struct PayloadEncryptionTypeSelectRequest {
    pub user_token: String,
    pub encryption_token: Bytes,
}

pub struct PayloadEncryptionTypeSelectResult {
    pub user_token: String,
    pub encryption_token: Bytes,
    pub payload_encryption_type: PayloadEncryptionType,
}

pub struct PayloadEncryptionTypeSelector;

impl PayloadEncryptionTypeSelector {
    pub async fn select(request: PayloadEncryptionTypeSelectRequest) -> Result<PayloadEncryptionTypeSelectResult, PpaassError> {
        let PayloadEncryptionTypeSelectRequest { user_token, encryption_token } = request;
        Ok(PayloadEncryptionTypeSelectResult {
            payload_encryption_type: PayloadEncryptionType::Blowfish(encryption_token.clone()),
            user_token,
            encryption_token,
        })
    }
}

pub struct TcpConnectRequest {
    pub connect_addresses: Vec<SocketAddr>,
    pub connected_stream_so_linger: u64,
}

pub struct TcpConnectResult {
    pub connected_stream: TcpStream,
}

pub struct TcpConnector;

impl TcpConnector {
    pub async fn connect(request: TcpConnectRequest) -> Result<TcpConnectResult, PpaassError> {
        let TcpConnectRequest {
            connect_addresses,
            connected_stream_so_linger,
        } = request;
        debug!("Begin connect to target: {:?}", connect_addresses);
        let connected_stream = TcpStream::connect(&connect_addresses.as_slice()).await?;
        connected_stream.set_nodelay(true)?;
        connected_stream.set_linger(Some(Duration::from_secs(connected_stream_so_linger)))?;
        debug!("Success connect to target: {:?}", connect_addresses);
        Ok(TcpConnectResult { connected_stream })
    }
}
