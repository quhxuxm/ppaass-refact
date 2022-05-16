use common::CommonError;
use tokio_util::codec::Decoder;

use tracing::{debug, error};

const SOCKS5_FLAG: u8 = 5;
const SOCKS4_FLAG: u8 = 4;
pub(crate) enum Protocol {
    Http,
    Socks5,
}
pub(crate) struct InitializeProtocolDecoder;

impl Decoder for InitializeProtocolDecoder {
    type Item = Protocol;

    type Error = CommonError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 1 {
            debug!("Incoming agent client stream is empty, nothing to decode.");
            return Ok(None);
        }
        let protocol_flag = src[0];
        match protocol_flag {
            SOCKS5_FLAG => {
                debug!("Incoming agent client protocol is socks5.");
                return Ok(Some(Protocol::Socks5));
            },
            SOCKS4_FLAG => {
                error!("Incoming agent client protocol is socks4, which is unspported!");
                return Err(CommonError::CodecError);
            },
            _ => {
                debug!("Incoming agent client protocol is http.");
                return Ok(Some(Protocol::Http));
            },
        }
    }
}
