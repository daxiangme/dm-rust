//! 存储过程 / 函数调用的参数与返回值。
use dm_proto::types::*;
use dm_proto::Value;

use crate::row::QueryResult;

/// 服务端类型提示。PREPARE 已经给出确定类型时，调用按服务端类型编解码，`TypeHint` 只在类型未知时使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeHint {
    pub col_type: i32,
    pub prec: i32,
    pub scale: i32,
}

impl TypeHint {
    pub const fn new(col_type: i32, prec: i32, scale: i32) -> Self {
        TypeHint { col_type, prec, scale }
    }
    pub const fn int() -> Self {
        Self::new(INT, 10, 0)
    }
    pub const fn bigint() -> Self {
        Self::new(BIGINT, 19, 0)
    }
    pub const fn varchar(prec: i32) -> Self {
        Self::new(VARCHAR, prec, 0)
    }
    pub const fn decimal(prec: i32, scale: i32) -> Self {
        Self::new(DECIMAL, prec, scale)
    }
    pub const fn date() -> Self {
        Self::new(DATE, 3, 0)
    }
    pub const fn time() -> Self {
        Self::new(TIME, 5, 6)
    }
    pub const fn datetime() -> Self {
        Self::new(DATETIME, 8, 6)
    }
    pub const fn float() -> Self {
        Self::new(REAL, 24, 0)
    }
    pub const fn double() -> Self {
        Self::new(DOUBLE, 53, 0)
    }
    /// `SYS_REFCURSOR` / 类型码 CURSOR(120)。
    pub const fn cursor() -> Self {
        Self::new(CURSOR, 0, 0)
    }
}

/// `Connection::call` 的一个参数。
#[derive(Debug, Clone)]
pub enum Param {
    In(Value),
    /// OUT。`hint` 仅当 PREPARE 未给出类型（UNKNOWN）时用于声明类型。
    Out(TypeHint),
    InOut(Value),
}

impl Param {
    pub fn out_int() -> Self {
        Param::Out(TypeHint::int())
    }
    pub fn out_cursor() -> Self {
        Param::Out(TypeHint::cursor())
    }
    pub(crate) fn io_type(&self) -> i16 {
        match self {
            Param::In(_) => 0,
            Param::Out(_) => 1,
            Param::InOut(_) => 2,
        }
    }
}

/// 一个 OUT 或 INOUT 参数的返回值。
#[derive(Debug, Clone)]
pub enum OutValue {
    Value(Value),
    /// REF CURSOR。行已按 `query` 同样的方式取完（含后续 FETCH）。
    Cursor(QueryResult),
}

impl OutValue {
    pub fn as_value(&self) -> Option<&Value> {
        match self {
            OutValue::Value(v) => Some(v),
            OutValue::Cursor(_) => None,
        }
    }
    pub fn as_cursor(&self) -> Option<&QueryResult> {
        match self {
            OutValue::Cursor(q) => Some(q),
            OutValue::Value(_) => None,
        }
    }
}

/// `Connection::call` 的结果。`outs` 只含 OUT 与 INOUT，顺序与参数列表一致。
#[derive(Debug, Clone)]
pub struct CallResult {
    pub outs: Vec<OutValue>,
    /// EXECUTE 响应里的影响行数。游标调用上通常是第一个结果集的行数。
    pub rows_affected: i64,
    pub print_msg: String,
}

/// 把 JDBC 转义 `{call ...}` / `{? = call ...}` 改写成服务端接受的匿名块。
/// 服务端不认识花括号语法（语法错误 -2007）。已经是 `begin` / `declare` / `call` 的文本原样返回。
pub fn rewrite_jdbc_call(sql: &str) -> String {
    let trimmed = sql.trim();
    let candidate = trimmed.trim_end_matches(';').trim();
    let Some(inner) = candidate.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return sql.to_string();
    };
    let inner = inner.trim();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut returning = false;
    if bytes.get(i) == Some(&b'?') {
        returning = true;
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            return sql.to_string();
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    }
    let rest = &inner[i..];
    let is_call = rest.len() >= 4
        && rest[..4].eq_ignore_ascii_case("call")
        && rest.as_bytes().get(4).map(|b| b.is_ascii_whitespace()).unwrap_or(true);
    if !is_call {
        return sql.to_string();
    }
    let invocation = rest[4..].trim();
    if invocation.is_empty() {
        return sql.to_string();
    }
    if returning {
        format!("begin ? := {invocation}; end;")
    } else {
        format!("begin {invocation}; end;")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jdbc_escape_rewrite() {
        assert_eq!(rewrite_jdbc_call("{call P(?, ?)}"), "begin P(?, ?); end;");
        assert_eq!(rewrite_jdbc_call("  { call P(?) }; "), "begin P(?); end;");
        assert_eq!(rewrite_jdbc_call("{?=call F(?, ?)}"), "begin ? := F(?, ?); end;");
        assert_eq!(rewrite_jdbc_call("{ ? = call F(?) }"), "begin ? := F(?); end;");
        assert_eq!(rewrite_jdbc_call("begin P(?, ?); end;"), "begin P(?, ?); end;");
        assert_eq!(rewrite_jdbc_call("declare begin P(?); end;"), "declare begin P(?); end;");
        assert_eq!(rewrite_jdbc_call("call P(?)"), "call P(?)");
        assert_eq!(rewrite_jdbc_call("{not a call}"), "{not a call}");
    }
}
