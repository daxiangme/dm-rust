//! dm-conn：基于 tokio 的达梦 DM8 异步客户端。
mod conn;
mod error;
mod options;
mod row;
mod stream;

pub use conn::Connection;
pub use dm_proto::value::{Date, DateTime, Decimal, IntervalDt, IntervalQual, IntervalYm, Time};
pub use dm_proto::{msg::Column, Value};
pub use error::{BatchRowError, Error, Result};
pub use options::{CompressAlgo, Compression, ConnectOptions, IsolationLevel};
pub use row::{ExecResult, QueryResult, Row, StatementInfo};
