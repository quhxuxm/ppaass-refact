use std::{
    fmt::{Debug, Formatter},
    mem::size_of,
    sync::Arc,
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use lz4::block::{compress, decompress};
use pretty_hex::*;
use tokio_util::codec::{Decoder, Encoder};
use tracing::{debug, error};

use crate::crypto::{
    decrypt_with_aes, decrypt_with_blowfish, encrypt_with_aes, encrypt_with_blowfish,
    RsaCryptoFetcher,
};
use crate::{Message, PayloadEncryptionType, PpaassError};

const PPAASS_FLAG: &[u8] = "__PPAASS__".as_bytes();

enum DecodeStatus {
    Head,
    Data(bool, u64),
}

pub struct MessageCodec<T: RsaCryptoFetcher> {
    rsa_crypto_fetcher: Arc<T>,
    compress: bool,
    status: DecodeStatus,
}

impl<T> Debug for MessageCodec<T>
where
    T: RsaCryptoFetcher,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "MessageCodec: compress={}", self.compress)
    }
}

impl<T> MessageCodec<T>
where
    T: RsaCryptoFetcher,
{
    pub fn new(compress: bool, rsa_crypto_fetcher: Arc<T>) -> Self {
        Self {
            rsa_crypto_fetcher,
            compress,
            status: DecodeStatus::Head,
        }
    }
}

impl<T> Decoder for MessageCodec<T>
where
    T: RsaCryptoFetcher,
{
    type Item = Message;
    type Error = PpaassError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let (compressed, body_length) = match self.status {
            DecodeStatus::Head => {
                if src.len() < PPAASS_FLAG.len() + size_of::<u8>() + size_of::<u64>() {
                    return Ok(None);
                }
                let ppaass_flag = src.copy_to_bytes(PPAASS_FLAG.len());
                if !PPAASS_FLAG.eq(&ppaass_flag) {
                    error!( "Fail to decode input message because of it dose not begin with ppaass flag, hex data:\n\n{}\n\n", pretty_hex(src));
                    return Err(PpaassError::CodecError);
                }
                let compressed = src.get_u8() == 1;
                let body_length = src.get_u64();
                self.status = DecodeStatus::Data(compressed, body_length);
                (compressed, body_length)
            },
            DecodeStatus::Data(compressed, body_length) => (compressed, body_length),
        };
        if src.remaining() < body_length as usize {
            return Ok(None);
        }
        self.status = DecodeStatus::Data(compressed, body_length);
        let body_bytes = src.copy_to_bytes(body_length as usize);
        let mut message: Message = if compressed {
            let lz4_decompress_result = match decompress(body_bytes.chunk(), None) {
                Err(e) => {
                    error!(
                        "Fail to decompress message because of error: {:?}, hex data: \n{}\n",
                        e,
                        pretty_hex(src)
                    );
                    self.status = DecodeStatus::Head;
                    return Err(PpaassError::IoError { source: e });
                },
                Ok(r) => Bytes::from(r),
            };
            match lz4_decompress_result.try_into() {
                Err(e) => {
                    error!(
                        "Fail to parse message because of error: {:?}, hex data: \n{}\n",
                        e,
                        pretty_hex(src)
                    );
                    self.status = DecodeStatus::Head;
                    return Err(e);
                },
                Ok(r) => r,
            }
        } else {
            match body_bytes.try_into() {
                Err(e) => {
                    error!(
                        "Fail to parse message because of error: {:?}, hex data: \n{}\n",
                        e,
                        pretty_hex(src)
                    );
                    return Err(e);
                },
                Ok(r) => r,
            }
        };
        let rsa_crypto = match self.rsa_crypto_fetcher.fetch(message.user_token.as_str()) {
            Err(e) => {
                error!(
                    "Fail to decrypt message because of error when fetch rsa crypto: {:#?}",
                    e
                );
                self.status = DecodeStatus::Head;
                return Err(e);
            },
            Ok(None) => {
                error!(
                    "Fail to decrypt message because of rsa crypto not exist for user: {}",
                    message.user_token
                );
                self.status = DecodeStatus::Head;
                return Err(PpaassError::UnknownError);
            },
            Ok(Some(v)) => v,
        };
        debug!("Decode message from input(encrypted): {:?}", message);
        match message.payload_encryption_type {
            PayloadEncryptionType::Blowfish(ref encryption_token) => match message.payload {
                None => {
                    debug!("Nothing to decrypt for blowfish.")
                },
                Some(ref content) => {
                    let original_encryption_token = match rsa_crypto.decrypt(encryption_token) {
                        Err(e) => {
                            error!(
                                "Fail to decrypt message with blowfish because of error: {:#?}",
                                e
                            );
                            self.status = DecodeStatus::Head;
                            return Err(e);
                        },
                        Ok(r) => r,
                    };
                    let decrypt_payload =
                        decrypt_with_blowfish(&original_encryption_token, content);
                    message.payload = Some(decrypt_payload);
                },
            },
            PayloadEncryptionType::Aes(ref encryption_token) => match message.payload {
                None => {
                    debug!("Nothing to decrypt for aes.")
                },
                Some(ref content) => {
                    let original_encryption_token = match rsa_crypto.decrypt(encryption_token) {
                        Err(e) => {
                            error!(
                                "Fail to decrypt message with aes because of error: {:#?}",
                                e
                            );
                            self.status = DecodeStatus::Head;
                            return Err(e);
                        },
                        Ok(r) => r,
                    };
                    let decrypt_payload = decrypt_with_aes(&original_encryption_token, content);
                    message.payload = Some(decrypt_payload);
                },
            },
            PayloadEncryptionType::Plain => {},
        };
        debug!("Decode message from input(decrypted): {:?}", message);
        self.status = DecodeStatus::Head;
        Ok(Some(message))
    }
}

impl<T> Encoder<Message> for MessageCodec<T>
where
    T: RsaCryptoFetcher,
{
    type Error = PpaassError;

    fn encode(&mut self, original_message: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        debug!(
            "Encode message to output(decrypted): {:?}",
            original_message
        );
        dst.put(PPAASS_FLAG);
        if self.compress {
            dst.put_u8(1);
        } else {
            dst.put_u8(0);
        }
        if original_message.payload.is_none() {
            let result_bytes: Bytes = original_message.into();
            let result_bytes = if self.compress {
                Bytes::from(compress(result_bytes.chunk(), None, true)?)
            } else {
                result_bytes
            };
            let result_bytes_length = result_bytes.len();
            dst.put_u64(result_bytes_length as u64);
            dst.put(result_bytes);
            return Ok(());
        }
        let Message {
            id,
            ref_id,
            user_token,
            payload_encryption_type,
            payload,
        } = original_message;
        let rsa_crypto = match self.rsa_crypto_fetcher.fetch(user_token.as_str()) {
            Err(e) => {
                error!(
                    "Fail to encrypt message because of error when fetch rsa crypto: {:#?}",
                    e
                );
                return Err(e);
            },
            Ok(None) => {
                error!(
                    "Fail to encrypt message because of rsa crypto not exist for user: {}",
                    user_token
                );
                return Err(PpaassError::CodecError);
            },
            Ok(Some(v)) => v,
        };
        let (encrypted_payload, encrypted_payload_encryption_type) = match payload_encryption_type {
            PayloadEncryptionType::Plain => (payload, PayloadEncryptionType::Plain),
            PayloadEncryptionType::Blowfish(ref original_token) => {
                let encrypted_payload_encryption_token = match rsa_crypto.encrypt(original_token) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Fail the encrypt original message encryption token with rsa crypto for blowfish");
                        return Err(e);
                    },
                };
                (
                    Some(encrypt_with_blowfish(original_token, &payload.unwrap())),
                    PayloadEncryptionType::Blowfish(encrypted_payload_encryption_token),
                )
            },
            PayloadEncryptionType::Aes(ref original_token) => {
                let encrypted_payload_encryption_token = match rsa_crypto.encrypt(original_token) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Fail the encrypt original message encryption token with rsa crypto for aes");
                        return Err(e);
                    },
                };
                (
                    Some(encrypt_with_aes(original_token, &payload.unwrap())),
                    PayloadEncryptionType::Aes(encrypted_payload_encryption_token),
                )
            },
        };
        let message_to_encode = Message::new(
            id,
            ref_id,
            user_token,
            encrypted_payload_encryption_type,
            encrypted_payload,
        );
        let result_bytes: Bytes = message_to_encode.into();
        let result_bytes = if self.compress {
            Bytes::from(compress(result_bytes.chunk(), None, true)?)
        } else {
            result_bytes
        };
        let result_bytes_length = result_bytes.len();
        dst.put_u64(result_bytes_length as u64);
        dst.put(result_bytes);
        Ok(())
    }
}
