use crate::{error::CommonError, util::generate_uuid};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::error;

use super::address::NetAddress;

const ENCRYPTION_TYPE_PLAIN: u8 = 0;
const ENCRYPTION_TYPE_BLOWFISH: u8 = 1;
const ENCRYPTION_TYPE_AES: u8 = 2;

#[derive(Debug)]
pub enum PayloadEncryptionType {
    Plain,
    Blowfish(Bytes),
    Aes(Bytes),
}

#[derive(Debug)]
pub enum AgentMessagePayloadTypeValue {
    TcpConnect,
    TcpConnectionClose,
    TcpData,
    UdpAssociate,
    UdpData,
}

impl From<AgentMessagePayloadTypeValue> for u8 {
    fn from(value: AgentMessagePayloadTypeValue) -> Self {
        match value {
            AgentMessagePayloadTypeValue::TcpConnect => 110,
            AgentMessagePayloadTypeValue::TcpData => 111,
            AgentMessagePayloadTypeValue::TcpConnectionClose => 112,
            AgentMessagePayloadTypeValue::UdpAssociate => 120,
            AgentMessagePayloadTypeValue::UdpData => 121,
        }
    }
}

#[derive(Debug)]
pub enum ProxyMessagePayloadTypeValue {
    TcpConnectSuccess,
    TcpConnectFail,
    TcpConnectionClose,
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
            ProxyMessagePayloadTypeValue::TcpConnectionClose => 214,
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
            214 => Ok(PayloadType::ProxyPayload(
                ProxyMessagePayloadTypeValue::TcpConnectionClose,
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
            112 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::TcpConnectionClose,
            )),
            120 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::UdpAssociate,
            )),
            121 => Ok(PayloadType::AgentPayload(
                AgentMessagePayloadTypeValue::UdpData,
            )),

            invalid_type => {
                error!("Fail to parse payload type: {}", invalid_type);
                Err(CommonError::FailToParsePayloadType)
            }
        }
    }
}

impl TryFrom<&mut Bytes> for PayloadEncryptionType {
    type Error = CommonError;

    fn try_from(value: &mut Bytes) -> Result<Self, Self::Error> {
        if value.remaining() < 5 {
            error!("Fail to parse PayloadEncryptionType because of no remining in byte buffer.");
            return Err(CommonError::FailToParsePayloadEncryptionType);
        }
        let enc_type_value = value.get_u8();
        let enc_type_token_length = value.get_u32() as usize;
        if value.remaining() < enc_type_token_length {
            error!("Fail to parse PayloadEncryptionType because of no remining in byte buffer.");
            return Err(CommonError::FailToParsePayloadEncryptionType);
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
                Err(CommonError::FailToParsePayloadEncryptionType)
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
    source_address: NetAddress,
    /// The target address
    target_address: NetAddress,
    /// The payload type
    payload_type: PayloadType,
    /// The data
    data: Bytes,
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
            return Err(CommonError::FailToParsePayload);
        }
        let payload_type: PayloadType = match value.get_u8().try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message payload because of error: {:#?}", e);
                return Err(CommonError::FailToParsePayloadType);
            }
        };
        let source_address: NetAddress = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse source address because of error: {:#?}", e);
                return Err(CommonError::FailToParseNetAddress);
            }
        };
        let target_address: NetAddress = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse target address because of error: {:#?}", e);
                return Err(CommonError::FailToParseNetAddress);
            }
        };
        if value.remaining() < 8 {
            error!("Fail to parse message payload because of no remaining");
            return Err(CommonError::FailToParsePayload);
        }
        let data_length = value.get_u64() as usize;
        if value.remaining() < data_length {
            error!("Fail to parse message payload because of no remaining");
            return Err(CommonError::FailToParsePayload);
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
    id: String,
    /// The message id that this message reference to
    ref_id: Option<String>,
    /// The user token
    user_token: String,
    /// The payload encryption type
    payload_encryption_type: PayloadEncryptionType,
    /// The payload
    payload: Option<MessagePayload>,
}

impl TryFrom<Bytes> for Message {
    type Error = CommonError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
        let id_length = if value.remaining() >= 4 {
            value.get_u32() as usize
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };

        let id_bytes = if value.remaining() >= id_length {
            value.copy_to_bytes(id_length as usize)
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };
        let id = match String::from_utf8(id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::FailToParseMessage);
            }
        };
        let ref_id_length = if value.remaining() >= 8 {
            value.get_u32() as usize
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };
        let ref_id_bytes = if value.remaining() >= ref_id_length {
            value.copy_to_bytes(ref_id_length as usize)
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };
        let ref_id = match String::from_utf8(ref_id_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::FailToParseMessage);
            }
        };

        let user_token_length = if value.remaining() >= 8 {
            value.get_u64() as usize
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };
        let user_token_bytes = if value.remaining() >= user_token_length {
            value.copy_to_bytes(user_token_length as usize)
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };
        let user_token = match String::from_utf8(user_token_bytes.to_vec()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::FailToParseMessage);
            }
        };

        let payload_encryption_type: PayloadEncryptionType = match (&mut value).try_into() {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse message because of error: {:#?}", e);
                return Err(CommonError::FailToParseMessage);
            }
        };

        let payload_length = if value.remaining() >= 8 {
            value.get_u64() as usize
        } else {
            error!("Fail to parse message because of no remaining");
            return Err(CommonError::FailToParseMessage);
        };

        let payload = if payload_length == 0 {
            None
        } else {
            if value.remaining() >= payload_length {
                let payload_bytes = value.copy_to_bytes(payload_length as usize);
                let payload: MessagePayload = match payload_bytes.try_into() {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Fail to parse message because of no error: {:#?}", e);
                        return Err(CommonError::FailToParseMessage);
                    }
                };
                Some(payload)
            } else {
                error!("Fail to parse message because of no remaining");
                return Err(CommonError::FailToParseMessage);
            }
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
                result.put::<Bytes>(p.into());
            }
        }
        result.into()
    }
}

pub struct MessageBuilder {
    /// The message id
    id: String,
    /// The message id that this message reference to
    ref_id: Option<String>,
    /// The user token
    user_token: String,
    /// The payload encryption type
    payload_encryption_type: PayloadEncryptionType,
    /// The payload
    payload: Option<MessagePayload>,
}

impl MessageBuilder {
    pub fn new(user_token: String, payload_encryption_type: PayloadEncryptionType) -> Self {
        Self {
            id: generate_uuid(),
            ref_id: None,
            user_token,
            payload_encryption_type,
            payload: None,
        }
    }

    pub fn ref_id(mut self, ref_id: String) -> Self {
        self.ref_id = Some(ref_id);
        self
    }

    pub fn user_token(mut self, user_token: String) -> Self {
        self.user_token = user_token;
        self
    }

    pub fn payload(mut self, payload: MessagePayload) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn build(self) -> Message {
        Message {
            id: self.id,
            ref_id: self.ref_id,
            user_token: self.user_token,
            payload_encryption_type: self.payload_encryption_type,
            payload: self.payload,
        }
    }
}
