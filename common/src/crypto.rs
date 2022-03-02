use std::{borrow::BorrowMut, array};

use bytes::{BufMut, Bytes, BytesMut, Buf};
use crypto::symmetriccipher::{BlockDecryptor, BlockEncryptor};
use crypto::{aessafe, blowfish};
use rand::{rngs::OsRng, Rng};
use rsa::pkcs8::{FromPrivateKey, FromPublicKey};
use rsa::{PaddingScheme, PublicKey, RsaPrivateKey, RsaPublicKey};

use crate::{error, CommonError};
use tracing::error;

const BLOWFISH_CHUNK_LENGTH: usize = 8;
const AES_CHUNK_LENGTH: usize = 16;
const RSA_BIT_SIZE: usize = 2048;

/// The util to do RSA encryption and decryption.
#[derive(Debug)]
pub(crate) struct RsaCrypto {
    /// The private used to do decryption
    private_key: RsaPrivateKey,
    /// The public used to do encryption
    public_key: RsaPublicKey,
    rng: Box<OsRng>
}

impl RsaCrypto {
    pub fn new(public_key: &'static str, private_key: &'static str) -> Result<Self, CommonError> {
        let public_key = match RsaPublicKey::from_public_key_pem(public_key) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse rsa key because of error: {:#?}", e);
                return Err(CommonError::FailToParseRsaKey);
            }
        };
        let private_key = match RsaPrivateKey::from_pkcs8_pem(private_key) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse rsa key because of error: {:#?}", e);
                return Err(CommonError::FailToParseRsaKey);
            }
        };
        Ok(Self {
            public_key,
            private_key,
            rng: Box::new(OsRng)
        })
    }

    pub(crate) fn encrypt(&mut self, target: &Bytes) -> Result<Bytes, CommonError> {
        self.public_key
            .encrypt(self.rng.as_mut(), PaddingScheme::PKCS1v15Encrypt, target)
            .map_err(|e| {
                error!("Fail to encrypt data with rsa because of error: {:#?}", e);
                CommonError::FailToEncryptDataWithRsa
            })
            .map(|v| v.into())
    }

    pub(crate) fn decrypt(&self, target: &Bytes) -> Result<Bytes, CommonError> {
        self.private_key
            .decrypt(PaddingScheme::PKCS1v15Encrypt, target)
            .map_err(|e| {
                error!("Fail to encrypt data with rsa because of error: {:#?}", e);
                CommonError::FailToEncryptDataWithRsa
            })
            .map(|v| v.into())
    }
}

pub(crate) fn encrypt_with_aes(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let aes_encryptor = aessafe::AesSafe256Encryptor::new(encryption_token);
    let target_chunks = target.chunks(AES_CHUNK_LENGTH);
    target_chunks.for_each(|chunk|{
        let mut chunk_to_encrypt = &mut [0u8; AES_CHUNK_LENGTH];
        let chunk_encrypted = &mut [0u8; AES_CHUNK_LENGTH];
        unsafe{
            chunk_to_encrypt = std::mem::transmute(chunk);
        }
        aes_encryptor.encrypt_block(chunk_to_encrypt, chunk_encrypted);
        result.put_slice(chunk_encrypted);
    });
    result.into()
}

pub(crate) fn decrypt_with_aes(encryption_token: &[u8], target: &[u8]) -> Bytes {
    let mut result = BytesMut::new();
    let aes_decryptor = aessafe::AesSafe256Decryptor::new(encryption_token);
    let target_chunks = target.chunks(AES_CHUNK_LENGTH);
    for (_, current_chunk) in target_chunks.into_iter().enumerate() {
        let chunk_to_decrypt = &mut [0u8; AES_CHUNK_LENGTH];
        for (i, b) in current_chunk.iter().enumerate() {
            chunk_to_decrypt[i] = *b;
        }
        let chunk_decrypted = &mut [0u8; AES_CHUNK_LENGTH];
        aes_decryptor.decrypt_block(chunk_to_decrypt, chunk_decrypted);
        result.put_slice(chunk_decrypted);
    }
    result.into()
}

pub(crate) fn encrypt_with_blowfish(encryption_token: &[u8], target: &[u8]) -> Bytes {
    let mut result = BytesMut::new();
    let blowfish_encryption = blowfish::Blowfish::new(encryption_token);
    let target_chunks = target.chunks(BLOWFISH_CHUNK_LENGTH);
    for current_chunk in target_chunks {
        let chunk_to_encrypt = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        for (i, b) in current_chunk.iter().enumerate() {
            chunk_to_encrypt[i] = *b;
        }
        let chunk_encrypted = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        blowfish_encryption.encrypt_block(chunk_to_encrypt, chunk_encrypted);
        result.put_slice(chunk_encrypted);
    }
    result.into()
}

pub(crate) fn decrypt_with_blowfish(encryption_token: &[u8], target: &[u8]) -> Bytes {
    let mut result = BytesMut::new();
    let blowfish_encryption = blowfish::Blowfish::new(encryption_token);
    let target_chunks = target.chunks(BLOWFISH_CHUNK_LENGTH);
    for (_, current_chunk) in target_chunks.into_iter().enumerate() {
        let chunk_to_decrypt = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        for (i, b) in current_chunk.iter().enumerate() {
            chunk_to_decrypt[i] = *b;
        }
        let chunk_decrypted = &mut [0u8; BLOWFISH_CHUNK_LENGTH];
        blowfish_encryption.decrypt_block(chunk_to_decrypt, chunk_decrypted);
        result.put_slice(chunk_decrypted);
    }
    result.into()
}
