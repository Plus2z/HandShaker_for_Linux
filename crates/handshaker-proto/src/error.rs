use thiserror::Error;

pub type Result<T> = std::result::Result<T, ProtoError>;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protobuf decode error: {0}")]
    ProtoDecode(#[from] prost::DecodeError),

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("Wire framing error: {0}")]
    Framing(String),

    #[error("Discovery error: {0}")]
    Discovery(String),

    #[error("USB error: {0}")]
    Usb(#[from] rusb::Error),

    #[error("Handshake rejected (trust_type={trust_type}): {reason}")]
    HandshakeRejected { trust_type: i32, reason: String },

    #[error("Phone screen is locked; unlock phone and retry")]
    PhoneLocked,

    #[error("Remote file I/O error (code={code:?}): {message}")]
    RemoteIo { code: Option<i32>, message: String },

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Timed out waiting for {0}")]
    Timeout(&'static str),

    #[error("Session disconnected")]
    Disconnected,

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}
