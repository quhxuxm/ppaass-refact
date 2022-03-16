use std::fmt::Debug;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::error;

use common::CommonError;

#[derive(Debug)]
pub(crate) enum Socks5AuthMethod {
    NoAuthenticationRequired,
    GSSAPI,
    UsernameAndPassword,
    IanaAssigned,
    ReservedForPrivateMethods,
    NoAcceptableMethods,
}

impl From<u8> for Socks5AuthMethod {
    fn from(v: u8) -> Self {
        match v {
            0 => Socks5AuthMethod::NoAuthenticationRequired,
            1 => Socks5AuthMethod::GSSAPI,
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
            Socks5AuthMethod::GSSAPI => 1,
            Socks5AuthMethod::UsernameAndPassword => 2,
            Socks5AuthMethod::IanaAssigned => 3,
            Socks5AuthMethod::ReservedForPrivateMethods => 8,
            Socks5AuthMethod::NoAcceptableMethods => 16,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Socks5ConnectCommandType {
    Connect,
    Bind,
    UdpAssociate,
}

impl TryFrom<u8> for Socks5ConnectCommandType {
    type Error = CommonError;
    fn try_from(v: u8) -> Result<Self, CommonError> {
        match v {
            1 => Ok(Socks5ConnectCommandType::Connect),
            2 => Ok(Socks5ConnectCommandType::Bind),
            3 => Ok(Socks5ConnectCommandType::UdpAssociate),
            unknown_type => {
                error!(
                    "Fail to decode socks 5 connect request type: {}",
                    unknown_type
                );
                Err(CommonError::CodecError)
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum Socks5ConnectCommandResultStatus {
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

impl From<u8> for Socks5ConnectCommandResultStatus {
    fn from(v: u8) -> Self {
        match v {
            0 => Socks5ConnectCommandResultStatus::Succeeded,
            1 => Socks5ConnectCommandResultStatus::Failure,
            2 => Socks5ConnectCommandResultStatus::ConnectionNotAllowedByRuleSet,
            3 => Socks5ConnectCommandResultStatus::NetworkUnReachable,
            4 => Socks5ConnectCommandResultStatus::HostUnReachable,
            5 => Socks5ConnectCommandResultStatus::ConnectionRefused,
            6 => Socks5ConnectCommandResultStatus::TtlExpired,
            7 => Socks5ConnectCommandResultStatus::CommandNotSupported,
            8 => Socks5ConnectCommandResultStatus::AddressTypeNotSupported,
            9 => Socks5ConnectCommandResultStatus::Unassigned,
            unknown_status => {
                error!(
                    "Fail to decode socks 5 connect response status: {}",
                    unknown_status
                );
                Socks5ConnectCommandResultStatus::Failure
            }
        }
    }
}

impl From<Socks5ConnectCommandResultStatus> for u8 {
    fn from(value: Socks5ConnectCommandResultStatus) -> Self {
        match value {
            Socks5ConnectCommandResultStatus::Succeeded => 0,
            Socks5ConnectCommandResultStatus::Failure => 1,
            Socks5ConnectCommandResultStatus::ConnectionNotAllowedByRuleSet => 2,
            Socks5ConnectCommandResultStatus::NetworkUnReachable => 3,
            Socks5ConnectCommandResultStatus::HostUnReachable => 4,
            Socks5ConnectCommandResultStatus::ConnectionRefused => 5,
            Socks5ConnectCommandResultStatus::TtlExpired => 6,
            Socks5ConnectCommandResultStatus::CommandNotSupported => 7,
            Socks5ConnectCommandResultStatus::AddressTypeNotSupported => 8,
            Socks5ConnectCommandResultStatus::Unassigned => 9,
        }
    }
}

#[derive(Debug)]
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
            }
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
            }
            Self::Domain(host, port) => {
                format!("{}:{}", host, port)
            }
        }
    }
}

impl TryFrom<&mut Bytes> for Socks5Addr {
    type Error = CommonError;
    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if !value.has_remaining() {
            error!("Fail to parse socks5 address because of no remaining in bytes buffer.");
            return Err(CommonError::CodecError);
        }
        let address_type = value.get_u8();
        let address = match address_type {
            1 => {
                if value.remaining() < 6 {
                    error!("Fail to parse socks5 address (IpV4) because of not enough remaining in bytes buffer.");
                    return Err(CommonError::CodecError);
                }
                let mut addr_content = [0u8; 4];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                Socks5Addr::IpV4(addr_content, port)
            }
            4 => {
                if value.remaining() < 18 {
                    error!("Fail to parse socks5 address (IpV6) because of not enough remaining in bytes buffer.");
                    return Err(CommonError::CodecError);
                }
                let mut addr_content = [0u8; 16];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                Socks5Addr::IpV6(addr_content, port)
            }
            3 => {
                if value.remaining() < 1 {
                    error!("Fail to parse socks5 address(Domain) because of not enough remaining in bytes buffer.");
                    return Err(CommonError::CodecError);
                }
                let domain_name_length = value.get_u8() as usize;
                if value.remaining() < domain_name_length + 2 {
                    error!("Fail to parse socks5 address(Domain) because of not enough remaining in bytes buffer, require: {}.", domain_name_length+2);
                    return Err(CommonError::CodecError);
                }
                let domain_name_bytes = value.copy_to_bytes(domain_name_length);
                let domain_name = match String::from_utf8(domain_name_bytes.to_vec()) {
                    Ok(v) => v,
                    Err(e) => {
                        error!(
                            "Fail to parse socks5 address(Domain) because of error: {:#?}.",
                            e
                        );
                        return Err(CommonError::CodecError);
                    }
                };
                let port = value.get_u16();
                Socks5Addr::Domain(domain_name, port)
            }
            unknown_addr_type => {
                error!("Fail to decode socks 5 address type: {}", unknown_addr_type);
                return Err(CommonError::CodecError);
            }
        };
        Ok(address)
    }
}

impl TryFrom<Bytes> for Socks5Addr {
    type Error = CommonError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        let value_mut_ref = &mut value;
        value_mut_ref.try_into()
    }
}
impl TryFrom<&mut BytesMut> for Socks5Addr {
    type Error = CommonError;

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
            }
            Socks5Addr::IpV6(addr_content, port) => {
                result.put_u8(4);
                result.put_slice(&addr_content);
                result.put_u16(port);
            }
            Socks5Addr::Domain(addr_content, port) => {
                result.put_u8(3);
                result.put_u8(addr_content.len() as u8);
                result.put_slice(&addr_content.as_bytes());
                result.put_u16(port);
            }
        }
        result.into()
    }
}

#[derive(Debug)]
pub(crate) struct Socks5AuthCommand {
    pub version: u8,
    pub method_number: u8,
    pub methods: Vec<Socks5AuthMethod>,
}

impl Socks5AuthCommand {
    pub fn new(method_number: u8, methods: Vec<Socks5AuthMethod>) -> Self {
        Socks5AuthCommand {
            version: 5,
            method_number,
            methods,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Socks5AuthCommandResult {
    pub version: u8,
    pub method: Socks5AuthMethod,
}

impl Socks5AuthCommandResult {
    pub fn new(method: Socks5AuthMethod) -> Self {
        Socks5AuthCommandResult {
            version: 5u8,
            method,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Socks5ConnectCommand {
    pub version: u8,
    pub request_type: Socks5ConnectCommandType,
    pub source_address: Socks5Addr,
    pub dest_address: Socks5Addr,
}

impl Socks5ConnectCommand {
    pub fn new(
        request_type: Socks5ConnectCommandType,
        source_address: Socks5Addr,
        dest_address: Socks5Addr,
    ) -> Self {
        Socks5ConnectCommand {
            version: 5,
            request_type,
            source_address,
            dest_address,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Socks5ConnectCommandResult {
    pub version: u8,
    pub status: Socks5ConnectCommandResultStatus,
    pub bind_address: Option<Socks5Addr>,
}

impl Socks5ConnectCommandResult {
    pub fn new(status: Socks5ConnectCommandResultStatus, bind_address: Option<Socks5Addr>) -> Self {
        Socks5ConnectCommandResult {
            version: 5,
            status,
            bind_address,
        }
    }
}
/////////////////////////////////////////////////////////
/////////////////////////////////////////////////////////
/////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) struct Socks5UdpDataRequest {
    pub frag: u8,
    pub address: Socks5Addr,
    pub data: Bytes,
}

impl TryFrom<Bytes> for Socks5UdpDataRequest {
    type Error = CommonError;

    fn try_from(mut bytes: Bytes) -> Result<Self, Self::Error> {
        bytes.get_u16();
        let frag = bytes.get_u8();
        let address: Socks5Addr = match (&mut bytes).try_into() {
            Err(e) => {
                error!(
                    "Fail to decode socks5 udp data request because of error: {:#?}",
                    e
                );
                return Err(CommonError::CodecError);
            }
            Ok(v) => v,
        };
        Ok(Self {
            frag,
            address,
            data: bytes,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Socks5UdpDataResponse {
    frag: u8,
    source_address: Socks5Addr,
    dest_address: Socks5Addr,
    data: Bytes,
}

impl Socks5UdpDataResponse {
    pub fn new(
        frag: u8,
        source_address: Socks5Addr,
        dest_address: Socks5Addr,
        data: Bytes,
    ) -> Self {
        Self {
            frag,
            source_address,
            dest_address,
            data,
        }
    }
}

impl From<Socks5UdpDataResponse> for Bytes {
    fn from(value: Socks5UdpDataResponse) -> Self {
        let mut result = BytesMut::new();
        result.put_u16(0);
        result.put_u8(value.frag);
        match value.source_address {
            Socks5Addr::IpV4(address_content, port) => {
                result.put_u8(1);
                address_content.iter().for_each(|item| {
                    result.put_u8(*item);
                });
                result.put_u16(port);
            }
            Socks5Addr::IpV6(address_content, port) => {
                result.put_u8(4);
                address_content.iter().for_each(|item| {
                    result.put_u8(*item);
                });
                result.put_u16(port);
            }
            Socks5Addr::Domain(address_content, port) => {
                result.put_u8(3);
                result.put_slice(address_content.as_bytes());
                result.put_u16(port);
            }
        }
        result.put_slice(value.data.chunk());
        result.into()
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