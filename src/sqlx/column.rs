use crate::sqlx::type_info::DmTypeInfo;
use crate::sqlx::Dm;
use crate::proto::msg::Column as ProtoColumn;
use sqlx_core::column::Column;

/// 结果列。名字保持服务端返回的文本（未加引号的标识符通常是大写）。
#[derive(Debug, Clone)]
pub struct DmColumn {
    ordinal: usize,
    name: String,
    type_info: DmTypeInfo,
}

impl DmColumn {
    pub(crate) fn from_proto(ordinal: usize, column: &ProtoColumn) -> Self {
        Self {
            ordinal,
            name: column.name.clone(),
            type_info: DmTypeInfo::from_code(column.col_type),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(ordinal: usize, name: &str, type_code: i32) -> Self {
        Self {
            ordinal,
            name: name.to_string(),
            type_info: DmTypeInfo::from_code(type_code),
        }
    }
}

impl Column for DmColumn {
    type Database = Dm;

    fn ordinal(&self) -> usize {
        self.ordinal
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn type_info(&self) -> &DmTypeInfo {
        &self.type_info
    }
}
