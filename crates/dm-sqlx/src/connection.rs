use std::fmt::{self, Debug, Formatter};

use crate::error::map_err;
use crate::options::DmConnectOptions;
use crate::Dm;
use futures_core::future::BoxFuture;
use sqlx_core::connection::{Connection, LogSettings};
use sqlx_core::error::Error;

/// 丢弃未提交事务时排队的回滚。下一次 ping / 查询 / 提交之前执行。
#[derive(Debug, Clone)]
pub(crate) enum PendingRollback {
    /// 最外层事务：发协议 CMD_ROLLBACK。
    Protocol,
    /// 嵌套保存点：`ROLLBACK TO SAVEPOINT <name>`。
    ToSavepoint(String),
}

/// 一条到 DM8 的连接。
pub struct DmConnection {
    pub(crate) inner: dm_conn::Connection,
    pub(crate) log_settings: LogSettings,
    pub(crate) transaction_depth: usize,
    pub(crate) pending_rollback: Option<PendingRollback>,
}

impl DmConnection {
    pub(crate) fn new(inner: dm_conn::Connection, log_settings: LogSettings) -> Self {
        Self {
            inner,
            log_settings,
            transaction_depth: 0,
            pending_rollback: None,
        }
    }

    /// 执行 `Drop` 排进来的回滚。失败时保留队列，下一次调用再试。
    ///
    /// 直接调用 dm-conn，不再进入 [`crate::DmTransactionManager`]，避免递归。
    pub(crate) async fn ensure_ready(&mut self) -> Result<(), Error> {
        let Some(pending) = self.pending_rollback.clone() else {
            return Ok(());
        };
        let result = match &pending {
            PendingRollback::Protocol => self.inner.rollback().await.map_err(map_err),
            PendingRollback::ToSavepoint(name) => {
                let sql = format!("ROLLBACK TO SAVEPOINT {name}");
                self.inner
                    .execute(&sql, &[])
                    .await
                    .map(|_| ())
                    .map_err(map_err)
            }
        };
        if result.is_ok() {
            self.pending_rollback = None;
        }
        result
    }
}

impl Debug for DmConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmConnection")
            .field("transaction_depth", &self.transaction_depth)
            .field("pending_rollback", &self.pending_rollback)
            .finish()
    }
}

impl Connection for DmConnection {
    type Database = Dm;
    type Options = DmConnectOptions;

    fn close(mut self) -> BoxFuture<'static, Result<(), Error>> {
        Box::pin(async move {
            let rolled = self.ensure_ready().await;
            let closed = self.inner.close().await.map_err(map_err);
            rolled?;
            closed
        })
    }

    fn close_hard(self) -> BoxFuture<'static, Result<(), Error>> {
        // 丢掉 TcpStream 会发 FIN。不发协议回滚。
        Box::pin(async move {
            drop(self);
            Ok(())
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.ensure_ready().await?;
            self.inner.ping().await.map_err(map_err)
        })
    }

    fn begin(&mut self) -> BoxFuture<'_, Result<sqlx_core::transaction::Transaction<'_, Dm>, Error>>
    where
        Self: Sized,
    {
        sqlx_core::transaction::Transaction::begin(self, None)
    }

    fn shrink_buffers(&mut self) {}

    fn flush(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.ensure_ready().await })
    }

    fn should_flush(&self) -> bool {
        self.pending_rollback.is_some()
    }
}
