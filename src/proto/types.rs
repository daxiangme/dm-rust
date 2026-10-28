//! 服务端类型码（规格 §11.1）。
pub const CHAR: i32 = 0;
pub const VARCHAR2: i32 = 1;
pub const VARCHAR: i32 = 2;
pub const BIT: i32 = 3;
pub const TINYINT: i32 = 5;
pub const SMALLINT: i32 = 6;
pub const INT: i32 = 7;
pub const BIGINT: i32 = 8;
pub const DECIMAL: i32 = 9;
pub const REAL: i32 = 10;
pub const DOUBLE: i32 = 11;
pub const BLOB: i32 = 12;
pub const BOOLEAN: i32 = 13;
pub const DATE: i32 = 14;
pub const TIME: i32 = 15;
pub const DATETIME: i32 = 16;
pub const BINARY: i32 = 17;
pub const VARBINARY: i32 = 18;
pub const CLOB: i32 = 19;
pub const INTERVAL_YM: i32 = 20;
pub const INTERVAL_DT: i32 = 21;
pub const TIME_TZ: i32 = 22;
pub const DATETIME_TZ: i32 = 23;
pub const DATETIME2: i32 = 26;
pub const DATETIME2_TZ: i32 = 27;
pub const NULL: i32 = 28;
pub const UNKNOWN: i32 = 54;
pub const ARRAY: i32 = 117;
pub const CLASS: i32 = 119;
pub const CURSOR: i32 = 120;
pub const PLTYPE_RECORD: i32 = 121;
pub const SARRAY: i32 = 122;

pub const VARCHAR_PREC: i32 = 8188;
pub const VARBINARY_PREC: i32 = 8188;

// 响应 retSqlType
pub const SQL_INSERT: i16 = 157;
pub const SQL_DELETE: i16 = 158;
pub const SQL_UPDATE: i16 = 159;
pub const SQL_SELECT: i16 = 160;
pub const SQL_SELECT2: i16 = 162;
pub const SQL_SET_TZ: i16 = 165;
pub const SQL_SET_ISO: i16 = 166;
pub const SQL_SET_SCHEMA: i16 = 153;
pub const SQL_EXPLAIN: i16 = 149;

pub fn is_select(t: i16) -> bool {
    t == SQL_SELECT || t == SQL_SELECT2
}

pub fn is_char(t: i32) -> bool {
    matches!(t, CHAR | VARCHAR | VARCHAR2)
}
pub fn is_lob(t: i32) -> bool {
    matches!(t, BLOB | CLOB)
}
