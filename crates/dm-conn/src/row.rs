use dm_proto::msg::Column;
use dm_proto::Value;
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
    pub params: Vec<dm_proto::msg::ParamDesc>,
    pub columns: Vec<Column>,
}
