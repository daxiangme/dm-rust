//! 达梦 DM8 的可选 [`sqlx-core`] 0.8 驱动。
//!
//! 默认不启用。打开 `sqlx` feature 后才编译驱动、才依赖 `sqlx-core`：
//!
//! ```toml
//! dm-sqlx = { version = "0.1", features = ["sqlx"] }
//! ```
//!
//! `cargo build --workspace --features sqlx` 与 `cargo build -p dm-sqlx --features sqlx` 都会打开这个 feature。
//! 不带 `--features sqlx` 的工作区构建不会编译 `sqlx-core`。
//!
//! 实现坐在 [`dm_conn::Connection`] 上：查询、执行、协议事务、语句缓存都由 dm-conn 完成。
//! 连接池用 [`sqlx_core::pool::Pool`]，不使用 `dm-pool`。
//!
//! [`sqlx-core`]: https://docs.rs/sqlx-core/0.8.6/sqlx_core/

#[cfg(feature = "sqlx")]
mod arguments;
#[cfg(feature = "sqlx")]
mod column;
#[cfg(feature = "sqlx")]
mod connection;
#[cfg(feature = "sqlx")]
mod database;
#[cfg(feature = "sqlx")]
mod error;
#[cfg(feature = "sqlx")]
mod executor;
#[cfg(feature = "sqlx")]
mod options;
#[cfg(feature = "sqlx")]
mod query_result;
#[cfg(feature = "sqlx")]
mod row;
#[cfg(feature = "sqlx")]
mod statement;
#[cfg(feature = "sqlx")]
mod transaction;
#[cfg(feature = "sqlx")]
mod type_info;
#[cfg(feature = "sqlx")]
mod types;
#[cfg(feature = "sqlx")]
mod value;

#[cfg(feature = "sqlx")]
pub use arguments::DmArguments;
#[cfg(feature = "sqlx")]
pub use column::DmColumn;
#[cfg(feature = "sqlx")]
pub use connection::DmConnection;
#[cfg(feature = "sqlx")]
pub use database::Dm;
#[cfg(feature = "sqlx")]
pub use dm_conn::Decimal;
#[cfg(feature = "sqlx")]
pub use error::DmDatabaseError;
#[cfg(feature = "sqlx")]
pub use options::DmConnectOptions;
#[cfg(feature = "sqlx")]
pub use query_result::DmQueryResult;
#[cfg(feature = "sqlx")]
pub use row::DmRow;
#[cfg(feature = "sqlx")]
pub use sqlx_core::acquire::Acquire;
#[cfg(feature = "sqlx")]
pub use sqlx_core::connection::{ConnectOptions, Connection};
#[cfg(feature = "sqlx")]
pub use sqlx_core::error::Error;
#[cfg(feature = "sqlx")]
pub use sqlx_core::executor::Executor;
#[cfg(feature = "sqlx")]
pub use sqlx_core::pool::{Pool, PoolOptions};
#[cfg(feature = "sqlx")]
pub use sqlx_core::row::Row;
#[cfg(feature = "sqlx")]
pub use statement::DmStatement;
#[cfg(feature = "sqlx")]
pub use transaction::DmTransactionManager;
#[cfg(feature = "sqlx")]
pub use type_info::DmTypeInfo;
#[cfg(feature = "sqlx")]
pub use value::{DmValue, DmValueRef};

#[cfg(feature = "sqlx")]
pub type DmPool = sqlx_core::pool::Pool<Dm>;
#[cfg(feature = "sqlx")]
pub type DmPoolOptions = sqlx_core::pool::PoolOptions<Dm>;
#[cfg(feature = "sqlx")]
pub type DmTransaction<'c> = sqlx_core::transaction::Transaction<'c, Dm>;

/// 集成测试是独立 crate，Cargo 又不允许 optional dev-dependency。
/// 默认构建不打开 `sqlx`，这里的再导出也就不存在，`cargo test --workspace` 不会编译 sqlx-core。
/// 调用方仍应直接依赖 `sqlx-core` 并使用 `sqlx_core::query::query`。
#[doc(hidden)]
#[cfg(feature = "sqlx")]
pub mod driver_test {
    pub use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
    pub use futures_util::TryStreamExt;
    pub use sqlx_core::error::{DatabaseError, ErrorKind};
    pub use sqlx_core::query::query;
}

#[cfg(feature = "sqlx")]
use sqlx_core::{
    impl_acquire, impl_column_index_for_row, impl_column_index_for_statement,
    impl_encode_for_option, impl_into_arguments_for_arguments,
};

#[cfg(feature = "sqlx")]
impl_into_arguments_for_arguments!(DmArguments);
#[cfg(feature = "sqlx")]
impl_acquire!(Dm, DmConnection);
#[cfg(feature = "sqlx")]
impl_column_index_for_row!(DmRow);
#[cfg(feature = "sqlx")]
impl_column_index_for_statement!(DmStatement);
#[cfg(feature = "sqlx")]
impl_encode_for_option!(Dm);
