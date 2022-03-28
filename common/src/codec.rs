use bytes::{Buf, Bytes, BytesMut};
use lz4::block::{compress, decompress};
use rand::rngs::OsRng;
use rand::Rng;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use tracing::{debug, error};

use crate::crypto::{
    decrypt_with_aes, decrypt_with_blowfish, encrypt_with_aes, encrypt_with_blowfish, RsaCrypto,
};
use crate::{CommonError, Message, PayloadEncryptionType};

const LENGTH_DELIMITED_CODEC_LENGTH_FIELD_LENGTH: usize = 8;

pub struct MessageCodec{
    rsa_crypto: RsaCrypto<OsRng>,
    length_delimited_codec: LengthDelimitedCodec,
    compress: bool,
}

impl MessageCodec {
    pub fn new(
        public_key: &'static str,
        private_key: &'static str,
        max_frame_size: usize,
        compress: bool,
    ) -> Self {
        let mut length_delimited_codec_builder = LengthDelimitedCodec::builder();
        length_delimited_codec_builder.max_frame_length(max_frame_size);
        length_delimited_codec_builder
            .length_field_length(LENGTH_DELIMITED_CODEC_LENGTH_FIELD_LENGTH);
        let length_delimited_codec = length_delimited_codec_builder.new_codec();
        let rng = OsRng;
        let rsa_crypto = RsaCrypto::new(public_key, private_key, rng);
        let rsa_crypto = match rsa_crypto {
            Ok(r) => r,
            Err(e) => {
                error!("Fail to create RSA Crypto because of error: {:#?}", e);
                panic!("Fail to create RSA Crypto.")
            }
        };
        Self {
            rsa_crypto,
            length_delimited_codec,
            compress,
        }
    }
}

impl Decoder for MessageCodec {
    type Item = Message;
    type Error = CommonError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let length_delimited_decode_result = self.length_delimited_codec.decode(src)?;
        let length_delimited_decode_result = match length_delimited_decode_result {
            None => return Ok(None),
            Some(r) => r,
        };
        let mut message: Message = if self.compress {
            let lz4_decompress_result =
                match decompress(length_delimited_decode_result.chunk(), None) {
                    Err(e) => {
                        error!("Fail to decompress message because of error: {:#?}", e);
                        return Err(CommonError::IoError { source: e });
                    }
                    Ok(r) => Bytes::from(r),
                };
            match lz4_decompress_result.try_into() {
                Err(e) => {
                    error!("Fail to parse message because of error: {:#?}", e);
                    return Err(e);
                }
                Ok(r) => r,
            }
        } else {
            match length_delimited_decode_result.freeze().try_into() {
                Err(e) => {
                    error!("Fail to parse message because of error: {:#?}", e);
                    return Err(e);
                }
                Ok(r) => r,
            }
        };
        debug!("Decode message from input(encrypted): {:?}", message);
        match message.payload_encryption_type {
            PayloadEncryptionType::Blowfish(ref encryption_token) => match message.payload {
                None => {
                    debug!("Nothing to decrypt for blowfish.")
                }
                Some(ref content) => {
                    let original_encryption_token = match self.rsa_crypto.decrypt(encryption_token)
                    {
                        Err(e) => {
                            error!(
                                "Fail to decrypt message with blowfish because of error: {:#?}",
                                e
                            );
                            return Err(e);
                        }
                        Ok(r) => r,
                    };
                    let decrypt_payload =
                        decrypt_with_blowfish(&original_encryption_token, content);
                    message.payload = Some(decrypt_payload);
                }
            },
            PayloadEncryptionType::Aes(ref encryption_token) => match message.payload {
                None => {
                    debug!("Nothing to decrypt for aes.")
                }
                Some(ref content) => {
                    let original_encryption_token = match self.rsa_crypto.decrypt(encryption_token)
                    {
                        Err(e) => {
                            error!(
                                "Fail to decrypt message with aes because of error: {:#?}",
                                e
                            );
                            return Err(e);
                        }
                        Ok(r) => r,
                    };
                    let decrypt_payload = decrypt_with_aes(&original_encryption_token, content);
                    message.payload = Some(decrypt_payload);
                }
            },
            PayloadEncryptionType::Plain => {}
        };
        debug!("Decode message from input(decrypted): {:?}", message);
        Ok(Some(message))
    }
}

impl Encoder<Message> for MessageCodec {
    type Error = CommonError;

    fn encode(&mut self, original_message: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        debug!(
            "Encode message to output(decrypted): {:?}",
            original_message
        );
        if original_message.payload.is_none() {
            let result_bytes: Bytes = original_message.into();
            if let Err(e) = self.length_delimited_codec.encode(
                if self.compress {
                    Bytes::from(compress(result_bytes.as_ref(), None, true)?)
                } else {
                    result_bytes
                },
                dst,
            ) {
                error!("Fail to encode original message because of error: {:#?}", e);
                return Err(CommonError::IoError { source: e });
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
        let (encrypted_payload, encrypted_payload_encryption_type) = match payload_encryption_type {
            PayloadEncryptionType::Plain => (payload, PayloadEncryptionType::Plain),
            PayloadEncryptionType::Blowfish(ref original_token) => {
                let encrypted_payload_encryption_token = match self
                    .rsa_crypto
                    .encrypt(original_token)
                {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Fail the encrypt original message encryption token with rsa crypto for blowfish");
                        return Err(e);
                    }
                };
                (
                    Some(encrypt_with_blowfish(original_token, &payload.unwrap())),
                    PayloadEncryptionType::Blowfish(encrypted_payload_encryption_token),
                )
            }
            PayloadEncryptionType::Aes(ref original_token) => {
                let encrypted_payload_encryption_token = match self
                    .rsa_crypto
                    .encrypt(original_token)
                {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Fail the encrypt original message encryption token with rsa crypto for aes");
                        return Err(e);
                    }
                };
                (
                    Some(encrypt_with_aes(original_token, &payload.unwrap())),
                    PayloadEncryptionType::Aes(encrypted_payload_encryption_token),
                )
            }
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
                Bytes::from(compress(result_bytes.chunk(), None, true)?)
            } else {
                result_bytes
            },
            dst,
        ) {
            error!("Fail to encode original message because of error: {:#?}", e);
            return Err(CommonError::IoError { source: e });
        }
        Ok(())
    }
}
