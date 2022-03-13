use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tracing::error;

use common::CommonError;

use crate::protocol::socks::{
    Socks5Addr, Socks5AuthMethod, Socks5AuthRequest, Socks5AuthResponse, Socks5ConnectRequest,
    Socks5ConnectRequestType, Socks5ConnectResponse,
};

pub(crate) struct Socks5AuthCodec;

impl Decoder for Socks5AuthCodec {
    type Item = Socks5AuthRequest;
    type Error = CommonError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }
        let version = src.get_u8();
        if version != 5 {
            return Err(CommonError::CodecError);
        }
        let methods_number = src.get_u8();
        let mut methods = Vec::<Socks5AuthMethod>::new();
        (0..methods_number).for_each(|_| {
            methods.push(Socks5AuthMethod::from(src.get_u8()));
        });
        Ok(Some(Socks5AuthRequest::new(methods_number, methods)))
    }
}

impl Encoder<Socks5AuthResponse> for Socks5AuthCodec {
    type Error = CommonError;

    fn encode(&mut self, item: Socks5AuthResponse, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.put_u8(item.version);
        dst.put_u8(item.method.into());
        Ok(())
    }
}

pub(crate) struct Socks5ConnectCodec;

impl Decoder for Socks5ConnectCodec {
    type Item = Socks5ConnectRequest;
    type Error = CommonError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }
        let version = src.get_u8();
        if version != 5 {
            return Err(CommonError::CodecError);
        }
        let request_type: Socks5ConnectRequestType = match src.get_u8().try_into() {
            Err(e) => {
                error!(
                    "Fail to parse socks5 connect request type because of error: {:#?}",
                    e
                );
                return Err(CommonError::CodecError);
            }
            Ok(v) => v,
        };
        src.get_u8();
        let source_address: Socks5Addr = match src.try_into() {
            Err(e) => {
                error!(
                    "Fail to parse socks5 connect request source address because of error: {:#?}",
                    e
                );
                return Err(CommonError::CodecError);
            }
            Ok(v) => v,
        };
        let dest_address: Socks5Addr = match src.try_into() {
            Err(e) => {
                error!(
                    "Fail to parse socks5 connect request destination address because of error: {:#?}",
                    e
                );
                return Err(CommonError::CodecError);
            }
            Ok(v) => v,
        };
        Ok(Some(Socks5ConnectRequest::new(
            request_type,
            source_address,
            dest_address,
        )))
    }
}

impl Encoder<Socks5ConnectResponse> for Socks5ConnectCodec {
    type Error = CommonError;

    fn encode(
        &mut self,
        item: Socks5ConnectResponse,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        dst.put_u8(item.version);
        dst.put_u8(item.status.into());
        dst.put_u8(0);
        if let Some(bind_address) = item.bind_address {
            dst.put::<Bytes>(bind_address.into());
        }
        Ok(())
    }
}
