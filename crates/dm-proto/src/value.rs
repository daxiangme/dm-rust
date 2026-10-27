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

/// INTERVAL 的 qualifier。数值是值字节 scale 字里的 qualifier 字节（§11.2.3）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IntervalQual {
    Year = 0,
    YearToMonth = 1,
    Month = 2,
    Day = 3,
    DayToHour = 4,
    DayToMinute = 5,
    DayToSecond = 6,
    Hour = 7,
    HourToMinute = 8,
    HourToSecond = 9,
    Minute = 10,
    MinuteToSecond = 11,
    Second = 12,
}

impl IntervalQual {
    pub fn is_ym(self) -> bool {
        (self as u8) <= Self::Month as u8
    }
    pub fn is_dt(self) -> bool {
        !self.is_ym()
    }
    /// 文本里是否带小数秒（SECOND 结尾的限定符）。
    fn has_seconds_field(self) -> bool {
        matches!(self, Self::DayToSecond | Self::HourToSecond | Self::MinuteToSecond | Self::Second)
    }
}

impl TryFrom<u8> for IntervalQual {
    type Error = ProtoError;
    fn try_from(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Self::Year,
            1 => Self::YearToMonth,
            2 => Self::Month,
            3 => Self::Day,
            4 => Self::DayToHour,
            5 => Self::DayToMinute,
            6 => Self::DayToSecond,
            7 => Self::Hour,
            8 => Self::HourToMinute,
            9 => Self::HourToSecond,
            10 => Self::Minute,
            11 => Self::MinuteToSecond,
            12 => Self::Second,
            _ => return Err(ProtoError::Invalid(format!("interval qualifier {v}"))),
        })
    }
}

/// INTERVAL YEAR / MONTH / YEAR TO MONTH。
///
/// 负间隔的 `years` 与 `months` 同为负或零。`lead_prec` 为前导精度（1..=9）；
/// `frac_prec` 是 scale 低 4 位，年月类型在服务端恒为 6。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalYm {
    pub years: i32,
    pub months: i32,
    pub lead_prec: u8,
    pub frac_prec: u8,
    pub qualifier: IntervalQual,
}

/// INTERVAL DAY / HOUR / MINUTE / SECOND 及其 TO 组合。
///
/// `nanos` 为小数秒（纳秒）。线上只保留到微秒，编码时向零截断到 1000 纳秒的倍数。
/// 负间隔的各字段同为负或零。`lead_prec` 1..=9，`frac_prec` 0..=6。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalDt {
    pub days: i32,
    pub hours: i32,
    pub minutes: i32,
    pub seconds: i32,
    pub nanos: i32,
    pub lead_prec: u8,
    pub frac_prec: u8,
    pub qualifier: IntervalQual,
}

impl IntervalYm {
    /// `INTERVAL year-month YEAR(9) TO MONTH`（字面量的默认前导精度是 9）。
    pub fn new(years: i32, months: i32) -> Self {
        Self { years, months, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::YearToMonth }
    }
    pub fn year(years: i32) -> Self {
        Self { years, months: 0, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::Year }
    }
    pub fn month(months: i32) -> Self {
        Self { years: 0, months, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::Month }
    }
    pub fn scale_word(self) -> i32 {
        pack_interval_scale(self.lead_prec, self.frac_prec, self.qualifier).unwrap_or(-1)
    }
}

impl IntervalDt {
    /// `INTERVAL day hour:minute:second.fraction DAY(9) TO SECOND(6)`。
    pub fn new(days: i32, hours: i32, minutes: i32, seconds: i32, nanos: i32) -> Self {
        Self { days, hours, minutes, seconds, nanos, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::DayToSecond }
    }
    pub fn scale_word(self) -> i32 {
        pack_interval_scale(self.lead_prec, self.frac_prec, self.qualifier).unwrap_or(-1)
    }
}

/// `scale = qualifier<<8 | lead_prec<<4 | frac_prec`（高 16 位为 0）。
fn pack_interval_scale(lead: u8, frac: u8, q: IntervalQual) -> Result<i32> {
    if lead > 0xF || frac > 0xF {
        return Err(ProtoError::Convert(format!("interval precision lead={lead} frac={frac} does not fit in 4 bits")));
    }
    Ok(((q as i32) << 8) | ((lead as i32) << 4) | (frac as i32))
}

fn unpack_interval_scale(scale: i32) -> Result<(u8, u8, IntervalQual)> {
    let u = scale as u32;
    let lead = ((u >> 4) & 0xF) as u8;
    let frac = (u & 0xF) as u8;
    let q = IntervalQual::try_from(((u >> 8) & 0xFF) as u8)?;
    Ok((lead, frac, q))
}

fn interval_neg(parts: &[i32]) -> bool {
    parts.iter().any(|p| *p < 0)
}

fn interval_num(n: i32) -> String {
    n.unsigned_abs().to_string()
}

/// 小数秒按 frac_prec 位输出（线上是微秒，最多 6 位；更长则补 0）。
fn interval_frac_digits(nanos: i32, prec: u8) -> String {
    let micros = nanos.unsigned_abs() / 1000;
    let mut digits = format!("{micros:06}");
    let n = prec as usize;
    if n > digits.len() {
        digits.extend(std::iter::repeat('0').take(n - digits.len()));
    } else {
        digits.truncate(n);
    }
    digits
}

fn interval_qual_sql(q: IntervalQual, lead: u8, frac: u8) -> String {
    match q {
        IntervalQual::Year => format!("YEAR({lead})"),
        IntervalQual::Month => format!("MONTH({lead})"),
        IntervalQual::YearToMonth => format!("YEAR({lead}) TO MONTH"),
        IntervalQual::Day => format!("DAY({lead})"),
        IntervalQual::Hour => format!("HOUR({lead})"),
        IntervalQual::Minute => format!("MINUTE({lead})"),
        IntervalQual::Second => format!("SECOND({lead}, {frac})"),
        IntervalQual::DayToHour => format!("DAY({lead}) TO HOUR"),
        IntervalQual::DayToMinute => format!("DAY({lead}) TO MINUTE"),
        IntervalQual::DayToSecond => format!("DAY({lead}) TO SECOND({frac})"),
        IntervalQual::HourToMinute => format!("HOUR({lead}) TO MINUTE"),
        IntervalQual::HourToSecond => format!("HOUR({lead}) TO SECOND({frac})"),
        IntervalQual::MinuteToSecond => format!("MINUTE({lead}) TO SECOND({frac})"),
    }
}

fn fmt_interval(sign_parts: &[i32], body: &str, q: IntervalQual, lead: u8, frac: u8) -> String {
    let sign = if interval_neg(sign_parts) { "-" } else { "" };
    format!("INTERVAL {sign}'{body}' {}", interval_qual_sql(q, lead, frac))
}

impl fmt::Display for IntervalYm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (neg, body) = match self.qualifier {
            IntervalQual::Year => (self.years < 0, interval_num(self.years)),
            IntervalQual::Month => (self.months < 0, interval_num(self.months)),
            _ => (self.years < 0 || self.months < 0, format!("{}-{}", interval_num(self.years), interval_num(self.months))),
        };
        let sign = if neg { "-" } else { "" };
        write!(f, "INTERVAL {sign}'{body}' {}", interval_qual_sql(self.qualifier, self.lead_prec, self.frac_prec))
    }
}

impl fmt::Display for IntervalDt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sec = |out: &mut String| {
            out.push_str(&interval_num(self.seconds));
            if self.qualifier.has_seconds_field() && self.frac_prec > 0 {
                out.push('.');
                out.push_str(&interval_frac_digits(self.nanos, self.frac_prec));
            }
        };
        let (parts, body) = match self.qualifier {
            IntervalQual::Day => (vec![self.days], interval_num(self.days)),
            IntervalQual::Hour => (vec![self.hours], interval_num(self.hours)),
            IntervalQual::Minute => (vec![self.minutes], interval_num(self.minutes)),
            IntervalQual::Second => {
                let mut b = String::new();
                sec(&mut b);
                (vec![self.seconds, self.nanos], b)
            }
            IntervalQual::DayToHour => (vec![self.days, self.hours], format!("{} {}", interval_num(self.days), interval_num(self.hours))),
            IntervalQual::DayToMinute => {
                (vec![self.days, self.hours, self.minutes], format!("{} {}:{}", interval_num(self.days), interval_num(self.hours), interval_num(self.minutes)))
            }
            IntervalQual::DayToSecond => {
                let mut b = format!("{} {}:{}:", interval_num(self.days), interval_num(self.hours), interval_num(self.minutes));
                sec(&mut b);
                (vec![self.days, self.hours, self.minutes, self.seconds, self.nanos], b)
            }
            IntervalQual::HourToMinute => (vec![self.hours, self.minutes], format!("{}:{}", interval_num(self.hours), interval_num(self.minutes))),
            IntervalQual::HourToSecond => {
                let mut b = format!("{}:{}:", interval_num(self.hours), interval_num(self.minutes));
                sec(&mut b);
                (vec![self.hours, self.minutes, self.seconds, self.nanos], b)
            }
            IntervalQual::MinuteToSecond => {
                let mut b = format!("{}:", interval_num(self.minutes));
                sec(&mut b);
                (vec![self.minutes, self.seconds, self.nanos], b)
            }
            _ => {
                let mut b = format!("{} {}:{}:", interval_num(self.days), interval_num(self.hours), interval_num(self.minutes));
                sec(&mut b);
                (vec![self.days, self.hours, self.minutes, self.seconds, self.nanos], b)
            }
        };
        f.write_str(&fmt_interval(&parts, &body, self.qualifier, self.lead_prec, self.frac_prec))
    }
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
from_impl!(IntervalYm, IntervalYm);
from_impl!(IntervalDt, IntervalDt);
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

impl TryFrom<&Value> for IntervalYm {
    type Error = ProtoError;
    fn try_from(v: &Value) -> Result<Self> {
        match v {
            Value::IntervalYm(i) => Ok(*i),
            _ => Err(ProtoError::Convert(format!("cannot convert {v:?} to interval year to month"))),
        }
    }
}
impl TryFrom<Value> for IntervalYm {
    type Error = ProtoError;
    fn try_from(v: Value) -> Result<Self> {
        IntervalYm::try_from(&v)
    }
}
impl TryFrom<&Value> for IntervalDt {
    type Error = ProtoError;
    fn try_from(v: &Value) -> Result<Self> {
        match v {
            Value::IntervalDt(i) => Ok(*i),
            _ => Err(ProtoError::Convert(format!("cannot convert {v:?} to interval day to second"))),
        }
    }
}
impl TryFrom<Value> for IntervalDt {
    type Error = ProtoError;
    fn try_from(v: Value) -> Result<Self> {
        IntervalDt::try_from(&v)
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
    pub fn as_interval_ym(&self) -> Option<&IntervalYm> {
        match self {
            Value::IntervalYm(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_interval_dt(&self) -> Option<&IntervalDt> {
        match self {
            Value::IntervalDt(v) => Some(v),
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
            Value::IntervalYm(i) => i.to_string(),
            Value::IntervalDt(i) => i.to_string(),
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
            Value::IntervalYm(i) => (INTERVAL_YM, 12, i.scale_word()),
            Value::IntervalDt(i) => (INTERVAL_DT, 24, i.scale_word()),
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
            need(b, 12, "interval ym")?;
            let (lead, frac, qualifier) = unpack_interval_scale(i32::from_le_bytes(b[8..12].try_into().unwrap()))?;
            if !qualifier.is_ym() {
                return Err(ProtoError::Invalid(format!("interval year-month qualifier {qualifier:?}")));
            }
            Value::IntervalYm(IntervalYm {
                years: i32::from_le_bytes(b[0..4].try_into().unwrap()),
                months: i32::from_le_bytes(b[4..8].try_into().unwrap()),
                lead_prec: lead,
                frac_prec: frac,
                qualifier,
            })
        }
        INTERVAL_DT => {
            need(b, 24, "interval dt")?;
            let g = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            let (lead, frac, qualifier) = unpack_interval_scale(g(20))?;
            if !qualifier.is_dt() {
                return Err(ProtoError::Invalid(format!("interval day-time qualifier {qualifier:?}")));
            }
            let micros = g(16);
            let nanos = micros.checked_mul(1000).ok_or_else(|| ProtoError::Invalid(format!("interval fraction {micros} us overflows")))?;
            Value::IntervalDt(IntervalDt { days: g(0), hours: g(4), minutes: g(8), seconds: g(12), nanos, lead_prec: lead, frac_prec: frac, qualifier })
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
/// INTERVAL 的 scale 字取自值本身。
pub fn encode_value(v: &Value, ty: i32, enc: Encoding) -> Result<Vec<u8>> {
    encode_value_with_scale(v, ty, None, enc)
}

/// 同 [`encode_value`]。`scale` 仅用于 INTERVAL：`Some` 时写入该 scale 字
///（参数绑定用，与参数描述里的 scale 保持一致），`None` 时用值自带的精度与 qualifier。
pub fn encode_value_with_scale(v: &Value, ty: i32, scale: Option<i32>, enc: Encoding) -> Result<Vec<u8>> {
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
        INTERVAL_YM => {
            let Value::IntervalYm(iv) = v else { return Err(bad()) };
            encode_interval_ym(iv, scale)?
        }
        INTERVAL_DT => {
            let Value::IntervalDt(iv) = v else { return Err(bad()) };
            encode_interval_dt(iv, scale)?
        }
        t => return Err(ProtoError::Unsupported(format!("parameter type {t}"))),
    })
}

fn interval_scale_for(scale: Option<i32>, lead: u8, frac: u8, q: IntervalQual, ym: bool) -> Result<i32> {
    match scale {
        Some(s) => {
            if s < 0 {
                return Err(ProtoError::Convert(format!("invalid interval scale {s}")));
            }
            let (_, _, qual) = unpack_interval_scale(s)?;
            if qual.is_ym() != ym {
                return Err(ProtoError::Convert(format!("interval scale {s:#x} qualifier {qual:?} does not match the target type")));
            }
            Ok(s)
        }
        None => {
            if q.is_ym() != ym {
                return Err(ProtoError::Convert(format!("interval qualifier {q:?} does not match the target type")));
            }
            pack_interval_scale(lead, frac, q)
        }
    }
}

fn encode_interval_ym(iv: &IntervalYm, scale: Option<i32>) -> Result<Vec<u8>> {
    let scale = interval_scale_for(scale, iv.lead_prec, iv.frac_prec, iv.qualifier, true)?;
    let mut o = Vec::with_capacity(12);
    o.extend_from_slice(&iv.years.to_le_bytes());
    o.extend_from_slice(&iv.months.to_le_bytes());
    o.extend_from_slice(&scale.to_le_bytes());
    Ok(o)
}

fn encode_interval_dt(iv: &IntervalDt, scale: Option<i32>) -> Result<Vec<u8>> {
    let scale = interval_scale_for(scale, iv.lead_prec, iv.frac_prec, iv.qualifier, false)?;
    let micros = iv.nanos / 1000;
    let mut o = Vec::with_capacity(24);
    o.extend_from_slice(&iv.days.to_le_bytes());
    o.extend_from_slice(&iv.hours.to_le_bytes());
    o.extend_from_slice(&iv.minutes.to_le_bytes());
    o.extend_from_slice(&iv.seconds.to_le_bytes());
    o.extend_from_slice(&micros.to_le_bytes());
    o.extend_from_slice(&scale.to_le_bytes());
    Ok(o)
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
    fn interval_wire_and_display() {
        let enc = Encoding::Utf8;
        let ctx_ym = ColumnCtx { col_type: INTERVAL_YM, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };
        let ctx_dt = ColumnCtx { col_type: INTERVAL_DT, new_lob: true, lob_tab_id: 0, lob_col_id: 0 };

        let ym = IntervalYm::new(1, 2);
        assert_eq!(ym.scale_word(), 406);
        assert_eq!(ym.to_string(), "INTERVAL '1-2' YEAR(9) TO MONTH");
        let b = encode_value(&ym.into(), INTERVAL_YM, enc).unwrap();
        assert_eq!(b, [1, 0, 0, 0, 2, 0, 0, 0, 0x96, 1, 0, 0]);
        assert_eq!(decode_value(&ctx_ym, Bytes::from(b), enc).unwrap(), Value::IntervalYm(ym));

        let neg_ym = IntervalYm { years: -100, months: -11, lead_prec: 3, frac_prec: 6, qualifier: IntervalQual::YearToMonth };
        assert_eq!(neg_ym.to_string(), "INTERVAL -'100-11' YEAR(3) TO MONTH");
        let b = encode_value(&neg_ym.into(), INTERVAL_YM, enc).unwrap();
        assert_eq!(b, [0x9c, 0xff, 0xff, 0xff, 0xf5, 0xff, 0xff, 0xff, 0x36, 1, 0, 0]);
        assert_eq!(decode_value(&ctx_ym, Bytes::from(b), enc).unwrap().as_interval_ym(), Some(&neg_ym));

        let year = IntervalYm { lead_prec: 2, ..IntervalYm::year(1) };
        assert_eq!(year.to_string(), "INTERVAL '1' YEAR(2)");
        assert_eq!(encode_value(&year.into(), INTERVAL_YM, enc).unwrap()[8..], [0x26, 0, 0, 0]);

        let dt = IntervalDt::new(3, 4, 5, 6, 789_000_000);
        assert_eq!(dt.to_string(), "INTERVAL '3 4:5:6.789000' DAY(9) TO SECOND(6)");
        assert_eq!(dt.scale_word(), 1686);
        let b = encode_value(&dt.into(), INTERVAL_DT, enc).unwrap();
        assert_eq!(
            b,
            vec![3, 0, 0, 0, 4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0, 0x08, 0x0a, 0x0c, 0x00, 0x96, 0x06, 0, 0]
        );
        assert_eq!(decode_value(&ctx_dt, Bytes::from(b), enc).unwrap(), Value::IntervalDt(dt));

        let neg_dt = IntervalDt { days: -3, hours: -4, minutes: -5, seconds: -6, nanos: -789_000_000, ..IntervalDt::new(0, 0, 0, 0, 0) };
        assert_eq!(neg_dt.to_string(), "INTERVAL -'3 4:5:6.789000' DAY(9) TO SECOND(6)");
        let b = encode_value(&neg_dt.into(), INTERVAL_DT, enc).unwrap();
        assert_eq!(&b[..20], &[-3i32, -4, -5, -6, -789_000].into_iter().flat_map(|n| n.to_le_bytes()).collect::<Vec<_>>());

        // 纳秒向零截断到微秒。
        let trunc = IntervalDt::new(0, 0, 0, 1, 1_500);
        let b = encode_value(&trunc.into(), INTERVAL_DT, enc).unwrap();
        let back = IntervalDt::try_from(decode_value(&ctx_dt, Bytes::from(b), enc).unwrap()).unwrap();
        assert_eq!(back.nanos, 1_000);

        // 绑定到列时用列的 scale 覆盖值自带的精度。
        let stamped = encode_value_with_scale(&Value::IntervalYm(IntervalYm::new(1, 2)), INTERVAL_YM, Some(294), enc).unwrap();
        assert_eq!(&stamped[8..12], &294i32.to_le_bytes());
        let back = IntervalYm::try_from(decode_value(&ctx_ym, Bytes::from(stamped), enc).unwrap()).unwrap();
        assert_eq!(back, IntervalYm { years: 1, months: 2, lead_prec: 2, frac_prec: 6, qualifier: IntervalQual::YearToMonth });

        let sec = IntervalDt { seconds: 6, nanos: 789_123_000, lead_prec: 2, frac_prec: 6, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) };
        assert_eq!(sec.to_string(), "INTERVAL '6.789123' SECOND(2, 6)");
        let sec0 = IntervalDt { seconds: 6, nanos: 0, lead_prec: 5, frac_prec: 0, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) };
        assert_eq!(sec0.to_string(), "INTERVAL '6' SECOND(5, 0)");
        let us = IntervalDt { nanos: -1_000, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) };
        assert_eq!(us.to_string(), "INTERVAL -'0.000001' SECOND(9, 6)");

        assert!(encode_value(&Value::IntervalYm(ym), INTERVAL_DT, enc).is_err());
        assert!(encode_value(&Value::Int(1), INTERVAL_YM, enc).is_err());
        assert!(decode_value(&ctx_ym, Bytes::from(vec![0; 11]), enc).is_err());
        assert!(IntervalDt::try_from(Value::IntervalYm(ym)).is_err());
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
