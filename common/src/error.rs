#[derive(thiserror::Error, Debug)]
pub enum CommonError {
    #[error("Codec error happen.")]
    CodecError,
    #[error("I/O error happen.")]
    IoError {
        #[from]
        source: std::io::Error,
    },
    #[error("Unkown error happen.")]
    UnknownError,
}
