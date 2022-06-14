use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;

use tower::Service;

use common::{MessageFramedRead, MessageFramedWrite, NetAddress, PpaassError, RsaCryptoFetcher};

#[derive(Debug)]
#[allow(unused)]
pub(crate) struct Socks5UdpAssociateServiceRequest {
    pub proxy_addresses: Arc<Vec<SocketAddr>>,
    pub client_address: SocketAddr,
}

#[allow(unused)]
pub(crate) struct Socks5UdpAssociateServiceResponse<T>
where
    T: RsaCryptoFetcher,
{
    pub message_framed_read: MessageFramedRead<T>,
    pub message_framed_write: MessageFramedWrite<T>,
    pub source_address: NetAddress,
    pub target_address: NetAddress,
    pub connect_response_message_id: String,
    pub proxy_address: Option<SocketAddr>,
}

#[allow(unused)]
pub(crate) struct Socks5UdpAssociateService<T>
where
    T: RsaCryptoFetcher,
{
    rsa_crypto_fetcher: Arc<T>,
}

impl<T> Socks5UdpAssociateService<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(rsa_crypto_fetcher: Arc<T>) -> Self {
        Self { rsa_crypto_fetcher }
    }
}

impl<T> Service<Socks5UdpAssociateServiceRequest> for Socks5UdpAssociateService<T>
where
    T: RsaCryptoFetcher + Send + Sync + 'static,
{
    type Response = Socks5UdpAssociateServiceResponse<T>;
    type Error = PpaassError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Socks5UdpAssociateServiceRequest) -> Self::Future {
        todo!()
    }
}
