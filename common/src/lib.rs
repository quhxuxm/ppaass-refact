mod domain {
    mod payload {
        mod agent;
        mod proxy;
    }
    mod address;
    mod message;

    /// Public use by outside
    pub use message::Message;
    pub use message::MessageBuilder;
    pub use message::PayloadEncryptionType;
}

mod error;
mod util;

/// Public use by outside
pub use domain::Message;
pub use domain::MessageBuilder;
pub use domain::PayloadEncryptionType;
pub use error::CommonError;
