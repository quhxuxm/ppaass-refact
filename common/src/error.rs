#[derive(thiserror::Error, Debug)]
pub enum CommonError {
    #[error("Unknown payload type: {0}")]
    UnknownPayloadType(u8),
    #[error("Fail to parse NetAddress.")]
    FailToParseNetAddress,
    #[error("Fail to parse PayloadEncryptionType.")]
    FailToParsePayloadEncryptionType,
}
