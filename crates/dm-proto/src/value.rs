//! 值类型及其线上编码（规格 §11）。
use crate::error::{ProtoError, Result};
use crate::types::*;
use crate::wire::Encoding;
use bytes::Bytes;
use std::fmt;
use std::str::FromStr;

// ---------- Decimal ----------

/// 十进制定点数：值 = (±)unscaled × 10^-scale，scale ≥ 0；`digits` 为无前导零的十进制数字串（零为 "0"）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal {
    pub negative: bool,
    pub digits: String,
    pub scale: u32,
}

impl Decimal {
    pub fn zero() -> Self {
        Decimal { negative: false, digits: "0".into(), scale: 0 }
    }
    fn normalize(mut self) -> Self {
        let t = self.digits.trim_start_matches('0');
        self.digits = if t.is_empty() { "0".into() } else { t.to_string() };
        while self.scale > 0 && self.digits.ends_with('0') && self.digits != "0" {
            self.digits.pop();
            self.scale -= 1;
        }
        if self.digits == "0" {
            self.negative = false;
            self.scale = 0;
        }
        self
    }
    pub fn from_i64(v: i64) -> Self {
        Decimal { negative: v < 0, digits: v.unsigned_abs().to_string(), scale: 0 }
    }
    pub fn to_f64(&self) -> f64 {
        self.to_string().parse().unwrap_or(f64::NAN)
    }
    pub fn to_i64(&self) -> Option<i64> {
        if self.scale != 0 {
            let s = self.to_string();
            let ip = s.split('.').next()?;
            return ip.parse().ok();
        }
        self.to_string().parse().ok()
    }
    /// 解码 XDEC 字节（规格 §11.2.1）。
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.is_empty() {
            return Err(ProtoError::Invalid("empty decimal".into()));
        }
        if b[0] == 0x80 && b.len() == 1 {
            return Ok(Decimal::zero());
        }
        let pos = b[0] & 0x80 != 0;
        let exp: i32 = if pos { b[0] as i32 - 193 } else { 62 - b[0] as i32 };
        let mut digits = String::new();
        let mut pairs = 0i32;
        for &x in &b[1..] {
            let d = if pos { x as i32 - 1 } else { 101 - x as i32 };
            if !(0..=99).contains(&d) {
                break;
            }
            digits.push_str(&format!("{d:02}"));
            pairs += 1;
        }
        if pairs == 0 {
            return Ok(Decimal::zero());
        }
        let weight = exp * 2 - (pairs * 2 - 2);
        let mut scale = 0u32;
        if weight > 0 {
            digits.push_str(&"0".repeat(weight as usize));
        } else {
            scale = (-weight) as u32;
        }
        Ok(Decimal { negative: !pos, digits, scale }.normalize())
    }
    /// 编码为 XDEC 字节（≤ 21 字节）。
    pub fn encode(&self) -> Result<Vec<u8>> {
        let d = self.clone().normalize();
        if d.digits == "0" {
            return Ok(vec![0x80]);
        }
        let scale = d.scale as usize;
        let mut ds = d.digits.clone();
        if ds.len() <= scale {
            ds = format!("{}{}", "0".repeat(scale - ds.len() + 1), ds);
        }
        let (ip, fp) = ds.split_at(ds.len() - scale);
        let mut ip = ip.to_string();
        let mut fp = fp.to_string();
        if ip.len() % 2 == 1 {
            ip.insert(0, '0');
        }
        if fp.len() % 2 == 1 {
            fp.push('0');
        }
        let mut k = (ip.len() / 2) as i32;
        let all = format!("{ip}{fp}");
        let mut pairs: Vec<u8> = all.as_bytes().chunks(2).map(|c| (c[0] - b'0') * 10 + (c[1] - b'0')).collect();
        let mut lead = 0;
        while lead < pairs.len() && pairs[lead] == 0 {
            lead += 1;
        }
        pairs.drain(..lead);
        k -= lead as i32;
        while pairs.last() == Some(&0) {
            pairs.pop();
        }
        if pairs.len() > 20 {
            return Err(ProtoError::Convert("decimal has more than 40 significant digits".into()));
        }
        let exp = k - 1;
        let mut out = Vec::with_capacity(pairs.len() + 2);
        if !d.negative {
            out.push((exp + 193) as u8);
            out.extend(pairs.iter().map(|p| p + 1));
        } else {
            out.push((62 - exp) as u8);
            out.extend(pairs.iter().map(|p| 101 - p));
            if out.len() < 21 {
                out.push(0x66);
            }
        }
        Ok(out)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        let s = self.scale as usize;
        if s == 0 {
            return f.write_str(&self.digits);
        }
        let ds = if self.digits.len() <= s { format!("{}{}", "0".repeat(s - self.digits.len() + 1), self.digits) } else { self.digits.clone() };
        let (a, b) = ds.split_at(ds.len() - s);
        write!(f, "{a}.{b}")
    }
}

impl FromStr for Decimal {
    type Err = ProtoError;
    fn from_str(s: &str) -> Result<Self> {
        let bad = || ProtoError::Convert(format!("invalid decimal: {s:?}"));
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (mant, e) = match s.find(['e', 'E']) {
            Some(i) => (&s[..i], s[i + 1..].parse::<i32>().map_err(|_| bad())?),
            None => (s, 0),
        };
        let (ip, fp) = mant.split_once('.').unwrap_or((mant, ""));
        if (ip.is_empty() && fp.is_empty()) || !ip.bytes().chain(fp.bytes()).all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        let mut digits = format!("{ip}{fp}");
        let mut scale = fp.len() as i32 - e;
        if scale < 0 {
            digits.push_str(&"0".repeat((-scale) as usize));
            scale = 0;
        }
        Ok(Decimal { negative: neg, digits, scale: scale as u32 }.normalize())
    }
}

// ---------- 日期时间 ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanos: u32,
    /// 时区偏移（分钟），仅 TZ 类型
    pub tz: Option<i16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub date: Date,
    pub time: Time,
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}:{:02}", self.hour, self.minute, self.second)?;
        if self.nanos != 0 {
            let s = format!("{:09}", self.nanos);
            write!(f, ".{}", s.trim_end_matches('0'))?;
        }
        if let Some(tz) = self.tz {
            let (sg, a) = if tz < 0 { ('-', -(tz as i32)) } else { ('+', tz as i32) };
            write!(f, "{sg}{:02}:{:02}", a / 60, a % 60)?;
        }
        Ok(())
    }
}
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.date, self.time)
    }
}

fn dec_date(b: &[u8]) -> Date {
    let raw = (b[0] as u32 | (b[1] as u32) << 8) & 0x7FFF;
    let year = if raw > 9999 { raw as i32 - 0x8000 } else { raw as i32 };
    Date { year, month: ((b[1] >> 7) | ((b[2] & 7) << 1)) as u8, day: (b[2] >> 3) as u8 }
}

fn enc_date(d: &Date, out: &mut Vec<u8>) {
    let y = (d.year as u32) & 0x7FFF;
    out.push((y & 0xFF) as u8);
    out.push(((y >> 8) & 0x7F) as u8 | ((d.month & 1) << 7));
    out.push(((d.month >> 1) & 7) | (d.day << 3));
}

/// `b` 从时间部分起点开始；`nano` 为真表示 9 字节 DATETIME2 布局的纳秒小数。
fn dec_time(b: &[u8], nano: bool) -> Time {
    let hour = b[0] & 0x1F;
    let minute = (b[0] >> 5) | ((b[1] & 7) << 3);
    let second = (b[1] >> 3) | ((b[2] & 1) << 5);
    let nanos = if nano {
        ((b[2] >> 1) as u32) | (b[3] as u32) << 7 | (b[4] as u32) << 15 | ((b[5] & 0x7F) as u32) << 23
    } else {
        (((b[2] >> 1) as u32) | (b[3] as u32) << 7 | ((b[4] & 0x1F) as u32) << 15) * 1000
    };
    Time { hour, minute, second, nanos, tz: None }
}

fn enc_time(t: &Time, nano: bool, out: &mut Vec<u8>) {
    out.push(t.hour | ((t.minute & 7) << 5));
    out.push(((t.minute >> 3) & 7) | ((t.second & 0x1F) << 3));
    let frac = if nano { t.nanos } else { t.nanos / 1000 };
    out.push(((t.second >> 5) & 1) | (((frac & 0x7F) as u8) << 1));
    out.push(((frac >> 7) & 0xFF) as u8);
    if nano {
        out.push(((frac >> 15) & 0xFF) as u8);
        out.push(((frac >> 23) & 0x7F) as u8);
    } else {
        out.push(((frac >> 15) & 0x1F) as u8);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalYm {
    pub years: i32,
    pub months: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalDt {
    pub days: i32,
    pub hours: i32,
    pub minutes: i32,
    pub seconds: i32,
    pub fraction: i32,
}

// ---------- LOB ----------

/// LOB 头（规格 §11.3）。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LobHead {
    pub in_row: bool,
    pub blob_id: i64,
    pub length: i32,
    pub group_id: i16,
    pub file_id: i16,
    pub page_no: i32,
    pub table_id: i32,
    pub col_id: i16,
    pub row_id: i64,
    pub ex_group_id: i16,
    pub ex_file_id: i16,
    pub ex_page_no: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobRef {
    pub is_clob: bool,
    pub head: LobHead,
    /// 行内数据
    pub inline: Option<Bytes>,
}

/// 通用值。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    TinyInt(i8),
    SmallInt(i16),
    Int(i32),
    BigInt(i64),
    Float(f32),
    Double(f64),
    Decimal(Decimal),
    String(String),
    Bytes(Bytes),
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    IntervalYm(IntervalYm),
    IntervalDt(IntervalDt),
    /// 尚未取回的 LOB（连接层会自动解析为 Bytes/String）
    Lob(LobRef),
}

macro_rules! from_impl {
    ($t:ty, $v:ident) => {
        impl From<$t> for Value {
            fn from(x: $t) -> Self {
                Value::$v(x.into())
            }
        }
    };
}
from_impl!(bool, Bool);
from_impl!(i8, TinyInt);
from_impl!(i16, SmallInt);
from_impl!(i32, Int);
from_impl!(i64, BigInt);
from_impl!(f32, Float);
from_impl!(f64, Double);
from_impl!(String, String);
from_impl!(Decimal, Decimal);
from_impl!(Date, Date);
from_impl!(Time, Time);
from_impl!(DateTime, DateTime);
impl From<&str> for Value {
    fn from(x: &str) -> Self {
        Value::String(x.to_string())
    }
}
impl From<Vec<u8>> for Value {
    fn from(x: Vec<u8>) -> Self {
        Value::Bytes(Bytes::from(x))
    }
}
impl From<&[u8]> for Value {
    fn from(x: &[u8]) -> Self {
        Value::Bytes(Bytes::copy_from_slice(x))
    }
}
impl From<Bytes> for Value {
    fn from(x: Bytes) -> Self {
        Value::Bytes(x)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(x: Option<T>) -> Self {
        x.map_or(Value::Null, Into::into)
    }
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Value::Bool(b) => *b as i64,
            Value::TinyInt(v) => *v as i64,
            Value::SmallInt(v) => *v as i64,
            Value::Int(v) => *v as i64,
            Value::BigInt(v) => *v,
            Value::Decimal(d) => d.to_i64()?,
            Value::String(s) => s.trim().parse().ok()?,
            _ => return None,
        })
    }
    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Value::Float(v) => *v as f64,
            Value::Double(v) => *v,
            Value::Decimal(d) => d.to_f64(),
            Value::String(s) => s.trim().parse().ok()?,
            other => other.as_i64()? as f64,
        })
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            Value::String(s) => Some(s.as_bytes()),
            _ => None,
        }
    }
    /// 文本表示（NULL 返回 None）。
    pub fn to_text(&self) -> Option<String> {
        Some(match self {
            Value::Null => return None,
            Value::Bool(b) => (*b as u8).to_string(),
            Value::TinyInt(v) => v.to_string(),
            Value::SmallInt(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::BigInt(v) => v.to_string(),
            Value::Float(v) => v.to_string(),
            Value::Double(v) => v.to_string(),
            Value::Decimal(v) => v.to_string(),
            Value::String(s) => s.clone(),
            Value::Bytes(b) => b.iter().map(|x| format!("{x:02X}")).collect(),
            Value::Date(d) => d.to_string(),
            Value::Time(t) => t.to_string(),
            Value::DateTime(t) => t.to_string(),
            Value::IntervalYm(i) => format!("{}-{}", i.years, i.months),
            Value::IntervalDt(i) => format!("{} {}:{}:{}.{}", i.days, i.hours, i.minutes, i.seconds, i.fraction),
            Value::Lob(_) => "<lob>".into(),
        })
    }

    /// 无服务端类型提示时，按值选择参数类型 (type, prec, scale)。
    pub fn natural_type(&self) -> (i32, i32, i32) {
        match self {
            Value::Null => (VARCHAR, VARCHAR_PREC, 0),
            Value::Bool(_) => (BIT, 1, 0),
            Value::TinyInt(_) => (TINYINT, 3, 0),
            Value::SmallInt(_) => (SMALLINT, 5, 0),
            Value::Int(_) => (INT, 10, 0),
            Value::BigInt(_) => (BIGINT, 19, 0),
            Value::Float(_) => (REAL, 24, 0),
            Value::Double(_) => (DOUBLE, 53, 0),
            Value::Decimal(d) => {
                let nd = d.digits.len() as i32;
                let sc = d.scale as i32;
                if sc < 0 {
                    (DECIMAL, (nd - sc).clamp(1, 38), 0)
                } else {
                    (DECIMAL, nd.max(sc).clamp(1, 38), sc.min(38))
                }
            }
            Value::String(s) => (VARCHAR, bucket_prec(s.len() as i32), 0),
            Value::Bytes(b) => (VARBINARY, bucket_prec(b.len() as i32), 0),
            Value::Date(_) => (DATE, 3, 0),
            Value::Time(t) => if t.tz.is_some() { (TIME_TZ, 7, 6) } else { (TIME, 5, 6) },
            Value::DateTime(t) => if t.time.tz.is_some() { (DATETIME_TZ, 10, 6) } else { (DATETIME, 8, 6) },
            Value::IntervalYm(_) => (INTERVAL_YM, 12, 0),
            Value::IntervalDt(_) => (INTERVAL_DT, 24, 0),
            Value::Lob(_) => (BLOB, 0, 0),
        }
    }
}

/// 字符/二进制参数精度分档（对应服务端精度字段，规格 §9.1）。
pub fn bucket_prec(len: i32) -> i32 {
    if len < VARCHAR_PREC / 4 {
        VARCHAR_PREC / 4
    } else if len < VARCHAR_PREC / 2 {
        VARCHAR_PREC / 2
    } else if len < VARCHAR_PREC * 3 / 4 {
        VARCHAR_PREC * 3 / 4
    } else {
        VARCHAR_PREC
    }
}

// ---------- 解码 / 编码 ----------

fn need(b: &[u8], n: usize, what: &str) -> Result<()> {
    if b.len() < n {
        Err(ProtoError::Invalid(format!("{what}: need {n} bytes, got {}", b.len())))
    } else {
        Ok(())
    }
}

/// 解码 LOB 头。`new_lob` 为登录响应 @43 == 1。
pub fn decode_lob(data: &Bytes, is_clob: bool, new_lob: bool, col_tab: i32, col_col: i16) -> Result<LobRef> {
    need(data, 1, "lob head")?;
    let in_row = data[0] == 1;
    let b = &data[..];
    let mut h = LobHead { in_row, table_id: col_tab, col_id: col_col, ..Default::default() };
    let i64_at = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let i16_at = |o: usize| i16::from_le_bytes(b[o..o + 2].try_into().unwrap());
    if new_lob {
        need(b, 43, "lob head")?;
        h.blob_id = i64_at(1);
        h.length = i32_at(9);
        h.group_id = i16_at(13);
        h.file_id = i16_at(15);
        h.page_no = i32_at(17);
        h.table_id = i32_at(21);
        h.col_id = i16_at(25);
        h.row_id = i64_at(27);
        h.ex_group_id = i16_at(35);
        h.ex_file_id = i16_at(37);
        h.ex_page_no = i32_at(39);
        let inline = if in_row { Some(data.slice(43..)) } else { None };
        Ok(LobRef { is_clob, head: h, inline })
    } else {
        need(b, 13, "lob head")?;
        h.blob_id = i64_at(1);
        h.length = i32_at(9);
        if in_row {
            Ok(LobRef { is_clob, head: h, inline: Some(data.slice(13..)) })
        } else {
            need(b, 21, "lob head")?;
            h.group_id = i16_at(13);
            h.file_id = i16_at(15);
            h.page_no = i32_at(17);
            Ok(LobRef { is_clob, head: h, inline: None })
        }
    }
}

/// 列元信息（解码所需）。
pub struct ColumnCtx {
    pub col_type: i32,
    pub new_lob: bool,
    pub lob_tab_id: i32,
    pub lob_col_id: i16,
}

pub fn decode_value(c: &ColumnCtx, data: Bytes, enc: Encoding) -> Result<Value> {
    let b = &data[..];
    Ok(match c.col_type {
        BIT | BOOLEAN => {
            need(b, 1, "bool")?;
            Value::Bool(b[0] != 0)
        }
        TINYINT => {
            need(b, 1, "tinyint")?;
            Value::TinyInt(b[0] as i8)
        }
        SMALLINT => {
            need(b, 2, "smallint")?;
            Value::SmallInt(i16::from_le_bytes(b[..2].try_into().unwrap()))
        }
        INT => {
            need(b, 4, "int")?;
            Value::Int(i32::from_le_bytes(b[..4].try_into().unwrap()))
        }
        BIGINT => {
            need(b, 8, "bigint")?;
            Value::BigInt(i64::from_le_bytes(b[..8].try_into().unwrap()))
        }
        REAL => {
            need(b, 4, "real")?;
            Value::Float(f32::from_le_bytes(b[..4].try_into().unwrap()))
        }
        DOUBLE => {
            need(b, 8, "double")?;
            Value::Double(f64::from_le_bytes(b[..8].try_into().unwrap()))
        }
        DECIMAL => Value::Decimal(Decimal::decode(b)?),
        CHAR | VARCHAR | VARCHAR2 => Value::String(enc.decode(b)),
        BINARY | VARBINARY => Value::Bytes(data),
        DATE => {
            need(b, 3, "date")?;
            Value::Date(dec_date(b))
        }
        TIME | TIME_TZ => {
            need(b, 5, "time")?;
            let mut t = dec_time(b, false);
            if b.len() >= 7 {
                t.tz = Some(i16::from_le_bytes(b[5..7].try_into().unwrap()));
            }
            Value::Time(t)
        }
        DATETIME | DATETIME_TZ | DATETIME2 | DATETIME2_TZ => {
            need(b, 8, "datetime")?;
            let nano = matches!(b.len(), 9 | 11);
            let tzf = matches!(b.len(), 10 | 11);
            if nano {
                need(b, 9, "datetime2")?;
            }
            let mut t = dec_time(&b[3..], nano);
            if tzf {
                t.tz = Some(i16::from_le_bytes(b[b.len() - 2..].try_into().unwrap()));
            }
            Value::DateTime(DateTime { date: dec_date(b), time: t })
        }
        INTERVAL_YM => {
            need(b, 8, "interval ym")?;
            Value::IntervalYm(IntervalYm {
                years: i32::from_le_bytes(b[0..4].try_into().unwrap()),
                months: i32::from_le_bytes(b[4..8].try_into().unwrap()),
            })
        }
        INTERVAL_DT => {
            need(b, 20, "interval dt")?;
            let g = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            Value::IntervalDt(IntervalDt { days: g(0), hours: g(4), minutes: g(8), seconds: g(12), fraction: g(16) })
        }
        BLOB | CLOB => Value::Lob(decode_lob(&data, c.col_type == CLOB, c.new_lob, c.lob_tab_id, c.lob_col_id)?),
        t => return Err(ProtoError::Unsupported(format!("column type {t}"))),
    })
}

fn conv<T: TryFrom<i64>>(v: &Value, what: &str) -> Result<T> {
    let i = v.as_i64().ok_or_else(|| ProtoError::Convert(format!("cannot convert {v:?} to {what}")))?;
    T::try_from(i).map_err(|_| ProtoError::Convert(format!("{i} out of range for {what}")))
}

/// 把值编码为目标服务端类型的字节（NULL 由调用方处理）。
pub fn encode_value(v: &Value, ty: i32, enc: Encoding) -> Result<Vec<u8>> {
    let bad = || ProtoError::Convert(format!("cannot convert {v:?} to server type {ty}"));
    Ok(match ty {
        BIT | BOOLEAN => vec![match v {
            Value::Bool(b) => *b as u8,
            _ => (conv::<i64>(v, "bool")? != 0) as u8,
        }],
        TINYINT => vec![conv::<i8>(v, "tinyint")? as u8],
        SMALLINT => conv::<i16>(v, "smallint")?.to_le_bytes().to_vec(),
        INT => conv::<i32>(v, "int")?.to_le_bytes().to_vec(),
        BIGINT => conv::<i64>(v, "bigint")?.to_le_bytes().to_vec(),
        REAL => (v.as_f64().ok_or_else(bad)? as f32).to_le_bytes().to_vec(),
        DOUBLE => v.as_f64().ok_or_else(bad)?.to_le_bytes().to_vec(),
        DECIMAL => match v {
            Value::Decimal(d) => d.encode()?,
            Value::Float(_) | Value::Double(_) => format!("{}", v.as_f64().unwrap()).parse::<Decimal>()?.encode()?,
            Value::String(s) => s.parse::<Decimal>()?.encode()?,
            _ => Decimal::from_i64(v.as_i64().ok_or_else(bad)?).encode()?,
        },
        CHAR | VARCHAR | VARCHAR2 | CLOB => match v {
            Value::String(s) => enc.encode(s),
            Value::Bytes(b) if ty == CLOB => b.to_vec(),
            other => enc.encode(&other.to_text().ok_or_else(bad)?),
        },
        BINARY | VARBINARY | BLOB => match v {
            Value::Bytes(b) => b.to_vec(),
            Value::String(s) => s.as_bytes().to_vec(),
            _ => return Err(bad()),
        },
        DATE => match v {
            Value::Date(d) => {
                let mut o = Vec::with_capacity(3);
                enc_date(d, &mut o);
                o
            }
            Value::DateTime(t) => {
                let mut o = Vec::with_capacity(3);
                enc_date(&t.date, &mut o);
                o
            }
            _ => return Err(bad()),
        },
        TIME | TIME_TZ => match v {
            Value::Time(t) => time_bytes(t, ty == TIME_TZ),
            Value::DateTime(t) => time_bytes(&t.time, ty == TIME_TZ),
            _ => return Err(bad()),
        },
        DATETIME | DATETIME_TZ | DATETIME2 | DATETIME2_TZ => {
            let (d, t) = match v {
                Value::DateTime(x) => (x.date, x.time),
                Value::Date(d) => (*d, Time { hour: 0, minute: 0, second: 0, nanos: 0, tz: None }),
                _ => return Err(bad()),
            };
            let nano = ty == DATETIME2 || ty == DATETIME2_TZ;
            let mut o = Vec::with_capacity(11);
            enc_date(&d, &mut o);
            enc_time(&t, nano, &mut o);
            if ty == DATETIME_TZ || ty == DATETIME2_TZ {
                o.extend_from_slice(&t.tz.unwrap_or(0).to_le_bytes());
            }
            o
        }
        t => return Err(ProtoError::Unsupported(format!("parameter type {t}"))),
    })
}

fn time_bytes(t: &Time, tz: bool) -> Vec<u8> {
    let mut o = Vec::with_capacity(7);
    enc_time(t, false, &mut o);
    if tz {
        o.extend_from_slice(&t.tz.unwrap_or(0).to_le_bytes());
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt_dec(s: &str) {
        let d: Decimal = s.parse().unwrap();
        let e = d.encode().unwrap();
        let back = Decimal::decode(&e).unwrap();
        assert_eq!(back, d, "{s} -> {e:?}");
        assert_eq!(back.to_string(), s.trim_start_matches('+'));
    }

    #[test]
    fn decimal_roundtrip() {
        for s in ["0", "1", "-1", "100", "-100", "0.5", "-0.5", "12345.6789", "0.0001", "-0.0001", "99999999999999999999999999999999999999", "-123456789012345678.123456789", "10", "1000000"] {
            rt_dec(s);
        }
    }

    #[test]
    fn decimal_known_bytes() {
        assert_eq!(Decimal::from_i64(1).encode().unwrap(), [193, 2]);
        assert_eq!(Decimal::from_i64(100).encode().unwrap(), [194, 2]);
        assert_eq!("0.5".parse::<Decimal>().unwrap().encode().unwrap(), [192, 51]);
        assert_eq!(Decimal::from_i64(-1).encode().unwrap(), [62, 100, 0x66]);
        assert_eq!(Decimal::zero().encode().unwrap(), [0x80]);
        assert_eq!(Decimal::decode(&[193, 2]).unwrap().to_string(), "1");
        assert_eq!("1e3".parse::<Decimal>().unwrap().to_string(), "1000");
    }

    #[test]
    fn datetime_roundtrip() {
        let enc = Encoding::Utf8;
        let t = Time { hour: 23, minute: 59, second: 58, nanos: 123_456_000, tz: None };
        let d = Date { year: 2026, month: 10, day: 27 };
        for ty in [DATETIME, DATETIME2] {
            let v = Value::DateTime(DateTime { date: d, time: t });
            let b = encode_value(&v, ty, enc).unwrap();
            assert_eq!(b.len(), if ty == DATETIME { 8 } else { 9 });
            let ctx = ColumnCtx { col_type: ty, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };
            assert_eq!(decode_value(&ctx, Bytes::from(b), enc).unwrap(), v);
        }
        let tz = Time { tz: Some(480), ..t };
        let v = Value::DateTime(DateTime { date: d, time: tz });
        let b = encode_value(&v, DATETIME2_TZ, enc).unwrap();
        assert_eq!(b.len(), 11);
        let ctx = ColumnCtx { col_type: DATETIME2_TZ, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };
        assert_eq!(decode_value(&ctx, Bytes::from(b), enc).unwrap(), v);
        // 日期
        let b = encode_value(&Value::Date(d), DATE, enc).unwrap();
        let ctx = ColumnCtx { col_type: DATE, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };
        assert_eq!(decode_value(&ctx, Bytes::from(b), enc).unwrap(), Value::Date(d));
        // 时间
        let b = encode_value(&Value::Time(t), TIME, enc).unwrap();
        assert_eq!(b.len(), 5);
        let ctx = ColumnCtx { col_type: TIME, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };
        assert_eq!(decode_value(&ctx, Bytes::from(b), enc).unwrap(), Value::Time(t));
    }

    #[test]
    fn known_date_bytes() {
        // 2000-01-01 → year 2000 = 0x07D0, month 1, day 1
        let mut o = vec![];
        enc_date(&Date { year: 2000, month: 1, day: 1 }, &mut o);
        assert_eq!(o, [0xD0, 0x07 | 0x80, 0x08]);
    }

    #[test]
    fn ints_and_strings() {
        let enc = Encoding::Utf8;
        assert_eq!(encode_value(&Value::Int(5), BIGINT, enc).unwrap(), 5i64.to_le_bytes());
        assert!(encode_value(&Value::Int(300), TINYINT, enc).is_err());
        assert_eq!(encode_value(&"héllo".into(), VARCHAR, enc).unwrap(), "héllo".as_bytes());
        let ctx = ColumnCtx { col_type: VARCHAR, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };
        assert_eq!(decode_value(&ctx, Bytes::from("héllo"), enc).unwrap(), Value::String("héllo".into()));
    }

    #[test]
    fn lob_head() {
        let mut h = vec![1u8];
        h.extend_from_slice(&42i64.to_le_bytes());
        h.extend_from_slice(&3i32.to_le_bytes());
        h.extend_from_slice(&[0u8; 30]);
        h.extend_from_slice(b"abc");
        let l = decode_lob(&Bytes::from(h), false, true, 0, 0).unwrap();
        assert!(l.head.in_row);
        assert_eq!(l.head.blob_id, 42);
        assert_eq!(l.inline.unwrap(), Bytes::from_static(b"abc"));
    }
}
