use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::error;

use crate::error::CommonError;

const IPV4_TYPE: u8 = 0;
const IPV6_TYPE: u8 = 1;
const DOMAIN_TYPE: u8 = 2;

/// The net address
#[derive(Debug)]
pub enum NetAddress {
    /// Ip v4 net address
    IpV4([u8; 4], u16),
    /// Ip v6 net address
    IpV6([u8; 16], u16),
    /// Domain net address
    Domain(String, u16),
}

impl TryFrom<&mut Bytes> for NetAddress {
    type Error = CommonError;

    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if !value.has_remaining() {
            error!("Fail to parse NetAddress because of no remaining in bytes buffer.");
            return Err(CommonError::CodecError);
        }
        let address_type = value.get_u8();
        let address = match address_type {
            IPV4_TYPE => {
                //Convert the NetAddress::IpV4
                //A ip v6 address is 6 bytes: 4 bytes for host, 2 bytes for port
                if value.remaining() < 6 {
                    error!("Fail to parse NetAddress(IpV4) because of not enough remaining in bytes buffer.");
                    return Err(CommonError::CodecError);
                }
                let mut addr_content = [0u8; 4];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                NetAddress::IpV4(addr_content, port)
            }
            IPV6_TYPE => {
                //Convert the NetAddress::IpV6
                //A ip v6 address is 18 bytes: 16 bytes for host, 2 bytes for port
                if value.remaining() < 18 {
                    error!("Fail to parse NetAddress(IpV6) because of not enough remaining in bytes buffer.");
                    return Err(CommonError::CodecError);
                }
                let mut addr_content = [0u8; 16];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                NetAddress::IpV6(addr_content, port)
            }
            DOMAIN_TYPE => {
                //Convert the NetAddress::Domain
                if value.remaining() < 4 {
                    error!("Fail to parse NetAddress(Domain) because of not enough remaining in bytes buffer.");
                    return Err(CommonError::CodecError);
                }
                let host_name_length = value.get_u32() as usize;
                if value.remaining() < host_name_length + 2 {
                    error!("Fail to parse NetAddress(Domain) because of not enough remaining in bytes buffer, require: {}.", host_name_length + 2);
                    return Err(CommonError::CodecError);
                }
                let host_bytes = value.copy_to_bytes(host_name_length);
                let host = match String::from_utf8(host_bytes.to_vec()) {
                    Ok(v) => v,
                    Err(e) => {
                        error!(
                            "Fail to parse NetAddress(Domain) because of error: {:#?}.",
                            e
                        );
                        return Err(CommonError::CodecError);
                    }
                };
                let port = value.get_u16();
                NetAddress::Domain(host, port)
            }
            invalid_address_type => {
                error!(
                    "Fail to parse NetAddress because of invalide address type {}.",
                    invalid_address_type
                );
                return Err(CommonError::CodecError);
            }
        };
        Ok(address)
    }
}

impl TryFrom<Bytes> for NetAddress {
    type Error = CommonError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        let value_mut_ref: &mut Bytes = &mut value;
        value_mut_ref.try_into()
    }
}

impl From<NetAddress> for Bytes {
    fn from(address: NetAddress) -> Self {
        let mut result = BytesMut::new();
        match address {
            NetAddress::IpV4(addr_content, port) => {
                result.put_u8(IPV4_TYPE);
                result.put_slice(&addr_content);
                result.put_u16(port);
            }
            NetAddress::IpV6(addr_content, port) => {
                result.put_u8(IPV6_TYPE);
                result.put_slice(&addr_content);
                result.put_u16(port);
            }
            NetAddress::Domain(addr_content, port) => {
                result.put_u8(DOMAIN_TYPE);
                result.put_slice(addr_content.as_bytes());
                result.put_u16(port);
            }
        }
        result.into()
    }
}
