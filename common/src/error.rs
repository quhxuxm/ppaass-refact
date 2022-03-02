#[derive(thiserror::Error, Debug)]
pub enum CommonError {
    #[error("Fail to parse payload type")]
    FailToParsePayloadType,
    #[error("Fail to parse payload")]
    FailToParsePayload,
    #[error("Fail to parse NetAddress.")]
    FailToParseNetAddress,
    #[error("Fail to parse PayloadEncryptionType.")]
    FailToParsePayloadEncryptionType,
    #[error("Fail to parse FailToParseMessage.")]
    FailToParseMessage,
}
