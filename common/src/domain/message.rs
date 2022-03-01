use crate::{error::CommonError, util::generate_uuid};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::error;

#[derive(Debug)]
pub enum PayloadEncryptionType {
    Plain,
    Blowfish(Bytes),
    Aes(Bytes),
}

impl TryFrom<Bytes> for PayloadEncryptionType {
    type Error = CommonError;

    fn try_from(mut value: Bytes) -> Result<Self, Self::Error> {
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
            0 => Ok(PayloadEncryptionType::Plain),
            1 => Ok(PayloadEncryptionType::Blowfish(enc_token)),
            2 => Ok(PayloadEncryptionType::Aes(enc_token)),
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

impl From<PayloadEncryptionType> for Bytes {
    fn from(value: PayloadEncryptionType) -> Self {
        let mut result = BytesMut::new();
        match value {
            PayloadEncryptionType::Plain => {
                result.put_u8(0);
                result.put_u32(0);
            }
            PayloadEncryptionType::Blowfish(token) => {
                result.put_u8(1);
                result.put_u32(token.len() as u32);
                result.put(token);
            }
            PayloadEncryptionType::Aes(token) => {
                result.put_u8(2);
                result.put_u32(token.len() as u32);
                result.put(token);
            }
        }
        result.into()
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
    payload: Bytes,
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
    payload: Bytes,
}

impl MessageBuilder {
    pub fn new(user_token: String, payload_encryption_type: PayloadEncryptionType) -> Self {
        Self {
            id: generate_uuid(),
            ref_id: None,
            user_token,
            payload_encryption_type,
            payload: Bytes::new(),
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

    pub fn payload(mut self, payload: Bytes) -> Self {
        self.payload = payload;
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
