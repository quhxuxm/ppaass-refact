use std::fs;
use std::path::Path;

use bytes::{BufMut, Bytes, BytesMut};
use crypto::symmetriccipher::{BlockDecryptor, BlockEncryptor};
use crypto::{aessafe, blowfish};
use rand::rngs::OsRng;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{PaddingScheme, PublicKey, RsaPrivateKey, RsaPublicKey};
use tracing::error;

use crate::PpaassError;

const BLOWFISH_CHUNK_LENGTH: usize = 8;
const AES_CHUNK_LENGTH: usize = 16;
const AGENT_PRIVATE_KEY_PATH: &str = "AgentPrivateKey.pem";
const AGENT_PUBLIC_KEY_PATH: &str = "AgentPublicKey.pem";
const PROXY_PRIVATE_KEY_PATH: &str = "ProxyPrivateKey.pem";
const PROXY_PUBLIC_KEY_PATH: &str = "ProxyPublicKey.pem";

/// The rsa crypto fetcher,
/// each player have a rsa crypto
/// which can be fund from the storage
/// with user token
pub trait RsaCryptoFetcher {
    /// Fetch the rsa crypto by user token
    fn fetch<Q>(&self, user_token: Q) -> Result<Option<&RsaCrypto>, PpaassError>
    where
        Q: AsRef<str>;
}

/// The util to do RSA encryption and decryption.
#[derive(Debug)]
pub struct RsaCrypto {
    /// The private used to do decryption
    private_key: RsaPrivateKey,
    /// The public used to do encryption
    public_key: RsaPublicKey,
}

impl RsaCrypto {
    pub fn new<PU, PR>(public_key: PU, private_key: PR) -> Result<Self, PpaassError>
    where
        PU: AsRef<str>,
        PR: AsRef<str>,
    {
        let public_key = match RsaPublicKey::from_public_key_pem(public_key.as_ref()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse rsa key because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        let private_key = match RsaPrivateKey::from_pkcs8_pem(private_key.as_ref()) {
            Ok(v) => v,
            Err(e) => {
                error!("Fail to parse rsa key because of error: {:#?}", e);
                return Err(PpaassError::CodecError);
            },
        };
        Ok(Self { public_key, private_key })
    }

    pub(crate) fn encrypt(&self, target: &Bytes) -> Result<Bytes, PpaassError> {
        self.public_key
            .encrypt(&mut OsRng, PaddingScheme::PKCS1v15Encrypt, target)
            .map_err(|e| {
                error!("Fail to encrypt data with rsa because of error: {:#?}", e);
                PpaassError::CodecError
            })
            .map(|v| v.into())
    }

    pub(crate) fn decrypt(&self, target: &Bytes) -> Result<Bytes, PpaassError> {
        self.private_key
            .decrypt(PaddingScheme::PKCS1v15Encrypt, target)
            .map_err(|e| {
                error!("Fail to decrypt data with rsa because of error: {:#?}", e);
                PpaassError::CodecError
            })
            .map(|v| v.into())
    }
}

pub(crate) fn encrypt_with_aes(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let aes_encryptor = aessafe::AesSafe256Encryptor::new(encryption_token);
    let target_chunks = target.chunks(AES_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let mut chunk_to_encrypt = [0u8; AES_CHUNK_LENGTH];
        let mut chunk_encrypted = [0u8; AES_CHUNK_LENGTH];
        if chunk.len() < AES_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_encrypt[i] = *v;
            }
        } else {
            chunk_encrypted.copy_from_slice(chunk);
        }
        aes_encryptor.encrypt_block(&chunk_to_encrypt, &mut chunk_encrypted);
        result.put_slice(&chunk_encrypted);
    });
    result.into()
}

pub(crate) fn decrypt_with_aes(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let aes_decryptor = aessafe::AesSafe256Decryptor::new(encryption_token);
    let target_chunks = target.chunks(AES_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let mut chunk_to_decrypt = [0u8; AES_CHUNK_LENGTH];
        if chunk.len() < AES_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_decrypt[i] = *v;
            }
        } else {
            chunk_to_decrypt.copy_from_slice(chunk);
        }
        let mut chunk_decrypted = [0u8; AES_CHUNK_LENGTH];
        aes_decryptor.decrypt_block(&chunk_to_decrypt, &mut chunk_decrypted);
        result.put_slice(&chunk_decrypted);
    });
    result.into()
}

pub(crate) fn encrypt_with_blowfish(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let blowfish_encryption = blowfish::Blowfish::new(encryption_token);
    let target_chunks = target.chunks(BLOWFISH_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let mut chunk_to_encrypt = [0u8; BLOWFISH_CHUNK_LENGTH];
        if chunk.len() < BLOWFISH_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_encrypt[i] = *v;
            }
        } else {
            chunk_to_encrypt.copy_from_slice(chunk);
        }
        let mut chunk_encrypted = [0u8; BLOWFISH_CHUNK_LENGTH];
        blowfish_encryption.encrypt_block(&chunk_to_encrypt, &mut chunk_encrypted);
        result.put_slice(&chunk_encrypted);
    });
    result.into()
}

pub(crate) fn decrypt_with_blowfish(encryption_token: &Bytes, target: &Bytes) -> Bytes {
    let mut result = BytesMut::new();
    let blowfish_encryption = blowfish::Blowfish::new(encryption_token);
    let target_chunks = target.chunks(BLOWFISH_CHUNK_LENGTH);
    target_chunks.for_each(|chunk| {
        let mut chunk_to_decrypt = [0u8; BLOWFISH_CHUNK_LENGTH];
        if chunk.len() < BLOWFISH_CHUNK_LENGTH {
            for (i, v) in chunk.iter().enumerate() {
                chunk_to_decrypt[i] = *v;
            }
        } else {
            chunk_to_decrypt.copy_from_slice(chunk);
        }
        let mut chunk_decrypted = [0u8; BLOWFISH_CHUNK_LENGTH];
        blowfish_encryption.decrypt_block(&chunk_to_decrypt, &mut chunk_decrypted);
        result.put_slice(&chunk_decrypted);
    });
    result.into()
}

pub fn generate_agent_key_pairs(base_dir: &str, user_token: &str) -> Result<(), PpaassError> {
    let private_key_path = format!("{}/{}/{}", base_dir, user_token, AGENT_PRIVATE_KEY_PATH);
    let private_key_path = Path::new(private_key_path.as_str());
    let public_key_path = format!("{}/{}/{}", base_dir, user_token, AGENT_PUBLIC_KEY_PATH);
    let public_key_path = Path::new(public_key_path.as_str());
    generate_rsa_key_pairs(private_key_path, public_key_path)
}

pub fn generate_proxy_key_pairs(base_dir: &str, user_token: &str) -> Result<(), PpaassError> {
    let private_key_path = format!("{}/{}/{}", base_dir, user_token, PROXY_PRIVATE_KEY_PATH);
    let private_key_path = Path::new(private_key_path.as_str());
    let public_key_path = format!("{}/{}/{}", base_dir, user_token, PROXY_PUBLIC_KEY_PATH);
    let public_key_path = Path::new(public_key_path.as_str());
    generate_rsa_key_pairs(private_key_path, public_key_path)
}

fn generate_rsa_key_pairs(private_key_path: &Path, public_key_path: &Path) -> Result<(), PpaassError> {
    let private_key = RsaPrivateKey::new(&mut OsRng, 2048).expect("Fail to generate private key");
    let public_key = RsaPublicKey::from(&private_key);
    let private_key_pem = private_key.to_pkcs8_pem(LineEnding::CRLF).expect("Fail to generate pem for private key.");
    let public_key_pem = public_key.to_public_key_pem(LineEnding::CRLF).expect("Fail to generate pem for public key.");
    match private_key_path.parent() {
        None => {
            println!("Write private key: {:?}", private_key_path.to_str());
            fs::write(private_key_path, private_key_pem.as_bytes())?;
        },
        Some(parent) => {
            if !parent.exists() {
                println!("Create parent directory :{:?}", parent.to_str());
                fs::create_dir_all(parent)?;
            }
            println!("Write private key: {:?}", private_key_path.to_str());
            fs::write(private_key_path, private_key_pem.as_bytes())?;
        },
    };
    match public_key_path.parent() {
        None => {
            println!("Write public key: {:?}", public_key_path.to_str());
            fs::write(public_key_path, public_key_pem.as_bytes())?;
        },
        Some(parent) => {
            if !parent.exists() {
                println!("Create parent directory :{:?}", parent.to_str());
                fs::create_dir_all(parent)?;
            }
            println!("Write public key: {:?}", public_key_path.to_str());
            fs::write(public_key_path, public_key_pem.as_bytes())?;
        },
    };
    Ok(())
}
