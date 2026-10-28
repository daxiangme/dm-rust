use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};

use sqlx_core::error::{BoxDynError, DatabaseError, Error, ErrorKind};

/// 把连接层的错误映射成 sqlx 的 [`Error`]。
pub(crate) fn map_err(err: crate::conn::Error) -> Error {
    match err {
        crate::conn::Error::Io(e) => Error::Io(e),
        crate::conn::Error::Server { code, message } => {
            Error::Database(Box::new(DmDatabaseError { code, message }))
        }
        crate::conn::Error::Tls(message) => Error::Tls(message.into()),
        crate::conn::Error::Timeout(what) => {
            Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, what))
        }
        crate::conn::Error::Batch(rows) => {
            let code = rows.first().map(|r| r.code).unwrap_or(0);
            let message = if rows.is_empty() {
                "batch execution failed".to_string()
            } else {
                rows.iter()
                    .map(|r| format!("row {}: {} ({})", r.row, r.message, r.code))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            Error::Database(Box::new(DmDatabaseError { code, message }))
        }
        crate::conn::Error::Broken => Error::Protocol("connection is broken".into()),
        crate::conn::Error::Proto(e) => Error::Protocol(e.to_string()),
        crate::conn::Error::Other(message) => Error::Protocol(message),
    }
}

/// 服务端返回的错误。`code` 是包头里的负数错误码。
#[derive(Debug)]
pub struct DmDatabaseError {
    code: i32,
    message: String,
}

impl DmDatabaseError {
    /// 达梦错误码（负数）。
    pub fn dm_code(&self) -> i32 {
        self.code
    }
}

impl Display for DmDatabaseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "DM error {}: {}", self.code, self.message)
    }
}

impl StdError for DmDatabaseError {}

impl DatabaseError for DmDatabaseError {
    fn message(&self) -> &str {
        &self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.code.to_string()))
    }

    fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        // 实测（DM8 8.1.4.6，规格 §16 第 20 条）：-6602 唯一约束，-6609 非空约束。
        // -2501 是登录失败，不是约束。外键 / CHECK 的码没有测过，保持 Other。
        match self.code {
            -6602 => ErrorKind::UniqueViolation,
            -6609 => ErrorKind::NotNullViolation,
            _ => ErrorKind::Other,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DecodeError(pub(crate) String);

impl Display for DecodeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for DecodeError {}

pub(crate) fn dec_err(msg: impl Into<String>) -> BoxDynError {
    Box::new(DecodeError(msg.into()))
}

/// `FromStr` / URL 解析失败。`String` 本身不是 `Error`，包一层才能放进 `Error::Configuration`。
#[derive(Debug)]
pub(crate) struct ConfigError(pub(crate) String);

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for ConfigError {}
