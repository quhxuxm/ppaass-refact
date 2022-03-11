#[derive(thiserror::Error, Debug)]
pub enum CommonError {
    #[error("Fail to parse payload type")]
    FailToParsePayloadType,
    #[error("Fail to parse payload")]
    FailToParsePayload,
    #[error("Fail to parse net address.")]
    FailToParseNetAddress,
    #[error("Fail to parse encryption type.")]
    FailToParsePayloadEncryptionType,

    #[error("Fail to parse rsa key.")]
    FailToParseRsaKey,
    #[error("Fail to encrypt data with rsa.")]
    FailToEncryptDataWithRsa,

    #[error("I/O error happen.")]
    IoError {
        #[from] source: std::io::Error
    },
}
