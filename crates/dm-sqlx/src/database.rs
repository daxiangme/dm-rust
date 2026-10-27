use crate::arguments::{DmArgumentBuffer, DmArguments};
use crate::column::DmColumn;
use crate::connection::DmConnection;
use crate::query_result::DmQueryResult;
use crate::row::DmRow;
use crate::statement::DmStatement;
use crate::transaction::DmTransactionManager;
use crate::type_info::DmTypeInfo;
use crate::value::{DmValue, DmValueRef};
use sqlx_core::database::Database;

/// 达梦 DM8 驱动标记类型。
#[derive(Debug)]
pub struct Dm;

impl Database for Dm {
    type Connection = DmConnection;
    type TransactionManager = DmTransactionManager;
    type Row = DmRow;
    type QueryResult = DmQueryResult;
    type Column = DmColumn;
    type TypeInfo = DmTypeInfo;
    type Value = DmValue;
    type ValueRef<'r> = DmValueRef<'r>;
    type Arguments<'q> = DmArguments;
    type ArgumentBuffer<'q> = DmArgumentBuffer;
    type Statement<'q> = DmStatement<'q>;

    const NAME: &'static str = "DM";
    const URL_SCHEMES: &'static [&'static str] = &["dm"];
}
