//! 达梦 DM8 的纯 Rust 异步驱动。
//!
//! 默认 feature 是 `tls`（连接层）。`pool` 打开连接池，`sqlx` 打开 `dm_rust::sqlx`（sqlx-core 0.8）。
//!
//! ```toml
//! dm-rust = { version = "0.1", features = ["tls"] }
//! ```

mod conn;
pub mod proto;

pub use conn::{
    rewrite_jdbc_call, BatchRowError, CallResult, Column, CompressAlgo, Compression, Connection,
    ConnectOptions, Date, DateTime, Decimal, Encoding, Error, ExecResult, IntervalDt, IntervalQual,
    IntervalYm, IsolationLevel, MultiResult, OutValue, Param, QueryResult, Result, ResultSet, Row,
    StatementInfo, Time, TypeHint, Value,
};

#[cfg(feature = "pool")]
pub mod pool;
#[cfg(feature = "pool")]
pub use pool::{Pool, PoolConfig, PoolError, PoolStatus, PooledConnection};

#[cfg(feature = "sqlx")]
pub mod sqlx;
