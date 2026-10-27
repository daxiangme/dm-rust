use std::borrow::Cow;

use crate::type_info::DmTypeInfo;
use crate::Dm;
use dm_conn::Value;

/// 一行里的拥有值。类型信息来自结果列，不从 [`Value`] 变体猜测。
#[derive(Debug, Clone)]
pub struct DmValue {
    pub(crate) value: Value,
    pub(crate) type_info: DmTypeInfo,
}

/// 指向 [`DmValue`] 或行缓冲的引用。
#[derive(Debug, Clone)]
pub struct DmValueRef<'r> {
    pub(crate) value: &'r Value,
    pub(crate) type_info: DmTypeInfo,
}

impl sqlx_core::value::Value for DmValue {
    type Database = Dm;

    fn as_ref(&self) -> DmValueRef<'_> {
        DmValueRef {
            value: &self.value,
            type_info: self.type_info.clone(),
        }
    }

    fn type_info(&self) -> Cow<'_, DmTypeInfo> {
        Cow::Borrowed(&self.type_info)
    }

    fn is_null(&self) -> bool {
        self.value.is_null()
    }
}

impl<'r> sqlx_core::value::ValueRef<'r> for DmValueRef<'r> {
    type Database = Dm;

    fn to_owned(&self) -> DmValue {
        DmValue {
            value: self.value.clone(),
            type_info: self.type_info.clone(),
        }
    }

    fn type_info(&self) -> Cow<'_, DmTypeInfo> {
        Cow::Borrowed(&self.type_info)
    }

    fn is_null(&self) -> bool {
        self.value.is_null()
    }
}
