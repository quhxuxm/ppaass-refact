use std::net::{IpAddr, SocketAddr};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::error;

use crate::{error::CommonError, util::generate_uuid};

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

impl ToString for NetAddress {
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
                //A ip v4 address is 6 bytes: 4 bytes for host, 2 bytes for port
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
            }
            NetAddress::IpV6(addr_content, port) => {
                result.put_u8(IPV6_TYPE);
                result.put_slice(&addr_content);
                result.put_u16(port);
            }
            NetAddress::Domain(addr_content, port) => {
                result.put_u8(DOMAIN_TYPE);
                result.put_u32(addr_content.len() as u32);
                result.put_slice(addr_content.as_bytes());
                result.put_u16(port);
            }
        }
        result.into()
    }
}

#[derive(Debug)]
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
}

impl From<AgentMessagePayloadTypeValue> for u8 {
    fn from(value: AgentMessagePayloadTypeValue) -> Self {
        match value {
            AgentMessagePayloadTypeValue::TcpConnect => 110,
            AgentMessagePayloadTypeValue::TcpData => 111,
            AgentMessagePayloadTypeValue::UdpAssociate => 120,
            AgentMessagePayloadTypeValue::UdpData => 121,
        }
    }
}

#[derive(Debug)]
pub enum ProxyMessagePayloadTypeValue {
    TcpConnectSuccess,
    TcpConnectFail,
    TcpData,
    TcpDataRelayFail,
    UdpAssociateSuccess,
    UdpAssociateFail,
    UdpData,
    UdpDataRelayFail,
}

impl From<ProxyMessagePayloadTypeValue> for u8 {
    fn from(value: ProxyMessagePayloadTypeValue) -> Self {
        match value {
            ProxyMessagePayloadTypeValue::TcpConnectSuccess => 210,
            ProxyMessagePayloadTypeValue::TcpConnectFail => 211,
            ProxyMessagePayloadTypeValue::TcpData => 212,
            ProxyMessagePayloadTypeValue::TcpDataRelayFail => 213,
            ProxyMessagePayloadTypeValue::UdpAssociateSuccess => 221,
            ProxyMessagePayloadTypeValue::UdpAssociateFail => 222,
            ProxyMessagePayloadTypeValue::UdpDataRelayFail => 223,
            ProxyMessagePayloadTypeValue::UdpData => 224,
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
    type Error = CommonError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            210 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::TcpConnectSuccess,
            )),
            211 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::TcpConnectFail,
            )),
            212 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::TcpData,
            )),
            213 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::TcpDataRelayFail,
            )),
            221 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::UdpAssociateSuccess,
            )),
            222 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::UdpAssociateFail,
            )),
            223 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::UdpDataRelayFail,
            )),
            224 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::UdpData,
            )),

            110 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::TcpConnect,
            )),
            111 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::TcpData,
            )),
            120 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::UdpAssociate,
            )),
            121 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::UdpData,
            )),

            invalid_type => {
                error!("Fail to parse payload type: {}", invalid_type);
                Err(CommonError::CodecError)
            }
        }
    }
}

impl TryFrom<&mut Bytes> for PayloadEncryptionType {
    type Error = CommonError;

    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < 5 {
            error!("Fail to parse PayloadEncryptionType because of no remining in byte buffer.");
            return Err(CommonError::CodecError);
        }
        let enc_type_value = value.get_u8();
        let enc_type_token_length = value.get_u32() as usize;
        if value.remaining() < enc_type_token_length {
            error!("Fail to parse PayloadEncryptionType because of no remining in byte buffer.");
            return Err(CommonError::CodecError);
        }
        let enc_token = value.copy_to_bytes(enc_type_token_length);
        match enc_type_value {
            ENCRYPTION_TYPE_PLAIN => Ok(PayloadEncryptionType::Plain),
            ENCRYPTION_TYPE_BLOWFISH => Ok(PayloadEncryptionType::Blowfish(enc_token)),
            ENCRYPTION_TYPE_AES => Ok(PayloadEncryptionType::Aes(enc_token)),
            invalid_type => {
                error!(
                    "Fail to parse PayloadEncryptionType because of invalid type: {}.",
                    invalid_type
                );
                Err(CommonError::CodecError)
            }
        }
    }
}

impl TryFrom<Bytes> for PayloadEncryptionType {
    type Error = CommonError;

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
            }
            PayloadEncryptionType::Blowfish(token) => {
                result.put_u8(ENCRYPTION_TYPE_BLOWFISH);
                result.put_u32(token.len() as u32);
                result.put(token);
            }
            PayloadEncryptionType::Aes(token) => {
                result.put_u8(ENCRYPTION_TYPE_AES);
                result.put_u32(token.len() as u32);
                result.put(token);
            }
        }
        result.into()
    }
}

#[derive(Debug)]
pub struct MessagePayload {
    /// The source address
    pub source_address: NetAddress,
    /// The target address
    pub target_address: NetAddress,
    /// The payload type
    pub payload_type: PayloadType,
    /// The data
    pub data: Bytes,
}

impl MessagePayload {
    pub fn new(
        source_address: NetAddress,
        target_address: NetAddress,
        payload_type: PayloadType,
        data: Bytes,
    ) -> Self {
        Self {
            source_address,
            target_address,
            payload_type,
            data,
        }
    }
}

impl From<MessagePayload> for Bytes {
    fn from(value: MessagePayload) -> Self {
        let mut result = BytesMut::new();
        result.put_u8(value.payload_type.into());
        result.put::<Bytes>(value.source_address.into());
        result.put::<Bytes>(value.target_address.into());
        result.put_u64(value.data.len() as u64);
        result.put(value.data);
        result.into()
    }
}

impl TryFrom<Bytes> for MessagePayload {
    type Error = CommonError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < 1 {
            error!("Fail to parse message payload because of no remaining");
            return Err(CommonError::CodecError);
        }
        let payload_type: PayloadType = match value.get_u8().try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message payload because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        let source_address: NetAddress = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse source address because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        let target_address: NetAddress = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse target address because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        if value.remaining() < 8 {
            error!("Fail to parse message payload because of no remaining");
            return Err(CommonError::CodecError);
        }
        let data_length = value.get_u64() as usize;
        if value.remaining() < data_length {
            error!("Fail to parse message payload because of no remaining");
            return Err(CommonError::CodecError);
        }
        let data = value.copy_to_bytes(data_length);
        Ok(Self {
            payload_type,
            source_address,
            target_address,
            data,
        })
    }
}

/// The message
#[derive(Debug)]
pub struct Message {
    /// The message id
    pub id: String,
    /// The message id that this message reference to
    pub ref_id: Option<String>,
    /// The user token
    pub user_token: String,
    /// The payload encryption type
    pub payload_encryption_type: PayloadEncryptionType,
    /// The payload
    pub payload: Option<Bytes>,
}

impl Message {
    pub fn new_random_id(
        ref_id: Option<String>,
        user_token: String,
        payload_encryption_type: PayloadEncryptionType,
        payload: Option<Bytes>,
    ) -> Self {
        Self {
            id: generate_uuid(),
            ref_id,
            user_token,
            payload_encryption_type,
            payload,
        }
    }
    pub fn new(
        id: String,
        ref_id: Option<String>,
        user_token: String,
        payload_encryption_type: PayloadEncryptionType,
        payload: Option<Bytes>,
    ) -> Self {
        Self {
            id,
            ref_id,
            user_token,
            payload_encryption_type,
            payload,
        }
    }
}

impl TryFrom<Bytes> for Message {
    type Error = CommonError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < 4 {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let id_length = value.get_u32() as usize;
        if value.remaining() < id_length {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let id_bytes = value.copy_to_bytes(id_length as usize);
        let id = match String::from_utf8(id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        if value.remaining() < 8 {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let ref_id_length = value.get_u32() as usize;
        if value.remaining() < ref_id_length {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let ref_id_bytes = value.copy_to_bytes(ref_id_length as usize);
        let ref_id = match String::from_utf8(ref_id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        if value.remaining() < 8 {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let user_token_length = value.get_u64() as usize;
        if value.remaining() < user_token_length {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let user_token_bytes = value.copy_to_bytes(user_token_length as usize);
        let user_token = match String::from_utf8(user_token_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        let payload_encryption_type: PayloadEncryptionType = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            }
        };
        if value.remaining() < 8 {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        let payload_length = value.get_u64() as usize;
        let payload = if payload_length == 0 {
            None
        } else if value.remaining() >= payload_length {
            let payload_bytes = value.copy_to_bytes(payload_length as usize);
            Some(payload_bytes)
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::CodecError);
        };
        Ok(Self {
            id,
            ref_id: Some(ref_id),
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
            }
            None => {
                result.put_u32(0);
            }
        }
        result.put_u64(value.user_token.len() as u64);
        result.put_slice(value.user_token.as_bytes());
        result.put::<Bytes>(value.payload_encryption_type.into());

        match value.payload {
            None => {
                result.put_u64(0);
            }
            Some(p) => {
                result.put_u64(p.len() as u64);
                result.put(p);
            }
        }
        result.into()
    }
}
