use bytes::Bytes;

#[derive(Debug)]
pub enum AddressType {
    IpV4,
    IpV6,
    Domain,
}

#[derive(Debug)]
pub struct Address {
    host: Vec<u8>,
    port: u16,
    address_type: AddressType,
}

#[derive(Debug)]
pub enum MessagePayloadEncryptionType {
    Plain,
    Blowfish,
    Aes,
}

/// The message
#[derive(Debug)]
pub struct Message {
    /// The message id
    id: String,
    /// The message id that this message reference to
    ref_id: String,
    /// The user token
    user_token: String,
    /// The payload encryption token
    payload_encryption_token: Bytes,
    /// The payload encryption type
    payload_encryption_type: MessagePayloadEncryptionType,
    /// The payload
    payload: Bytes,
}