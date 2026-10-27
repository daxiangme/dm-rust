use std::borrow::Cow;
use std::sync::Arc;

use crate::column::DmColumn;
use crate::connection::DmConnection;
use crate::error::map_err;
use crate::query_result::DmQueryResult;
use crate::row::DmRow;
use crate::statement::DmStatement;
use crate::type_info::DmTypeInfo;
use crate::Dm;
use dm_conn::{ResultSet, Row as ConnRow};
use futures_core::future::BoxFuture;
use futures_core::stream::BoxStream;
use futures_util::TryStreamExt;
use sqlx_core::describe::Describe;
use sqlx_core::error::Error;
use sqlx_core::executor::{Execute, Executor};
use sqlx_core::logger::QueryLogger;
use sqlx_core::Either;

fn columns_from(cols: &[dm_proto::msg::Column]) -> Arc<Vec<DmColumn>> {
    Arc::new(
        cols.iter()
            .enumerate()
            .map(|(i, c)| DmColumn::from_proto(i, c))
            .collect(),
    )
}

fn affected(n: i64) -> u64 {
    if n < 0 {
        0
    } else {
        n as u64
    }
}

impl<'c> Executor<'c> for &'c mut DmConnection {
    type Database = Dm;

    fn fetch_many<'e, 'q, E>(
        self,
        mut query: E,
    ) -> BoxStream<'e, Result<Either<DmQueryResult, DmRow>, Error>>
    where
        'c: 'e,
        'q: 'e,
        E: 'q + Execute<'q, Self::Database>,
    {
        let sql = query.sql();
        let arguments = query.take_arguments().map_err(Error::Encode);
        Box::pin(sqlx_core::try_stream! {
            self.ensure_ready().await?;
            let arguments = arguments?;
            let params = match arguments {
                Some(args) => args.values,
                None => Vec::new(),
            };
            let mut logger = QueryLogger::new(sql, self.log_settings.clone());
            // query_multi 会把行全部取回。这里的 Stream 只是在内存里逐行交出。
            // 进行中的 query_multi 被取消时，dm-conn 把连接标成 broken。
            let multi = self.inner.query_multi(sql, &params).await.map_err(map_err)?;
            for result in multi.results {
                match result {
                    ResultSet::Query(query_result) => {
                        let columns = columns_from(&query_result.columns);
                        for row in query_result.rows {
                            logger.increment_rows_returned();
                            r#yield!(Either::Right(owned_row(columns.clone(), row)));
                        }
                        r#yield!(Either::Left(DmQueryResult::new(0, None)));
                    }
                    ResultSet::Update(exec) => {
                        let rows_affected = affected(exec.rows_affected);
                        logger.increase_rows_affected(rows_affected);
                        let last_insert_id = exec.insert_ids.last().copied();
                        r#yield!(Either::Left(DmQueryResult::new(rows_affected, last_insert_id)));
                    }
                }
            }
            logger.finish();
            Ok(())
        })
    }

    fn fetch_optional<'e, 'q, E>(self, query: E) -> BoxFuture<'e, Result<Option<DmRow>, Error>>
    where
        'c: 'e,
        'q: 'e,
        E: 'q + Execute<'q, Self::Database>,
    {
        let mut stream = self.fetch_many(query);
        Box::pin(async move {
            while let Some(step) = stream.try_next().await? {
                if let Either::Right(row) = step {
                    return Ok(Some(row));
                }
            }
            Ok(None)
        })
    }

    fn prepare_with<'e, 'q: 'e>(
        self,
        sql: &'q str,
        _parameters: &'e [DmTypeInfo],
    ) -> BoxFuture<'e, Result<DmStatement<'q>, Error>>
    where
        'c: 'e,
    {
        // 参数类型切片用不上：DM 的 PREPARE 不接受客户端提示来改推断。
        Box::pin(async move {
            self.ensure_ready().await?;
            let info = self.inner.prepare(sql).await.map_err(map_err)?;
            Ok(statement_from(Cow::Borrowed(sql), &info))
        })
    }

    fn describe<'e, 'q: 'e>(self, sql: &'q str) -> BoxFuture<'e, Result<Describe<Dm>, Error>>
    where
        'c: 'e,
    {
        Box::pin(async move {
            self.ensure_ready().await?;
            let info = self.inner.prepare(sql).await.map_err(map_err)?;
            let columns: Vec<DmColumn> = info
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| DmColumn::from_proto(i, c))
                .collect();
            // 实测：0 = NOT NULL，1 = 可空。其它值不解释。
            let nullable = info
                .columns
                .iter()
                .map(|c| match c.nullable {
                    0 => Some(false),
                    1 => Some(true),
                    _ => None,
                })
                .collect();
            let parameters = info
                .params
                .iter()
                .map(|p| DmTypeInfo::from_code(p.col_type))
                .collect();
            Ok(Describe {
                columns,
                parameters: Some(Either::Left(parameters)),
                nullable,
            })
        })
    }
}

fn owned_row(columns: Arc<Vec<DmColumn>>, row: ConnRow) -> DmRow {
    DmRow {
        columns,
        values: row.values,
    }
}

fn statement_from<'q>(sql: Cow<'q, str>, info: &dm_conn::StatementInfo) -> DmStatement<'q> {
    let columns = info
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| DmColumn::from_proto(i, c))
        .collect();
    let parameters = info
        .params
        .iter()
        .map(|p| DmTypeInfo::from_code(p.col_type))
        .collect();
    DmStatement::new(sql, columns, parameters)
}
