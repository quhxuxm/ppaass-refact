use std::mem::size_of;

use bytes::BytesMut;
use tokio_util::codec::Decoder;
use tracing::{debug, error};

use common::PpaassError;

pub(crate) const SOCKS5_FLAG: u8 = 5;
pub(crate) const SOCKS4_FLAG: u8 = 4;
pub(crate) enum Protocol {
    Http,
    Socks5,
}

pub(crate) struct SwitchProtocolDecoder;

impl Decoder for SwitchProtocolDecoder {
    type Item = Protocol;

    type Error = PpaassError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < size_of::<u8>() {
            debug!("Incoming agent client stream is empty, nothing to decode.");
            return Ok(None);
        }
        let protocol_flag = src[0];
        return match protocol_flag {
            SOCKS5_FLAG => {
                debug!("Incoming agent client protocol is socks5.");
                Ok(Some(Protocol::Socks5))
            },
            SOCKS4_FLAG => {
                error!("Incoming agent client protocol is socks4, which is unsupported!");
                Err(PpaassError::CodecError)
            },
            _ => {
                debug!("Incoming agent client protocol is http.");
                Ok(Some(Protocol::Http))
            },
        };
    }
}
