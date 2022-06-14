use std::mem::size_of;

use bytes::BytesMut;
use pretty_hex::*;
use tokio_util::codec::Decoder;
use tracing::{debug, error};

use anyhow::anyhow;
use common::PpaassError;

pub(crate) mod http;
pub(crate) mod socks5;

pub(crate) const SOCKS5_FLAG: u8 = 5;
pub(crate) const SOCKS4_FLAG: u8 = 4;

/// The client side protocol
pub(crate) enum Protocol {
    /// The client side choose to use HTTP proxy
    Http,
    /// The client side choose to use Socks5 proxy
    Socks5,
}

/// The decoder used to switch the client side protocol
pub(crate) struct SwitchClientProtocolDecoder;

impl Decoder for SwitchClientProtocolDecoder {
    type Item = Protocol;

    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Use the first byte to decide what protocol the client side is using.
        if src.len() < size_of::<u8>() {
            debug!(
                "Incoming agent client stream is empty, nothing to decode, input client packet: \n\n{}\n\n",
                pretty_hex(src)
            );
            return Ok(None);
        }
        let protocol_flag = src[0];
        return match protocol_flag {
            SOCKS5_FLAG => {
                debug!("Incoming agent client protocol is socks5, input client packet: \n\n{}\n\n", pretty_hex(src));
                Ok(Some(Protocol::Socks5))
            },
            SOCKS4_FLAG => {
                error!(
                    "Incoming agent client protocol is socks4, which is unsupported, input client packet: \n\n{}\n\n",
                    pretty_hex(src)
                );
                Err(anyhow!(PpaassError::CodecError))
            },
            _ => {
                debug!("Incoming agent client protocol is http, input client packet: \n\n{}\n\n", pretty_hex(src));
                Ok(Some(Protocol::Http))
            },
        };
    }
}
