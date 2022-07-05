#![allow(unused)]
use std::{
    collections::vec_deque::Iter,
    fmt::{Debug, Display, Formatter},
    io::{Error, ErrorKind},
    mem::size_of,
    pin::Pin,
    task::{Context, Poll},
};
use std::{
    io::Cursor,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
};
use std::{net::ToSocketAddrs, sync::Arc};
use std::{ops::Deref, str::FromStr};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use futures::{Stream, TryStream};
use pin_project::pin_project;
use pretty_hex::*;
use rsa::pkcs8::der::bigint::generic_array::typenum::PowerOfTwo;
use tracing::error;

use crate::NetAddress::IpV4;
use crate::{error::PpaassError, util::generate_uuid};

const ENCRYPTION_TYPE_PLAIN: u8 = 0;
const ENCRYPTION_TYPE_BLOWFISH: u8 = 1;
const ENCRYPTION_TYPE_AES: u8 = 2;

const IPV4_TYPE: u8 = 0;
const IPV6_TYPE: u8 = 1;
const DOMAIN_TYPE: u8 = 2;

/// The net address
#[derive(Debug, Clone)]
pub enum NetAddress {
    /// Ip v4 net address
    IpV4([u8; 4], u16),
    /// Ip v6 net address
    IpV6([u8; 16], u16),
    /// Domain net address
    Domain(String, u16),
}

impl Default for NetAddress {
    fn default() -> Self {
        IpV4([0, 0, 0, 0], 0)
    }
}

pub struct SocketAddrIter {
    elements: Vec<SocketAddr>,
    index: usize,
}

impl SocketAddrIter {
    pub fn new(elements: Vec<SocketAddr>) -> Self {
        Self { elements, index: 0 }
    }
}

impl Iterator for SocketAddrIter {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        let mut result = self.elements.get(self.index);
        self.index += 1;
        result.map(|item| *item)
    }
}

impl ToSocketAddrs for NetAddress {
    type Iter = SocketAddrIter;

    fn to_socket_addrs(&self) -> std::io::Result<Self::Iter> {
        match self {
            Self::IpV4(ip, port) => {
                let socket_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), *port));
                let elements = vec![socket_addr];
                Ok(SocketAddrIter::new(elements))
            },
            Self::IpV6(ip, port) => {
                let mut cursor = Cursor::new(ip);
                let socket_addr = SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::new(
                        cursor.get_u16(),
                        cursor.get_u16(),
                        cursor.get_u16(),
                        cursor.get_u16(),
                        cursor.get_u16(),
                        cursor.get_u16(),
                        cursor.get_u16(),
                        cursor.get_u16(),
                    ),
                    *port,
                    0,
                    0,
                ));
                let elements = vec![socket_addr];
                Ok(SocketAddrIter::new(elements))
            },
            Self::Domain(host, port) => {
                let addresses = format!("{}:{}", host, port).to_socket_addrs()?.collect::<Vec<_>>();
                Ok(SocketAddrIter::new(addresses))
            },
        }
    }
}

impl ToString for NetAddress {
    fn to_string(&self) -> String {
        match self {
            Self::IpV4(ip_content, port) => {
                format!("{}.{}.{}.{}:{}", ip_content[0], ip_content[1], ip_content[2], ip_content[3], port)
            },
            Self::IpV6(ip_content, port) => {
                let mut cursor = Cursor::new(ip_content);
                format!(
                    "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{}",
                    cursor.get_u16(),
                    cursor.get_u16(),
                    cursor.get_u16(),
                    cursor.get_u16(),
                    cursor.get_u16(),
                    cursor.get_u16(),
                    cursor.get_u16(),
                    cursor.get_u16(),
                    port
                )
            },
            Self::Domain(host, port) => {
                format!("{}:{}", host, port)
            },
        }
    }
}
impl TryFrom<&mut Bytes> for NetAddress {
    type Error = PpaassError;

    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if !value.has_remaining() {
            error!("Fail to parse NetAddress because of no remaining in bytes buffer.");
            return Err(PpaassError::CodecError);
        }
        let address_type = value.get_u8();
        let address = match address_type {
            IPV4_TYPE => {
                //Convert the NetAddress::IpV4
                //A ip v4 address is 6 bytes: 4 bytes for host, 2 bytes for port
                if value.remaining() < 6 {
                    error!("Fail to parse NetAddress(IpV4) because of not enough remaining in bytes buffer.");
                    return Err(PpaassError::CodecError);
                }
                let mut addr_content = [0u8; 4];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                NetAddress::IpV4(addr_content, port)
            },
            IPV6_TYPE => {
                //Convert the NetAddress::IpV6
                //A ip v6 address is 18 bytes: 16 bytes for host, 2 bytes for port
                if value.remaining() < 18 {
                    error!("Fail to parse NetAddress(IpV6) because of not enough remaining in bytes buffer.");
                    return Err(PpaassError::CodecError);
                }
                let mut addr_content = [0u8; 16];
                addr_content.iter_mut().for_each(|item| {
                    *item = value.get_u8();
                });
                let port = value.get_u16();
                NetAddress::IpV6(addr_content, port)
            },
            DOMAIN_TYPE => {
                //Convert the NetAddress::Domain
                if value.remaining() < 4 {
                    error!("Fail to parse NetAddress(Domain) because of not enough remaining in bytes buffer.");
                    return Err(PpaassError::CodecError);
                }
                let host_name_length = value.get_u32() as usize;
                if value.remaining() < host_name_length + 2 {
                    error!(
                        "Fail to parse NetAddress(Domain) because of not enough remaining in bytes buffer, require: {}.",
                        host_name_length + 2
                    );
                    return Err(PpaassError::CodecError);
                }
                let host_bytes = value.split_to(host_name_length);
                let host = match String::from_utf8(host_bytes.to_vec()) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Fail to parse NetAddress(Domain) because of error: {:#?}.", e);
                        return Err(PpaassError::CodecError);
                    },
                };
                let port = value.get_u16();
                NetAddress::Domain(host, port)
            },
            invalid_address_type => {
                error!("Fail to parse NetAddress because of invalide address type {}.", invalid_address_type);
                return Err(PpaassError::CodecError);
            },
        };
        Ok(address)
    }
}

impl TryFrom<Bytes> for NetAddress {
    type Error = PpaassError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        let value_mut_ref: &mut Bytes = &mut value;
        value_mut_ref.try_into()
    }
}

impl TryFrom<NetAddress> for Vec<SocketAddr> {
    type Error = PpaassError;
    fn try_from(net_address: NetAddress) -> Result<Self, PpaassError> {
        match net_address {
            NetAddress::IpV4(ip, port) => {
                let socket_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), port));
                Ok(vec![socket_addr])
            },
            NetAddress::IpV6(ip, port) => {
                let mut ip_cursor = Cursor::new(ip);
                let socket_addr = SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::new(
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                        ip_cursor.get_u16(),
                    ),
                    port,
                    0,
                    0,
                ));
                Ok(vec![socket_addr])
            },
            NetAddress::Domain(host, port) => {
                let addresses = format!("{}:{}", host, port).to_socket_addrs()?.collect::<Vec<_>>();
                Ok(addresses)
            },
        }
    }
}

impl From<SocketAddr> for NetAddress {
    fn from(value: SocketAddr) -> Self {
        let ip_address = value.ip();
        match ip_address {
            IpAddr::V4(addr) => Self::IpV4(addr.octets(), value.port()),
            IpAddr::V6(addr) => Self::IpV6(addr.octets(), value.port()),
        }
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
            },
            NetAddress::IpV6(addr_content, port) => {
                result.put_u8(IPV6_TYPE);
                result.put_slice(&addr_content);
                result.put_u16(port);
            },
            NetAddress::Domain(addr_content, port) => {
                result.put_u8(DOMAIN_TYPE);
                result.put_u32(addr_content.len() as u32);
                result.put_slice(addr_content.as_bytes());
                result.put_u16(port);
            },
        }
        result.into()
    }
}

#[derive(Debug, Clone)]
pub enum PayloadEncryptionType {
    Plain,
    Blowfish(Bytes),
    Aes(Bytes),
}

#[derive(Debug)]
pub enum AgentMessagePayloadTypeValue {
    TcpConnect,
    TcpData,
    UdpAssociate,
    UdpData,
    Heartbeat,
}

impl From<AgentMessagePayloadTypeValue> for u8 {
    fn from(value: AgentMessagePayloadTypeValue) -> Self {
        match value {
            AgentMessagePayloadTypeValue::TcpConnect => 110,
            AgentMessagePayloadTypeValue::TcpData => 111,
            AgentMessagePayloadTypeValue::UdpAssociate => 120,
            AgentMessagePayloadTypeValue::UdpData => 121,
            AgentMessagePayloadTypeValue::Heartbeat => 130,
        }
    }
}

#[derive(Debug)]
pub enum ProxyMessagePayloadTypeValue {
    TcpConnectSuccess,
    TcpConnectFail,
    TcpData,
    UdpAssociateSuccess,
    UdpAssociateFail,
    UdpData,
    UdpDataRelayFail,
    HeartbeatSuccess,
}

impl From<ProxyMessagePayloadTypeValue> for u8 {
    fn from(value: ProxyMessagePayloadTypeValue) -> Self {
        match value {
            ProxyMessagePayloadTypeValue::TcpConnectSuccess => 210,
            ProxyMessagePayloadTypeValue::TcpConnectFail => 211,
            ProxyMessagePayloadTypeValue::TcpData => 212,
            ProxyMessagePayloadTypeValue::UdpAssociateSuccess => 221,
            ProxyMessagePayloadTypeValue::UdpAssociateFail => 222,
            ProxyMessagePayloadTypeValue::UdpDataRelayFail => 223,
            ProxyMessagePayloadTypeValue::UdpData => 224,
            ProxyMessagePayloadTypeValue::HeartbeatSuccess => 230,
        }
    }
}

#[derive(Debug)]
pub enum PayloadType {
    AgentPayload(AgentMessagePayloadTypeValue),
    ProxyPayload(ProxyMessagePayloadTypeValue),
}

impl From<PayloadType> for u8 {
    fn from(value: PayloadType) -> Self {
        match value {
            PayloadType::AgentPayload(val) => val.into(),
            PayloadType::ProxyPayload(val) => val.into(),
        }
    }
}

impl TryFrom<u8> for PayloadType {
    type Error = PpaassError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            210 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectSuccess)),
            211 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpConnectFail)),
            212 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::TcpData)),
            221 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpAssociateSuccess)),
            222 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpAssociateFail)),
            223 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpDataRelayFail)),
            224 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::UdpData)),
            230 => Ok(PayloadType::ProxyPayload(ProxyMessagePayloadTypeValue::HeartbeatSuccess)),

            110 => Ok(PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpConnect)),
            111 => Ok(PayloadType::AgentPayload(AgentMessagePayloadTypeValue::TcpData)),
            120 => Ok(PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpAssociate)),
            121 => Ok(PayloadType::AgentPayload(AgentMessagePayloadTypeValue::UdpData)),
            130 => Ok(PayloadType::AgentPayload(AgentMessagePayloadTypeValue::Heartbeat)),

            invalid_type => {
                error!("Fail to parse payload type: {}", invalid_type);
                Err(PpaassError::CodecError)
            },
        }
    }
}

impl TryFrom<&mut Bytes> for PayloadEncryptionType {
    type Error = PpaassError;

    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < 5 {
            error!("Fail to parse PayloadEncryptionType because of no remining in byte buffer.");
            return Err(PpaassError::CodecError);
        }
        let enc_type_value = value.get_u8();
        let enc_type_token_length = value.get_u32() as usize;
        if value.remaining() < enc_type_token_length {
            error!("Fail to parse PayloadEncryptionType because of no remining in byte buffer.");
            return Err(PpaassError::CodecError);
        }
        let enc_token = value.split_to(enc_type_token_length);
        match enc_type_value {
            ENCRYPTION_TYPE_PLAIN => Ok(PayloadEncryptionType::Plain),
            ENCRYPTION_TYPE_BLOWFISH => Ok(PayloadEncryptionType::Blowfish(enc_token)),
            ENCRYPTION_TYPE_AES => Ok(PayloadEncryptionType::Aes(enc_token)),
            invalid_type => {
                error!("Fail to parse PayloadEncryptionType because of invalid type: {}.", invalid_type);
                Err(PpaassError::CodecError)
            },
        }
    }
}

impl TryFrom<Bytes> for PayloadEncryptionType {
    type Error = PpaassError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        let value_mut_ref: &mut Bytes = &mut value;
        value_mut_ref.try_into()
    }
}

impl From<PayloadEncryptionType> for Bytes {
    fn from(value: PayloadEncryptionType) -> Self {
        let mut result = BytesMut::new();
        match value {
            PayloadEncryptionType::Plain => {
                result.put_u8(ENCRYPTION_TYPE_PLAIN);
                result.put_u32(0);
            },
            PayloadEncryptionType::Blowfish(token) => {
                result.put_u8(ENCRYPTION_TYPE_BLOWFISH);
                result.put_u32(token.len() as u32);
                result.put(token);
            },
            PayloadEncryptionType::Aes(token) => {
                result.put_u8(ENCRYPTION_TYPE_AES);
                result.put_u32(token.len() as u32);
                result.put(token);
            },
        }
        result.into()
    }
}

pub struct MessagePayload {
    /// The source address
    pub source_address: Option<NetAddress>,
    /// The target address
    pub target_address: Option<NetAddress>,
    /// The payload type
    pub payload_type: PayloadType,
    /// The data
    pub data: Option<Bytes>,
}

impl Debug for MessagePayload {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagePayload")
            .field("source_address", &self.source_address)
            .field("target_address", &self.target_address)
            .field("payload_type", &self.payload_type)
            .field("data", &format!("\n\n{}\n", pretty_hex(self.data.as_ref().unwrap_or(&Bytes::new()))))
            .finish()
    }
}

impl From<MessagePayload> for Bytes {
    fn from(value: MessagePayload) -> Self {
        let mut result = BytesMut::new();
        result.put_u8(value.payload_type.into());
        match value.source_address {
            None => {
                result.put_u8(0);
            },
            Some(source_address) => {
                result.put_u8(1);
                result.put::<Bytes>(source_address.into());
            },
        }
        match value.target_address {
            None => {
                result.put_u8(0);
            },
            Some(target_address) => {
                result.put_u8(1);
                result.put::<Bytes>(target_address.into());
            },
        }
        match value.data {
            None => {
                result.put_u64(0);
                result.into()
            },
            Some(data_content) => {
                result.put_u64(data_content.len() as u64);
                result.put(data_content);
                result.into()
            },
        }
    }
}

impl TryFrom<Bytes> for MessagePayload {
    type Error = PpaassError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < 1 {
            error!("Fail to parse message payload because of no remaining");
            return Err(PpaassError::CodecError);
        }
        let payload_type: PayloadType = match value.get_u8().try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message payload because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        if value.remaining() < 1 {
            error!("Fail to parse message payload because of no remaining");
            return Err(PpaassError::CodecError);
        }
        let source_address_exist = value.get_u8();
        let source_address = if source_address_exist == 0 {
            None
        } else {
            Some(match (&mut value).try_into() {
                Ok(v) => v,
                Err(e) => {
                    error!("Fail to parse source address because of error: {:#?}", e);
                    return Err(PpaassError::CodecError);
                },
            })
        };
        if value.remaining() < 1 {
            error!("Fail to parse message payload because of no remaining");
            return Err(PpaassError::CodecError);
        }
        let target_address_exist = value.get_u8();
        let target_address = if target_address_exist == 0 {
            None
        } else {
            Some(match (&mut value).try_into() {
                Ok(v) => v,
                Err(e) => {
                    error!("Fail to parse target address because of error: {:#?}", e);
                    return Err(PpaassError::CodecError);
                },
            })
        };
        if value.remaining() < 8 {
            error!("Fail to parse message payload because of no remaining");
            return Err(PpaassError::CodecError);
        }
        let data_length = value.get_u64() as usize;
        if data_length == 0 {
            return Ok(Self {
                payload_type,
                source_address,
                target_address,
                data: None,
            });
        }
        if value.remaining() < data_length {
            error!("Fail to parse message payload because of no remaining");
            return Err(PpaassError::CodecError);
        }
        let data = value.split_to(data_length);
        Ok(Self {
            payload_type,
            source_address,
            target_address,
            data: Some(data),
        })
    }
}

/// The message
pub struct Message {
    /// The message id
    pub id: String,
    /// The message id that this message reference to
    pub ref_id: Option<String>,
    /// The connection id that initial this message
    pub connection_id: Option<String>,
    /// The user token
    pub user_token: String,
    /// The payload encryption type
    pub payload_encryption_type: PayloadEncryptionType,
    /// The payload
    pub payload: Option<Bytes>,
}

impl Debug for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("id", &self.id)
            .field("ref_id", &self.ref_id)
            .field("connection_id", &self.connection_id)
            .field("user_token", &self.user_token)
            .field("payload_encryption_type", &"[...omit...]")
            .field("payload", &"[... omit ...]")
            .finish()
    }
}

impl Message {
    pub fn new_random_id<R, U, PE, P>(ref_id: Option<R>, connection_id: Option<String>, user_token: U, payload_encryption_type: PE, payload: Option<P>) -> Self
    where
        R: Into<String>,
        U: Into<String>,
        P: Into<Bytes>,
        PE: Into<PayloadEncryptionType>,
    {
        Self {
            id: generate_uuid(),
            ref_id: match ref_id {
                None => None,
                Some(v) => Some(v.into()),
            },
            connection_id,
            user_token: user_token.into(),
            payload_encryption_type: payload_encryption_type.into(),
            payload: match payload {
                None => None,
                Some(v) => Some(v.into()),
            },
        }
    }
    pub fn new<I, R, U, PE, P>(id: I, ref_id: Option<R>, connection_id: Option<String>, user_token: U, payload_encryption_type: PE, payload: Option<P>) -> Self
    where
        I: Into<String>,
        R: Into<String>,
        U: Into<String>,
        P: Into<Bytes>,
        PE: Into<PayloadEncryptionType>,
    {
        Self {
            id: id.into(),
            ref_id: match ref_id {
                None => None,
                Some(v) => Some(v.into()),
            },
            connection_id,
            user_token: user_token.into(),
            payload_encryption_type: payload_encryption_type.into(),
            payload: match payload {
                None => None,
                Some(v) => Some(v.into()),
            },
        }
    }
}

impl TryFrom<Vec<u8>> for Message {
    type Error = PpaassError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let value_bytes = Bytes::from(value);
        value_bytes.try_into()
    }
}

impl TryFrom<BytesMut> for Message {
    type Error = PpaassError;

    fn try_from(value: BytesMut) -> Result<Self, Self::Error> {
        value.freeze().try_into()
    }
}

impl TryFrom<Bytes> for Message {
    type Error = PpaassError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < size_of::<u32>() {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let id_length = value.get_u32() as usize;
        if value.remaining() < id_length {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let id_bytes = value.split_to(id_length as usize);
        let id = match String::from_utf8(id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        if value.remaining() < size_of::<u32>() {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let ref_id_length = value.get_u32() as usize;
        if value.remaining() < ref_id_length {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let ref_id_bytes = value.split_to(ref_id_length as usize);
        let ref_id = match String::from_utf8(ref_id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        let connection_id_length = value.get_u32() as usize;
        if value.remaining() < connection_id_length {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let connection_id_bytes = value.split_to(connection_id_length as usize);
        let connection_id = match String::from_utf8(connection_id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        if value.remaining() < size_of::<u64>() {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let user_token_length = value.get_u64() as usize;
        if value.remaining() < user_token_length {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let user_token_bytes = value.split_to(user_token_length as usize);
        let user_token = match String::from_utf8(user_token_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        let payload_encryption_type: PayloadEncryptionType = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        if value.remaining() < size_of::<u64>() {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        let payload_length = value.get_u64() as usize;
        let payload = if payload_length == 0 {
            None
        } else if value.remaining() >= payload_length {
            let payload_bytes = value.split_to(payload_length as usize);
            Some(payload_bytes)
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(PpaassError::CodecError);
        };
        Ok(Self {
            id,
            ref_id: Some(ref_id),
            connection_id: Some(connection_id),
            user_token,
            payload_encryption_type,
            payload,
        })
    }
}

impl From<Message> for Bytes {
    fn from(value: Message) -> Self {
        let mut result = BytesMut::new();
        result.put_u32(value.id.len() as u32);
        result.put_slice(value.id.as_bytes());
        match value.ref_id {
            Some(v) => {
                result.put_u32(v.len() as u32);
                result.put_slice(v.as_bytes());
            },
            None => {
                result.put_u32(0);
            },
        }
        match value.connection_id {
            Some(v) => {
                result.put_u32(v.len() as u32);
                result.put_slice(v.as_bytes());
            },
            None => {
                result.put_u32(0);
            },
        }
        result.put_u64(value.user_token.len() as u64);
        result.put_slice(value.user_token.as_bytes());
        result.put::<Bytes>(value.payload_encryption_type.into());
        match value.payload {
            None => {
                result.put_u64(0);
            },
            Some(p) => {
                result.put_u64(p.len() as u64);
                result.put(p);
            },
        }
        result.into()
    }
}

#[pin_project]
pub struct MessageStream {
    #[pin]
    inner: Vec<Option<Message>>,
    index: usize,
}

impl MessageStream {
    fn new(messages: Vec<Message>) -> Self {
        let inner = messages.into_iter().map(|item| Some(item)).collect::<Vec<_>>();
        Self { inner, index: 0 }
    }
}

impl Stream for MessageStream {
    type Item = Result<Message, PpaassError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let mut index = *this.index;
        let mut inner_item = this.inner.get_mut().get_mut(index);
        inner_item.map_or(Poll::Ready(None), |value| {
            let message = value.take();
            index += 1;
            *this.index = index;
            let result = message.ok_or(PpaassError::IoError {
                source: Error::new(ErrorKind::InvalidData, "Fail to convert message stream because of item is a none value."),
            });
            return Poll::Ready(Some(result));
        })
    }
}

impl From<Vec<Message>> for MessageStream {
    fn from(messages: Vec<Message>) -> Self {
        MessageStream::new(messages)
    }
}
