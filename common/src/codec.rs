use std::{
    fmt::{Debug, Formatter},
    mem::size_of,
    sync::Arc,
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use lz4::block::{compress, decompress};
use pretty_hex::*;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use tracing::{debug, error};

use crate::{Message, PayloadEncryptionType, PpaassError};
use crate::crypto::{
    decrypt_with_aes, decrypt_with_blowfish, encrypt_with_aes, encrypt_with_blowfish,
    RsaCryptoFetcher,
};

const LENGTH_DELIMITED_CODEC_LENGTH_FIELD_LENGTH: usize = 8;
const PPAASS_FLAG: &[u8] = "__PPAASS__".as_bytes();

enum DecodeStatus {
    Head,
    Data(bool),
}
pub struct MessageCodec<T: RsaCryptoFetcher> {
    rsa_crypto_fetcher: Arc<T>,
    length_delimited_codec: LengthDelimitedCodec,
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
    pub fn new(max_frame_size: usize, compress: bool, rsa_crypto_fetcher: Arc<T>) -> Self {
        let mut length_delimited_codec_builder = LengthDelimitedCodec::builder();
        length_delimited_codec_builder.max_frame_length(max_frame_size);
        length_delimited_codec_builder
            .length_field_length(LENGTH_DELIMITED_CODEC_LENGTH_FIELD_LENGTH);
        let length_delimited_codec = length_delimited_codec_builder.new_codec();
        Self {
            rsa_crypto_fetcher,
            length_delimited_codec,
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
        let compressed = match self.status {
            DecodeStatus::Head => {
                if src.len() < PPAASS_FLAG.len() + size_of::<u8>() {
                    return Ok(None);
                }
                let ppaass_flag = src.copy_to_bytes(PPAASS_FLAG.len());
                if !PPAASS_FLAG.eq(&ppaass_flag) {
                    error!(
                    "Fail to decode input message because of it dose not begin with ppaass flag, hex data:\n{}\n", pretty_hex(src));
                    return Err(PpaassError::CodecError);
                }
                let compressed = src.get_u8() == 1;
                self.status = DecodeStatus::Data(compressed);
                compressed
            },
            DecodeStatus::Data(compressed) => compressed,
        };
        let length_delimited_decode_result = self.length_delimited_codec.decode(src);
        let length_delimited_decode_result = match length_delimited_decode_result {
            Err(e) => {
                error!(
                            "Fail to decode input message because of length delimited error: {:?}, hex data:\n{}\n",
                            e, pretty_hex(src));
                self.status = DecodeStatus::Head;
                return Err(PpaassError::CodecError);
            },
            Ok(None) => {
                self.status = DecodeStatus::Data(compressed);
                return Ok(None);
            },
            Ok(Some(r)) => r,
        };
        let mut message: Message = if compressed {
            let lz4_decompress_result =
                match decompress(length_delimited_decode_result.chunk(), None) {
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
            match length_delimited_decode_result.freeze().try_into() {
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
        if original_message.payload.is_none() {
            let result_bytes: Bytes = original_message.into();
            if let Err(e) = self.length_delimited_codec.encode(
                if self.compress {
                    dst.put_u8(1);
                    Bytes::from(compress(result_bytes.chunk(), None, true)?)
                } else {
                    dst.put_u8(0);
                    result_bytes
                },
                dst,
            ) {
                error!("Fail to encode original message because of error: {:#?}", e);
                return Err(PpaassError::IoError { source: e });
            }
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
                return Err(PpaassError::UnknownError);
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
        if let Err(e) = self.length_delimited_codec.encode(
            if self.compress {
                dst.put_u8(1);
                Bytes::from(compress(result_bytes.chunk(), None, true)?)
            } else {
                dst.put_u8(0);
                result_bytes
            },
            dst,
        ) {
            error!("Fail to encode original message because of error: {:#?}", e);
            return Err(PpaassError::IoError { source: e });
        }
        Ok(())
    }
}
