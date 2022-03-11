use bytes::{Buf, Bytes, BytesMut};
use lz4::block::{compress, decompress};
use rand::Rng;
use rand::rngs::ThreadRng;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use tracing::{debug, error};

use crate::{CommonError, Message, PayloadEncryptionType};
use crate::crypto::{
    decrypt_with_aes, decrypt_with_blowfish, encrypt_with_aes, encrypt_with_blowfish, RsaCrypto,
};

pub struct MessageCodec<T: Rng> {
    rsa_crypto: RsaCrypto<T>,
    length_delimited_codec: LengthDelimitedCodec,
    compress: bool,
}

impl MessageCodec<ThreadRng> {
    pub fn new(
        public_key: &'static str,
        private_key: &'static str,
        max_frame_size: usize,
        compress: bool,
    ) -> Self {
        let mut length_delimited_codec_builder = LengthDelimitedCodec::builder();
        length_delimited_codec_builder.max_frame_length(max_frame_size);
        length_delimited_codec_builder.length_field_length(8);
        let length_delimited_codec = length_delimited_codec_builder.new_codec();
        let rng = rand::thread_rng();
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

impl Decoder for MessageCodec<ThreadRng> {
    type Item = Message;
    type Error = CommonError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let length_delimited_decode_result = self.length_delimited_codec.decode(src)?;
        let length_delimited_decode_result = match length_delimited_decode_result {
            None => return Ok(None),
            Some(r) => r,
        };
        let mut message: Message = if self.compress {
            let lz4_decompress_result = match decompress(length_delimited_decode_result.chunk(), None) {
                Err(e) => {
                    error!("Fail to decompress message because of error: {:#?}", e);
                    return Err(CommonError::IoError {
                        source: e
                    });
                }
                Ok(r) => Bytes::from(r)
            };
            match lz4_decompress_result.try_into() {
                Err(e) => {
                    error!("Fail to parse message because of error: {:#?}", e);
                    return Err(e);
                }
                Ok(r) => r
            }
        } else {
            match length_delimited_decode_result.freeze().try_into() {
                Err(e) => {
                    error!("Fail to parse message because of error: {:#?}", e);
                    return Err(e);
                }
                Ok(r) => r
            }
        };
        debug!(
            "Decode message from input(encrypted): {:?}",
            message
        );
        match message.payload_encryption_type() {
            PayloadEncryptionType::Blowfish(encryption_token) => {
                match message.payload() {
                    None => {
                        debug!("Nothing to decrypt for blowfish.")
                    }
                    Some(content) => {
                        let original_encryption_token = match self
                            .rsa_crypto
                            .decrypt(encryption_token) {
                            Err(e) => {
                                error!("Fail to decrypt message with blowfish because of error: {:#?}", e);
                                return Err(e);
                            }
                            Ok(r) => r
                        };
                        let decrypt_payload = decrypt_with_blowfish(&original_encryption_token, content);
                        message.set_payload(Some(decrypt_payload));
                    }
                }
            }
            PayloadEncryptionType::Aes(encryption_token) => {
                match message.payload() {
                    None => {
                        debug!("Nothing to decrypt for aes.")
                    }
                    Some(content) => {
                        let original_encryption_token = match self
                            .rsa_crypto
                            .decrypt(encryption_token) {
                            Err(e) => {
                                error!("Fail to decrypt message with aes because of error: {:#?}", e);
                                return Err(e);
                            }
                            Ok(r) => r
                        };
                        let decrypt_payload = decrypt_with_aes(&original_encryption_token, content);
                        message.set_payload(Some(decrypt_payload));
                    }
                }
            }
            PayloadEncryptionType::Plain => {}
        };
        debug!("Decode message from input(decrypted): {:?}", message);
        Ok(Some(message))
    }
}

impl Encoder<Message> for MessageCodec<ThreadRng> {
    type Error = CommonError;

    fn encode(
        &mut self,
        original_message: Message,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        debug!(
            "Encode message to output(decrypted): {:?}",
            original_message
        );
        let rsa_encrypted_payload_encryption_token = match original_message.payload_encryption_type() {
            PayloadEncryptionType::Plain => {
                None
            }
            PayloadEncryptionType::Blowfish(original_token) => {
                match self
                    .rsa_crypto
                    .encrypt(original_token) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        return Err(CommonError::);
                    }
                }
            }
            PayloadEncryptionType::Aes(original_token) => {}
        };
        let encrypted_payload = match payload_encryption_type {
            PpaassMessagePayloadEncryptionType::Plain => payload,
            PpaassMessagePayloadEncryptionType::Blowfish => {
                encrypt_with_blowfish(payload_encryption_token.chunk(), payload.chunk())
            }
            PpaassMessagePayloadEncryptionType::Aes => {
                encrypt_with_aes(payload_encryption_token.chunk(), payload.chunk())
            }
        };
        let encrypted_message = PpaassMessage::new_with_random_encryption_type(
            ref_id,
            user_token,
            rsa_encrypted_payload_encryption_token,
            encrypted_payload,
        );
        debug!(
            "Encode ppaass message to output(encrypted): {:?}",
            encrypted_message
        );
        let result_bytes: Bytes = encrypted_message.into();
        self.length_delimited_codec.encode(
            if self.compress {
                Bytes::from(compress(result_bytes.chunk(), None, true)?)
            } else {
                result_bytes
            },
            dst,
        )?;
        Ok(())
    }
}
