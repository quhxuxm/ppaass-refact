use bytes::{BufMut, Bytes, BytesMut};
use crypto::symmetriccipher::{BlockDecryptor, BlockEncryptor};
use crypto::{aessafe, blowfish};
use rand::Rng;
use rsa::pkcs8::{FromPrivateKey, FromPublicKey};
use rsa::{PaddingScheme, PublicKey, RsaPrivateKey, RsaPublicKey};
use tracing::error;

use crate::CommonError;

const BLOWFISH_CHUNK_LENGTH: usize = 8;
const AES_CHUNK_LENGTH: usize = 16;

/// The util to do RSA encryption and decryption.
#[derive(Debug)]
pub(crate) struct RsaCrypto<T> {
    /// The private used to do decryption
    private_key: RsaPrivateKey,
    /// The public used to do encryption
    public_key: RsaPublicKey,
    rng: T,
}

impl<T> RsaCrypto<T>
where
    T: Rng + Send + Sync,
{
    pub fn new(
        public_key: &'static str,
        private_key: &'static str,
        rng: T,
    ) -> Result<Self, CommonError> {
        let public_key = match RsaPublicKey::from_public_key_pem(public_key) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse rsa key because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            },
        };
        let private_key = match RsaPrivateKey::from_pkcs8_pem(private_key) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse rsa key because of error: {:#?}", e);
                return Err(CommonError::CodecError);
            },
        };
        Ok(Self {
            public_key,
            private_key,
            rng,
        })
    }

    pub(crate) fn encrypt(&mut self, target: &Bytes) -> Result<Bytes, CommonError> {
        self.public_key
            .encrypt(&mut self.rng, PaddingScheme::PKCS1v15Encrypt, target)
            .map_err(|e| {
                error!("Fail to encrypt data with rsa because of error: {:#?}", e);
                CommonError::CodecError
            })
            .map(|v| v.into())
    }

    pub(crate) fn decrypt(&self, target: &Bytes) -> Result<Bytes, CommonError> {
        self.private_key
            .decrypt(PaddingScheme::PKCS1v15Encrypt, target)
            .map_err(|e| {
                error!("Fail to decrypt data with rsa because of error: {:#?}", e);
                CommonError::CodecError
            })
            .map(|v| v.into())
    }
}

pub(crate) fn encrypt_with_aes(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let aes_encryptor = aessafe::AesSafe256Encryptor::new(encryption_token);
    let target_chunks = target.chunks(AES_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let chunk_to_encrypt = &mut [0u8; AES_CHUNK_LENGTH];
        let chunk_encrypted = &mut [0u8; AES_CHUNK_LENGTH];
        if chunk.len() < AES_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_encrypt[i] = *v;
            }
        } else {
            chunk_encrypted.copy_from_slice(chunk);
        }
        aes_encryptor.encrypt_block(chunk_to_encrypt, chunk_encrypted);
        result.put_slice(chunk_encrypted);
    });
    result.into()
}

pub(crate) fn decrypt_with_aes(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let aes_decryptor = aessafe::AesSafe256Decryptor::new(encryption_token);
    let target_chunks = target.chunks(AES_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let chunk_to_decrypt = &mut [0u8; AES_CHUNK_LENGTH];
        if chunk.len() < AES_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_decrypt[i] = *v;
            }
        } else {
            chunk_to_decrypt.copy_from_slice(chunk);
        }
        let chunk_decrypted = &mut [0u8; AES_CHUNK_LENGTH];
        aes_decryptor.decrypt_block(chunk_to_decrypt, chunk_decrypted);
        result.put_slice(chunk_decrypted);
    });
    result.into()
}

pub(crate) fn encrypt_with_blowfish(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let blowfish_encryption = blowfish::Blowfish::new(encryption_token);
    let target_chunks = target.chunks(BLOWFISH_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let chunk_to_encrypt = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        if chunk.len() < BLOWFISH_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_encrypt[i] = *v;
            }
        } else {
            chunk_to_encrypt.copy_from_slice(chunk);
        }
        let chunk_encrypted = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        blowfish_encryption.encrypt_block(chunk_to_encrypt, chunk_encrypted);
        result.put_slice(chunk_encrypted);
    });
    result.into()
}

pub(crate) fn decrypt_with_blowfish(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let blowfish_encryption = blowfish::Blowfish::new(encryption_token);
    let target_chunks = target.chunks(BLOWFISH_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let chunk_to_decrypt = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        if chunk.len() < BLOWFISH_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_decrypt[i] = *v;
            }
        } else {
            chunk_to_decrypt.copy_from_slice(chunk);
        }
        let chunk_decrypted = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        blowfish_encryption.decrypt_block(chunk_to_decrypt, chunk_decrypted);
        result.put_slice(chunk_decrypted);
    });
    result.into()
}
