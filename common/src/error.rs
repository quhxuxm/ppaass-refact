/// The general error happen in ppaass project.
#[derive(thiserror::Error, Debug)]
pub enum PpaassError {
    #[error("Codec error happen.")]
    CodecError,
    #[error("I/O error happen.")]
    IoError {
        #[from]
        source: std::io::Error,
    },
    #[error("Unkown error happen.")]
    UnknownError,
    #[error("Timeout error happen.")]
    TimeoutError,
}
