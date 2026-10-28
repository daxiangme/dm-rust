use crate::proto::msg::Column;
use crate::proto::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Row {
    pub columns: Arc<Vec<Column>>,
    pub values: Vec<Value>,
}

impl Row {
    pub fn get(&self, idx: usize) -> Option<&Value> {
        self.values.get(idx)
    }
    /// 按列名（大小写不敏感）取值。
    pub fn get_by_name(&self, name: &str) -> Option<&Value> {
        self.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name)).and_then(|i| self.values.get(i))
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub columns: Arc<Vec<Column>>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    pub rows_affected: i64,
    /// 批量执行时每一行的影响行数
    pub batch_counts: Vec<i64>,
    /// 自增列的生成值（若服务端返回）
    pub insert_ids: Vec<i64>,
    /// 服务端 PRINT 输出
    pub print_msg: String,
}

/// PREPARE 得到的元信息。
#[derive(Debug, Clone)]
pub struct StatementInfo {
    pub params: Vec<crate::proto::msg::ParamDesc>,
    pub columns: Vec<Column>,
}

/// 一次执行里的一项结果：查询行，或一条更新计数。
#[derive(Debug, Clone)]
pub enum ResultSet {
    Query(QueryResult),
    Update(ExecResult),
}

impl ResultSet {
    pub fn as_query(&self) -> Option<&QueryResult> {
        match self {
            ResultSet::Query(q) => Some(q),
            ResultSet::Update(_) => None,
        }
    }
    pub fn as_update(&self) -> Option<&ExecResult> {
        match self {
            ResultSet::Update(e) => Some(e),
            ResultSet::Query(_) => None,
        }
    }
}

/// [`crate::conn::Connection::query_multi`] 的全部结果，顺序与服务端给出的一致。
#[derive(Debug, Clone, Default)]
pub struct MultiResult {
    pub results: Vec<ResultSet>,
    /// 各响应里的 PRINT 文本，按出现顺序用换行接起来。
    pub print_msg: String,
}

impl MultiResult {
    pub fn len(&self) -> usize {
        self.results.len()
    }
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}
