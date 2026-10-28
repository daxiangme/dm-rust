//! 基于 tokio 的达梦 DM8 异步客户端。
#[allow(clippy::module_inception)]
mod conn;
mod error;
mod options;
mod param;
mod row;
mod stream;

pub use conn::Connection;
pub use crate::proto::msg::Column;
pub use crate::proto::value::{Date, DateTime, Decimal, IntervalDt, IntervalQual, IntervalYm, Time};
pub use crate::proto::wire::Encoding;
pub use crate::proto::Value;
pub use error::{BatchRowError, Error, Result};
pub use options::{CompressAlgo, Compression, ConnectOptions, IsolationLevel};
pub use param::{rewrite_jdbc_call, CallResult, OutValue, Param, TypeHint};
pub use row::{ExecResult, MultiResult, QueryResult, ResultSet, Row, StatementInfo};
