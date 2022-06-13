use std::io::Error as StdIoError;
/// The general error happen in ppaass project.
#[derive(thiserror::Error, Debug)]
pub enum PpaassError {
    #[error("Codec error happen.")]
    CodecError,
    #[error("Error happen, original io error: {:?}", source)]
    IoError {
        #[from]
        source: StdIoError,
    },
    #[error("Timeout error happen, elapsed: {elapsed}.")]
    TimeoutError { elapsed: u64 },
}
