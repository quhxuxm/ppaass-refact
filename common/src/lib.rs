mod domain {
    mod address;
    mod message;

    /// Public use by outside
    pub use message::Message;
    pub use message::MessagePayload;
    pub use message::MessageBuilder;
    pub use message::PayloadEncryptionType;
}

mod error;
mod util;

/// Public use by outside
pub use domain::Message;
pub use domain::MessagePayload;
pub use domain::MessageBuilder;
pub use domain::PayloadEncryptionType;
pub use error::CommonError;
