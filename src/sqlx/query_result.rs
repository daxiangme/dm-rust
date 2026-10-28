/// 一条 DML 的结果，或一个 SELECT 结果集结束时的占位（`rows_affected == 0`）。
#[derive(Debug, Default, Clone)]
pub struct DmQueryResult {
    rows_affected: u64,
    last_insert_id: Option<i64>,
}

impl DmQueryResult {
    pub(crate) fn new(rows_affected: u64, last_insert_id: Option<i64>) -> Self {
        Self {
            rows_affected,
            last_insert_id,
        }
    }

    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }

    /// 服务端在 EXECUTE 响应里带回的最后一个自增 id。没有该标志时为 `None`。
    pub fn last_insert_id(&self) -> Option<i64> {
        self.last_insert_id
    }
}

impl Extend<DmQueryResult> for DmQueryResult {
    fn extend<T: IntoIterator<Item = DmQueryResult>>(&mut self, iter: T) {
        for elem in iter {
            self.rows_affected += elem.rows_affected;
            if elem.last_insert_id.is_some() {
                self.last_insert_id = elem.last_insert_id;
            }
        }
    }
}
