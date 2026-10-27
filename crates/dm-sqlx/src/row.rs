use std::sync::Arc;

use crate::column::DmColumn;
use crate::value::DmValueRef;
use crate::Dm;
use dm_conn::Value;
use sqlx_core::column::{Column, ColumnIndex};
use sqlx_core::error::Error;
use sqlx_core::row::Row;

/// 一行。列元数据在同一结果集的各行之间共享。
#[derive(Debug, Clone)]
pub struct DmRow {
    pub(crate) columns: Arc<Vec<DmColumn>>,
    pub(crate) values: Vec<Value>,
}

impl Row for DmRow {
    type Database = Dm;

    fn columns(&self) -> &[DmColumn] {
        &self.columns
    }

    fn try_get_raw<I>(&self, index: I) -> Result<DmValueRef<'_>, Error>
    where
        I: ColumnIndex<Self>,
    {
        let index = index.index(self)?;
        let value = self
            .values
            .get(index)
            .ok_or(Error::ColumnIndexOutOfBounds {
                index,
                len: self.values.len(),
            })?;
        let type_info = self
            .columns
            .get(index)
            .map(|c| c.type_info().clone())
            .ok_or(Error::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            })?;
        Ok(DmValueRef { value, type_info })
    }
}

impl ColumnIndex<DmRow> for &'_ str {
    fn index(&self, row: &DmRow) -> Result<usize, Error> {
        row.columns
            .iter()
            .position(|c| c.name().eq_ignore_ascii_case(self))
            .ok_or_else(|| Error::ColumnNotFound((*self).to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::DmColumn;
    use dm_proto::types::INT;
    use sqlx_core::column::ColumnIndex;
    use sqlx_core::row::Row;

    fn sample() -> DmRow {
        let columns = Arc::new(vec![
            DmColumn::from_parts(0, "ID", INT),
            DmColumn::from_parts(1, "Name", INT),
        ]);
        DmRow {
            columns,
            values: vec![Value::Int(1), Value::Null],
        }
    }

    #[test]
    fn column_index_is_case_insensitive() {
        let row = sample();
        assert_eq!(ColumnIndex::index(&"id", &row).unwrap(), 0);
        assert_eq!(ColumnIndex::index(&"NAME", &row).unwrap(), 1);
        assert!(ColumnIndex::index(&"missing", &row).is_err());
        assert!(ColumnIndex::index(&3_usize, &row).is_err());
        assert_eq!(row.len(), 2);
    }
}
