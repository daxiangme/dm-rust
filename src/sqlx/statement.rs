use std::borrow::Cow;

use crate::sqlx::column::DmColumn;
use crate::sqlx::type_info::DmTypeInfo;
use crate::sqlx::Dm;
use crate::sqlx::DmArguments;
use sqlx_core::column::{Column, ColumnIndex};
use sqlx_core::error::Error;
use sqlx_core::statement::Statement;
use sqlx_core::Either;

/// PREPARE 得到的语句元数据。表达式列在执行前可能仍是 `UNKNOWN`。
#[derive(Debug, Clone)]
pub struct DmStatement<'q> {
    sql: Cow<'q, str>,
    columns: Vec<DmColumn>,
    parameters: Vec<DmTypeInfo>,
}

impl<'q> DmStatement<'q> {
    pub(crate) fn new(
        sql: Cow<'q, str>,
        columns: Vec<DmColumn>,
        parameters: Vec<DmTypeInfo>,
    ) -> Self {
        Self {
            sql,
            columns,
            parameters,
        }
    }
}

impl<'q> Statement<'q> for DmStatement<'q> {
    type Database = Dm;

    fn to_owned(&self) -> DmStatement<'static> {
        DmStatement {
            sql: Cow::Owned(self.sql.clone().into_owned()),
            columns: self.columns.clone(),
            parameters: self.parameters.clone(),
        }
    }

    fn sql(&self) -> &str {
        &self.sql
    }

    fn parameters(&self) -> Option<Either<&[DmTypeInfo], usize>> {
        Some(Either::Left(&self.parameters))
    }

    fn columns(&self) -> &[DmColumn] {
        &self.columns
    }

    sqlx_core::impl_statement_query!(DmArguments);
}

impl ColumnIndex<DmStatement<'_>> for &'_ str {
    fn index(&self, statement: &DmStatement<'_>) -> Result<usize, Error> {
        statement
            .columns
            .iter()
            .position(|c| c.name().eq_ignore_ascii_case(self))
            .ok_or_else(|| Error::ColumnNotFound((*self).to_string()))
    }
}
