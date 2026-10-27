use std::fmt::{self, Display, Formatter};

use dm_proto::types::*;
use sqlx_core::type_info::TypeInfo;

/// 达梦类型码。`name()` 是类型码对应的规范名，不是服务端 `type_name` 字符串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmTypeInfo {
    type_code: i32,
}

impl DmTypeInfo {
    pub fn new(type_code: i32) -> Self {
        Self { type_code }
    }

    pub fn type_code(&self) -> i32 {
        self.type_code
    }

    pub(crate) fn from_code(type_code: i32) -> Self {
        Self::new(type_code)
    }
}

pub(crate) fn canonical_name(code: i32) -> Option<&'static str> {
    Some(match code {
        CHAR => "CHAR",
        VARCHAR2 => "VARCHAR2",
        VARCHAR => "VARCHAR",
        BIT => "BIT",
        TINYINT => "TINYINT",
        SMALLINT => "SMALLINT",
        INT => "INT",
        BIGINT => "BIGINT",
        DECIMAL => "DECIMAL",
        REAL => "REAL",
        DOUBLE => "DOUBLE",
        BLOB => "BLOB",
        BOOLEAN => "BOOLEAN",
        DATE => "DATE",
        TIME => "TIME",
        DATETIME => "DATETIME",
        BINARY => "BINARY",
        VARBINARY => "VARBINARY",
        CLOB => "CLOB",
        INTERVAL_YM => "INTERVAL YEAR TO MONTH",
        INTERVAL_DT => "INTERVAL DAY TO SECOND",
        TIME_TZ => "TIME WITH TIME ZONE",
        DATETIME_TZ => "TIMESTAMP WITH TIME ZONE",
        DATETIME2 => "DATETIME",
        DATETIME2_TZ => "TIMESTAMP WITH TIME ZONE",
        NULL => "NULL",
        UNKNOWN => "UNKNOWN",
        _ => return None,
    })
}

impl Display for DmTypeInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match canonical_name(self.type_code) {
            Some(name) => f.write_str(name),
            None => write!(f, "UNKNOWN({})", self.type_code),
        }
    }
}

impl TypeInfo for DmTypeInfo {
    fn is_null(&self) -> bool {
        self.type_code == NULL
    }

    fn name(&self) -> &str {
        canonical_name(self.type_code).unwrap_or("UNKNOWN")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_type_codes() {
        assert_eq!(DmTypeInfo::new(INT).name(), "INT");
        assert_eq!(DmTypeInfo::new(VARCHAR).name(), "VARCHAR");
        assert_eq!(DmTypeInfo::new(NULL).name(), "NULL");
        assert!(DmTypeInfo::new(NULL).is_null());
        assert_eq!(DmTypeInfo::new(INT), DmTypeInfo::new(INT));
        assert_ne!(DmTypeInfo::new(INT), DmTypeInfo::new(BIGINT));
        assert_eq!(DmTypeInfo::new(117).name(), "UNKNOWN");
        assert_eq!(DmTypeInfo::new(117).to_string(), "UNKNOWN(117)");
    }
}
