use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tracing::error;

use common::PpaassError;

use crate::message::socks5::{
    Socks5Addr, Socks5AuthCommandContent, Socks5AuthCommandResultContent, Socks5AuthMethod, Socks5InitCommandContent, Socks5InitCommandResultContent,
    Socks5InitCommandType, Socks5UdpDataCommandContent, Socks5UdpDataCommandResultContent,
};

use super::SOCKS5_FLAG;

pub(crate) struct Socks5AuthCommandContentCodec;

impl Decoder for Socks5AuthCommandContentCodec {
    type Item = Socks5AuthCommandContent;
    type Error = PpaassError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }
        let version = src.get_u8();
        if version != 5 {
            error!("The incoming protocol is not for socks 5.");
            return Err(PpaassError::CodecError);
        }
        let methods_number = src.get_u8();
        let mut methods = Vec::<Socks5AuthMethod>::new();
        (0..methods_number).for_each(|_| {
            methods.push(Socks5AuthMethod::from(src.get_u8()));
        });
        Ok(Some(Socks5AuthCommandContent::new(methods_number, methods)))
    }
}

impl Encoder<Socks5AuthCommandResultContent> for Socks5AuthCommandContentCodec {
    type Error = PpaassError;

    fn encode(&mut self, item: Socks5AuthCommandResultContent, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.put_u8(item.version);
        dst.put_u8(item.method.into());
        Ok(())
    }
}
pub(crate) struct Socks5InitCommandContentCodec;

impl Decoder for Socks5InitCommandContentCodec {
    type Item = Socks5InitCommandContent;
    type Error = PpaassError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }
        let version = src.get_u8();
        if version != SOCKS5_FLAG {
            error!("The incoming protocol is not for socks 5.");
            return Err(PpaassError::CodecError);
        }
        let request_type: Socks5InitCommandType = match src.get_u8().try_into() {
            Err(e) => {
                error!("Fail to parse socks5 connect request type because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
            Ok(v) => v,
        };
        src.get_u8();
        let dest_address: Socks5Addr = match src.try_into() {
            Err(e) => {
                error!("Fail to parse socks5 connect request destination address because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
            Ok(v) => v,
        };
        Ok(Some(Socks5InitCommandContent::new(request_type, dest_address)))
    }
}

impl Encoder<Socks5InitCommandResultContent> for Socks5InitCommandContentCodec {
    type Error = PpaassError;

    fn encode(&mut self, item: Socks5InitCommandResultContent, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.put_u8(item.version);
        dst.put_u8(item.status.into());
        dst.put_u8(0);
        if let Some(bind_address) = item.bind_address {
            dst.put::<Bytes>(bind_address.into());
        }
        Ok(())
    }
}

pub(crate) struct Socks5UdpDataCommandContentCodec;

impl Decoder for Socks5UdpDataCommandContentCodec {
    type Item = Socks5UdpDataCommandContent;
    type Error = PpaassError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Check the buffer
        if !src.has_remaining() {
            return Ok(None);
        }
        // Check and skip the revision
        if src.remaining() < 2 {
            return Err(PpaassError::CodecError);
        }
        src.get_u16();
        if src.remaining() < 1 {
            return Err(PpaassError::CodecError);
        }
        let frag = src.get_u8();
        let address: Socks5Addr = match src.try_into() {
            Err(e) => {
                error!("Fail to decode socks5 udp data request because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
            Ok(v) => v,
        };
        let data = src.copy_to_bytes(src.remaining());
        Ok(Some(Socks5UdpDataCommandContent { frag, address, data }))
    }
}

impl Encoder<Socks5UdpDataCommandResultContent> for Socks5UdpDataCommandContentCodec {
    type Error = PpaassError;

    fn encode(&mut self, item: Socks5UdpDataCommandResultContent, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.put_u16(0);
        dst.put_u8(item.frag);
        dst.put::<Bytes>(item.dest_address.into());
        dst.put_slice(item.data.chunk());
        Ok(())
    }
}
