#![allow(unused)]

use std::fmt::Debug;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::error;

use common::{NetAddress, PpaassError};

#[derive(Debug)]
pub(crate) enum Socks5AuthMethod {
    NoAuthenticationRequired,
    GssApi,
    UsernameAndPassword,
    IanaAssigned,
    ReservedForPrivateMethods,
    NoAcceptableMethods,
}

impl From<u8> for Socks5AuthMethod {
    fn from(v: u8) -> Self {
        match v {
            0 => Socks5AuthMethod::NoAuthenticationRequired,
            1 => Socks5AuthMethod::GssApi,
            2 => Socks5AuthMethod::UsernameAndPassword,
            3 => Socks5AuthMethod::IanaAssigned,
            8 => Socks5AuthMethod::ReservedForPrivateMethods,
            16 => Socks5AuthMethod::NoAcceptableMethods,
            _ => Socks5AuthMethod::NoAuthenticationRequired,
        }
    }
}

impl From<Socks5AuthMethod> for u8 {
    fn from(value: Socks5AuthMethod) -> Self {
        match value {
            Socks5AuthMethod::NoAuthenticationRequired => 0,
            Socks5AuthMethod::GssApi => 1,
            Socks5AuthMethod::UsernameAndPassword => 2,
            Socks5AuthMethod::IanaAssigned => 3,
            Socks5AuthMethod::ReservedForPrivateMethods => 8,
            Socks5AuthMethod::NoAcceptableMethods => 16,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Socks5InitCommandType {
    Connect,
    Bind,
    UdpAssociate,
}

impl TryFrom<u8> for Socks5InitCommandType {
    type Error = PpaassError;
    fn try_from(v: u8) -> Result<Self, PpaassError> {
        match v {
            1 => Ok(Socks5InitCommandType::Connect),
            2 => Ok(Socks5InitCommandType::Bind),
            3 => Ok(Socks5InitCommandType::UdpAssociate),
            unknown_type => {
                error!("Fail to decode socks 5 connect request type: {}", unknown_type);
                Err(PpaassError::CodecError)
            },
        }
    }
}

#[derive(Debug)]
pub(crate) enum Socks5InitCommandResultStatus {
    Succeeded,
    Failure,
    ConnectionNotAllowedByRuleSet,
    NetworkUnReachable,
    HostUnReachable,
    ConnectionRefused,
    TtlExpired,
    CommandNotSupported,
    AddressTypeNotSupported,
    Unassigned,
}

impl From<u8> for Socks5InitCommandResultStatus {
    fn from(v: u8) -> Self {
        match v {
            0 => Socks5InitCommandResultStatus::Succeeded,
            1 => Socks5InitCommandResultStatus::Failure,
            2 => Socks5InitCommandResultStatus::ConnectionNotAllowedByRuleSet,
            3 => Socks5InitCommandResultStatus::NetworkUnReachable,
            4 => Socks5InitCommandResultStatus::HostUnReachable,
            5 => Socks5InitCommandResultStatus::ConnectionRefused,
            6 => Socks5InitCommandResultStatus::TtlExpired,
            7 => Socks5InitCommandResultStatus::CommandNotSupported,
            8 => Socks5InitCommandResultStatus::AddressTypeNotSupported,
            9 => Socks5InitCommandResultStatus::Unassigned,
            unknown_status => {
                error!("Fail to decode socks 5 connect response status: {}", unknown_status);
                Socks5InitCommandResultStatus::Failure
            },
        }
    }
}

impl From<Socks5InitCommandResultStatus> for u8 {
    fn from(value: Socks5InitCommandResultStatus) -> Self {
        match value {
            Socks5InitCommandResultStatus::Succeeded => 0,
            Socks5InitCommandResultStatus::Failure => 1,
            Socks5InitCommandResultStatus::ConnectionNotAllowedByRuleSet => 2,
            Socks5InitCommandResultStatus::NetworkUnReachable => 3,
            Socks5InitCommandResultStatus::HostUnReachable => 4,
            Socks5InitCommandResultStatus::ConnectionRefused => 5,
            Socks5InitCommandResultStatus::TtlExpired => 6,
            Socks5InitCommandResultStatus::CommandNotSupported => 7,
            Socks5InitCommandResultStatus::AddressTypeNotSupported => 8,
            Socks5InitCommandResultStatus::Unassigned => 9,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Socks5Addr {
    IpV4([u8; 4], u16),
    IpV6([u8; 16], u16),
    Domain(String, u16),
}

impl ToString for Socks5Addr {
    fn to_string(&self) -> String {
        match self {
            Self::IpV4(ip_content, port) => {
                format!(
                    "{}.{}.{}.{}:{}",
                    ip_content[0], ip_content[1], ip_content[2], ip_content[3], port
                )
            },
            Self::IpV6(ip_content, port) => {
                let mut ip_content_bytes = Bytes::from(ip_content.to_vec());
                format!(
                    "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{}",
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    ip_content_bytes.get_u16(),
                    port
                )
            },
            Self::Domain(host, port) => {
                format!("{}:{}", host, port)
            },
        }
    }
}

impl TryFrom<&mut Bytes> for Socks5Addr {
    type Error = PpaassError;
    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if !value.has_remaining() {
            error!("Fail to parse socks5 address because of no remaining in bytes buffer.");
            return Err(PpaassError::CodecError);
        }
        let address_type = value.get_u8();
        let address = match address_type {
            1 => {
                if value.remaining() < 6 {
                    error!("Fail to parse socks5 address (IpV4) because of not enough remaining in bytes buffer.");
                    return Err(PpaassError::CodecError);
                }
                let mut addr_content = [0u8; 4];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                Socks5Addr::IpV4(addr_content, port)
            },
            4 => {
                if value.remaining() < 18 {
                    error!("Fail to parse socks5 address (IpV6) because of not enough remaining in bytes buffer.");
                    return Err(PpaassError::CodecError);
                }
                let mut addr_content = [0u8; 16];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                Socks5Addr::IpV6(addr_content, port)
            },
            3 => {
                if value.remaining() < 1 {
                    error!("Fail to parse socks5 address(Domain) because of not enough remaining in bytes buffer.");
                    return Err(PpaassError::CodecError);
                }
                let domain_name_length = value.get_u8() as usize;
                if value.remaining() < domain_name_length + 2 {
                    error!("Fail to parse socks5 address(Domain) because of not enough remaining in bytes buffer, require: {}.", domain_name_length+2);
                    return Err(PpaassError::CodecError);
                }
                let domain_name_bytes = value.copy_to_bytes(domain_name_length);
                let domain_name = match String::from_utf8(domain_name_bytes.to_vec()) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Fail to parse socks5 address(Domain) because of error: {:#?}.", e);
                        return Err(PpaassError::CodecError);
                    },
                };
                let port = value.get_u16();
                Socks5Addr::Domain(domain_name, port)
            },
            unknown_addr_type => {
                error!("Fail to decode socks 5 address type: {}", unknown_addr_type);
                return Err(PpaassError::CodecError);
            },
        };
        Ok(address)
    }
}

impl TryFrom<Bytes> for Socks5Addr {
    type Error = PpaassError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        let value_mut_ref = &mut value;
        value_mut_ref.try_into()
    }
}
impl TryFrom<&mut BytesMut> for Socks5Addr {
    type Error = PpaassError;

    fn try_from(value: &mut BytesMut) -> Result<Self, Self::Error> {
        let value = value.copy_to_bytes(value.len());
        value.try_into()
    }
}
impl From<Socks5Addr> for Bytes {
    fn from(address: Socks5Addr) -> Self {
        let mut result = BytesMut::new();
        match address {
            Socks5Addr::IpV4(addr_content, port) => {
                result.put_u8(1);
                result.put_slice(&addr_content);
                result.put_u16(port);
            },
            Socks5Addr::IpV6(addr_content, port) => {
                result.put_u8(4);
                result.put_slice(&addr_content);
                result.put_u16(port);
            },
            Socks5Addr::Domain(addr_content, port) => {
                result.put_u8(3);
                result.put_u8(addr_content.len() as u8);
                result.put_slice(&addr_content.as_bytes());
                result.put_u16(port);
            },
        }
        result.into()
    }
}
impl From<Socks5Addr> for NetAddress {
    fn from(value: Socks5Addr) -> Self {
        match value {
            Socks5Addr::IpV4(ip_bytes, port) => NetAddress::IpV4(ip_bytes, port),
            Socks5Addr::IpV6(ip_bytes, port) => NetAddress::IpV6(ip_bytes, port),
            Socks5Addr::Domain(host, port) => NetAddress::Domain(host, port),
        }
    }
}
#[derive(Debug)]
pub(crate) struct Socks5AuthCommandContent {
    pub version: u8,
    pub method_number: u8,
    pub methods: Vec<Socks5AuthMethod>,
}

impl Socks5AuthCommandContent {
    pub fn new(method_number: u8, methods: Vec<Socks5AuthMethod>) -> Self {
        Socks5AuthCommandContent {
            version: 5,
            method_number,
            methods,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Socks5AuthCommandResultContent {
    pub version: u8,
    pub method: Socks5AuthMethod,
}

impl Socks5AuthCommandResultContent {
    pub fn new(method: Socks5AuthMethod) -> Self {
        Socks5AuthCommandResultContent {
            version: 5u8,
            method,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Socks5InitCommandContent {
    pub version: u8,
    pub request_type: Socks5InitCommandType,
    pub dest_address: Socks5Addr,
}

impl Socks5InitCommandContent {
    pub fn new(request_type: Socks5InitCommandType, dest_address: Socks5Addr) -> Self {
        Socks5InitCommandContent {
            version: 5,
            request_type,
            dest_address,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Socks5InitCommandResultContent {
    pub version: u8,
    pub status: Socks5InitCommandResultStatus,
    pub bind_address: Option<Socks5Addr>,
}

impl Socks5InitCommandResultContent {
    pub fn new(status: Socks5InitCommandResultStatus, bind_address: Option<Socks5Addr>) -> Self {
        Socks5InitCommandResultContent {
            version: 5,
            status,
            bind_address,
        }
    }
}

/// Socks5 udp data request
#[derive(Debug)]
pub(crate) struct Socks5UdpDataCommandContent {
    pub frag: u8,
    pub address: Socks5Addr,
    pub data: Bytes,
}

#[derive(Debug)]
pub(crate) struct Socks5UdpDataCommandResultContent {
    pub frag: u8,
    pub dest_address: Socks5Addr,
    pub data: Bytes,
}

impl Socks5UdpDataCommandResultContent {
    pub fn new(frag: u8, dest_address: Socks5Addr, data: Bytes) -> Self {
        Self {
            frag,
            dest_address,
            data,
        }
    }
}

#[derive(Debug)]
pub(crate) struct UdpDiagram {
    pub source_port: u16,
    pub target_port: u16,
    pub length: u16,
    pub checksum: u16,
    pub data: Bytes,
}

impl From<Bytes> for UdpDiagram {
    fn from(bytes: Bytes) -> Self {
        let mut bytes = Bytes::from(bytes);
        let source_port = bytes.get_u16();
        let target_port = bytes.get_u16();
        let length = bytes.get_u16();
        let checksum = bytes.get_u16();
        let data: Bytes = bytes.copy_to_bytes(length as usize);
        Self {
            source_port,
            target_port,
            length,
            checksum,
            data,
        }
    }
}

impl From<UdpDiagram> for Vec<u8> {
    fn from(value: UdpDiagram) -> Self {
        let mut result = BytesMut::new();
        result.put_u16(value.source_port);
        result.put_u16(value.target_port);
        result.put_u16(value.length);
        result.put_u16(value.checksum);
        result.put_slice(value.data.chunk());
        result.to_vec()
    }
}
