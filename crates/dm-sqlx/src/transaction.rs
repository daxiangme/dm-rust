use std::borrow::Cow;

use crate::connection::{DmConnection, PendingRollback};
use crate::error::map_err;
use crate::Dm;
use futures_core::future::BoxFuture;
use sqlx_core::error::Error;
use sqlx_core::transaction::TransactionManager;

/// 事务深度 0 走协议（auto_commit 标志 + CMD_COMMIT / CMD_ROLLBACK）。
/// 嵌套层用 SQL `SAVEPOINT` / `ROLLBACK TO SAVEPOINT` / `RELEASE SAVEPOINT`。
/// 这三条 SQL 已在本机 DM8 上执行成功（规格 §16 第 20 条）。
#[derive(Debug)]
pub struct DmTransactionManager;

fn savepoint_name(depth: usize) -> String {
    format!("_sqlx_savepoint_{depth}")
}

impl TransactionManager for DmTransactionManager {
    type Database = Dm;

    fn begin<'conn>(
        conn: &'conn mut DmConnection,
        statement: Option<Cow<'static, str>>,
    ) -> BoxFuture<'conn, Result<(), Error>> {
        Box::pin(async move {
            conn.ensure_ready().await?;
            let depth = conn.transaction_depth;
            if statement.is_some() {
                // 自定义 BEGIN 不会把会话带进协议事务。深度 > 0 时 sqlx 要求保存点，不能换成用户 SQL。
                return Err(if depth > 0 {
                    Error::InvalidSavePointStatement
                } else {
                    Error::Protocol(
                        "custom BEGIN is not supported; DM transactions start by clearing auto-commit, not by a SQL BEGIN"
                            .into(),
                    )
                });
            }
            if depth == 0 {
                conn.inner.begin().await.map_err(map_err)?;
            } else {
                let sql = format!("SAVEPOINT {}", savepoint_name(depth));
                conn.inner.execute(&sql, &[]).await.map_err(map_err)?;
            }
            conn.transaction_depth = depth + 1;
            Ok(())
        })
    }

    fn commit(conn: &mut DmConnection) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            conn.ensure_ready().await?;
            let depth = conn.transaction_depth;
            if depth == 0 {
                return Ok(());
            }
            if depth == 1 {
                conn.inner.commit().await.map_err(map_err)?;
            } else {
                let sql = format!("RELEASE SAVEPOINT {}", savepoint_name(depth - 1));
                conn.inner.execute(&sql, &[]).await.map_err(map_err)?;
            }
            conn.transaction_depth = depth - 1;
            Ok(())
        })
    }

    fn rollback(conn: &mut DmConnection) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            conn.ensure_ready().await?;
            let depth = conn.transaction_depth;
            if depth == 0 {
                return Ok(());
            }
            if depth == 1 {
                conn.inner.rollback().await.map_err(map_err)?;
            } else {
                let sql = format!("ROLLBACK TO SAVEPOINT {}", savepoint_name(depth - 1));
                conn.inner.execute(&sql, &[]).await.map_err(map_err)?;
            }
            conn.transaction_depth = depth - 1;
            Ok(())
        })
    }

    fn start_rollback(conn: &mut DmConnection) {
        let depth = conn.transaction_depth;
        if depth == 0 {
            return;
        }
        conn.pending_rollback = Some(if depth == 1 {
            PendingRollback::Protocol
        } else {
            PendingRollback::ToSavepoint(savepoint_name(depth - 1))
        });
        conn.transaction_depth = depth - 1;
    }

    fn get_transaction_depth(conn: &DmConnection) -> usize {
        conn.transaction_depth
    }
}
