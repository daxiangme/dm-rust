//! 协议层错误类型。
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("truncated message: need {need} bytes at offset {at}, have {have}")]
    Truncated { at: usize, need: usize, have: usize },
    #[error("message checksum mismatch")]
    BadChecksum,
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("compression error: {0}")]
    Compress(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("message too long")]
    TooLong,
    #[error("value conversion error: {0}")]
    Convert(String),
}

pub type Result<T> = std::result::Result<T, ProtoError>;
