use crate::proto::ProtoError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Proto(#[from] ProtoError),
    /// 服务端返回的错误（返回码 < 0）。
    #[error("DM error {code}: {message}")]
    Server { code: i32, message: String },
    #[error("batch execution failed: {0:?}")]
    Batch(Vec<BatchRowError>),
    #[error("connection is broken (a previous operation failed or was cancelled)")]
    Broken,
    #[error("timeout: {0}")]
    Timeout(&'static str),
    #[error("tls error: {0}")]
    Tls(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone)]
pub struct BatchRowError {
    pub row: i32,
    pub code: i32,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, Error>;
