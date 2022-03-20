use std::task::{Context, Poll};

use futures_util::future::BoxFuture;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::StreamExt;
use rand::rngs::OsRng;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tower::Service;

use crate::{CommonError, Message, MessageCodec};

pub type MessageFrameRead = SplitStream<Framed<TcpStream, MessageCodec<OsRng>>>;
pub type MessageFrameWrite = SplitSink<Framed<TcpStream, MessageCodec<OsRng>>, Message>;

pub struct PrepareMessageFramedResult {
    pub sink: MessageFrameWrite,
    pub stream: MessageFrameRead,
}

#[derive(Clone)]
pub struct PrepareMessageFramedService {
    public_key: &'static str,
    private_key: &'static str,
    buffer_size: usize,
    compress: bool,
}

impl PrepareMessageFramedService {
    pub fn new(
        public_key: &'static str,
        private_key: &'static str,
        buffer_size: usize,
        compress: bool,
    ) -> Self {
        Self {
            public_key,
            private_key,
            buffer_size,
            compress,
        }
    }
}

impl Service<TcpStream> for PrepareMessageFramedService {
    type Response = PrepareMessageFramedResult;
    type Error = CommonError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, input_stream: TcpStream) -> Self::Future {
        let framed = Framed::with_capacity(
            input_stream,
            MessageCodec::new(
                &(*self.public_key),
                &(*self.private_key),
                self.buffer_size,
                self.compress,
            ),
            self.buffer_size,
        );
        let (sink, stream) = framed.split();
        Box::pin(async move { Ok(PrepareMessageFramedResult { sink, stream }) })
    }
}
