#[derive(thiserror::Error, Debug)]
pub enum CommonError {
    #[error("Unknown payload type: {0}")]
    UnknownPayloadType(u8),
}
