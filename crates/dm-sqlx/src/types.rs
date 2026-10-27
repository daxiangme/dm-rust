//! Rust 类型与达梦类型的编码。
//!
//! 整数的 `Type::compatible` 接受 TINYINT / SMALLINT / INT / BIGINT，解码时再做范围检查。
//! `i32` 始终编码成 `Value::Int`，即使数值放得进更窄的类型。
//! 无符号整数按数值宽度放到能装下它的有符号类型里；`u64` 超过 `i64::MAX` 则编码失败。
//!
//! `bool` 对应 BIT / BOOLEAN。`f32` 对应 REAL，`f64` 对应 REAL 与 DOUBLE。
//! 字符串对应 CHAR / VARCHAR / VARCHAR2 / CLOB，`CHAR` 的补空格原样返回。
//! `Vec<u8>` 对应 BINARY / VARBINARY / BLOB。
//!
//! `chrono` 的朴素日期时间保留墙上时间。`DateTime<Utc>` 写入时区偏移 0，读回时把带偏移的墙上时间折成 UTC，没有偏移则当作 UTC。
//! `DateTime<FixedOffset>` 写入本地墙上时间和偏移分钟数；没有偏移的值按 UTC（偏移 0）读。
//! DATETIME 的线上小数只到微秒，亚微秒会丢掉。
//!
//! DECIMAL 使用 [`dm_conn::Decimal`]，不是 `rust_decimal`。

use crate::arguments::DmArgumentBuffer;
use crate::error::dec_err;
use crate::type_info::DmTypeInfo;
use crate::value::DmValueRef;
use crate::Dm;
use chrono::{
    DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc,
};
use dm_conn::{Date, DateTime as DmDateTime, Decimal, Time, Value};
use dm_proto::types::*;
use sqlx_core::decode::Decode;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::{BoxDynError, UnexpectedNullError};
use sqlx_core::types::Type;

fn is_integer(code: i32) -> bool {
    matches!(code, TINYINT | SMALLINT | INT | BIGINT)
}

fn is_text(code: i32) -> bool {
    matches!(code, CHAR | VARCHAR | VARCHAR2 | CLOB)
}

fn is_binary(code: i32) -> bool {
    matches!(code, BINARY | VARBINARY | BLOB)
}

fn is_datetime(code: i32) -> bool {
    matches!(code, DATETIME | DATETIME_TZ | DATETIME2 | DATETIME2_TZ)
}

fn expect<'r, T>(
    value: DmValueRef<'r>,
    f: impl FnOnce(&'r Value) -> Result<T, BoxDynError>,
) -> Result<T, BoxDynError> {
    if value.value.is_null() {
        Err(UnexpectedNullError.into())
    } else {
        f(value.value)
    }
}

fn as_i64(value: &Value) -> Result<i64, BoxDynError> {
    match value {
        Value::TinyInt(n) => Ok(*n as i64),
        Value::SmallInt(n) => Ok(*n as i64),
        Value::Int(n) => Ok(*n as i64),
        Value::BigInt(n) => Ok(*n),
        other => Err(dec_err(format!("expected an integer, got {other:?}"))),
    }
}

fn put(buf: &mut DmArgumentBuffer, value: Value) -> Result<IsNull, BoxDynError> {
    buf.value = Some(value);
    Ok(IsNull::No)
}

macro_rules! int_type {
    ($rust:ty, $code:expr, $variant:ident) => {
        impl Type<Dm> for $rust {
            fn type_info() -> DmTypeInfo {
                DmTypeInfo::new($code)
            }
            fn compatible(ty: &DmTypeInfo) -> bool {
                is_integer(ty.type_code())
            }
        }

        impl Encode<'_, Dm> for $rust {
            fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
                put(buf, Value::$variant(*self))
            }
        }

        impl<'r> Decode<'r, Dm> for $rust {
            fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
                expect(value, |v| {
                    as_i64(v)?.try_into().map_err(|e| dec_err(format!("{e}")))
                })
            }
        }
    };
}

int_type!(i8, TINYINT, TinyInt);
int_type!(i16, SMALLINT, SmallInt);
int_type!(i32, INT, Int);
int_type!(i64, BIGINT, BigInt);

impl Type<Dm> for u8 {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(TINYINT)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_integer(ty.type_code())
    }
}
impl Encode<'_, Dm> for u8 {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        let value = if *self <= i8::MAX as u8 {
            Value::TinyInt(*self as i8)
        } else {
            Value::SmallInt(*self as i16)
        };
        put(buf, value)
    }
}
impl<'r> Decode<'r, Dm> for u8 {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| {
            as_i64(v)?
                .try_into()
                .map_err(|e: std::num::TryFromIntError| dec_err(e.to_string()))
        })
    }
}

impl Type<Dm> for u16 {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(SMALLINT)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_integer(ty.type_code())
    }
}
impl Encode<'_, Dm> for u16 {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        let value = if *self <= i16::MAX as u16 {
            Value::SmallInt(*self as i16)
        } else {
            Value::Int(*self as i32)
        };
        put(buf, value)
    }
}
impl<'r> Decode<'r, Dm> for u16 {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| {
            as_i64(v)?
                .try_into()
                .map_err(|e: std::num::TryFromIntError| dec_err(e.to_string()))
        })
    }
}

impl Type<Dm> for u32 {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(INT)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_integer(ty.type_code())
    }
}
impl Encode<'_, Dm> for u32 {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        let value = if *self <= i32::MAX as u32 {
            Value::Int(*self as i32)
        } else {
            Value::BigInt(*self as i64)
        };
        put(buf, value)
    }
}
impl<'r> Decode<'r, Dm> for u32 {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| {
            as_i64(v)?
                .try_into()
                .map_err(|e: std::num::TryFromIntError| dec_err(e.to_string()))
        })
    }
}

impl Type<Dm> for u64 {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(BIGINT)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_integer(ty.type_code())
    }
}
impl Encode<'_, Dm> for u64 {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        let n = i64::try_from(*self).map_err(|_| dec_err("u64 value does not fit in DM BIGINT"))?;
        put(buf, Value::BigInt(n))
    }
}
impl<'r> Decode<'r, Dm> for u64 {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| {
            as_i64(v)?
                .try_into()
                .map_err(|e: std::num::TryFromIntError| dec_err(e.to_string()))
        })
    }
}

impl Type<Dm> for bool {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(BIT)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        matches!(ty.type_code(), BIT | BOOLEAN)
    }
}
impl Encode<'_, Dm> for bool {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Bool(*self))
    }
}
impl<'r> Decode<'r, Dm> for bool {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::Bool(b) => Ok(*b),
            other => Err(dec_err(format!("expected bool, got {other:?}"))),
        })
    }
}

impl Type<Dm> for f32 {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(REAL)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        ty.type_code() == REAL
    }
}
impl Encode<'_, Dm> for f32 {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Float(*self))
    }
}
impl<'r> Decode<'r, Dm> for f32 {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::Float(n) => Ok(*n),
            other => Err(dec_err(format!("expected REAL, got {other:?}"))),
        })
    }
}

impl Type<Dm> for f64 {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(DOUBLE)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        matches!(ty.type_code(), REAL | DOUBLE)
    }
}
impl Encode<'_, Dm> for f64 {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Double(*self))
    }
}
impl<'r> Decode<'r, Dm> for f64 {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::Float(n) => Ok(*n as f64),
            Value::Double(n) => Ok(*n),
            other => Err(dec_err(format!("expected floating point, got {other:?}"))),
        })
    }
}

impl Type<Dm> for str {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(VARCHAR)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_text(ty.type_code())
    }
}
impl Encode<'_, Dm> for str {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::String((*self).to_owned()))
    }
}

// sqlx 给 `&T` 的 Encode 要求 `T: Sized`，所以 `&str` 要单独实现。
impl Encode<'_, Dm> for &'_ str {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <str as Encode<Dm>>::encode_by_ref(self, buf)
    }
}

impl Type<Dm> for String {
    fn type_info() -> DmTypeInfo {
        <str as Type<Dm>>::type_info()
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        <str as Type<Dm>>::compatible(ty)
    }
}
impl Encode<'_, Dm> for String {
    fn encode(self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::String(self))
    }
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <str as Encode<Dm>>::encode_by_ref(self, buf)
    }
}
impl<'r> Decode<'r, Dm> for String {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::String(s) => Ok(s.clone()),
            other => Err(dec_err(format!("expected a string, got {other:?}"))),
        })
    }
}

impl Type<Dm> for [u8] {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(VARBINARY)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_binary(ty.type_code())
    }
}
impl Encode<'_, Dm> for [u8] {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::from(self))
    }
}

impl Encode<'_, Dm> for &'_ [u8] {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <[u8] as Encode<Dm>>::encode_by_ref(self, buf)
    }
}

impl Type<Dm> for Vec<u8> {
    fn type_info() -> DmTypeInfo {
        <[u8] as Type<Dm>>::type_info()
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        <[u8] as Type<Dm>>::compatible(ty)
    }
}
impl Encode<'_, Dm> for Vec<u8> {
    fn encode(self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::from(self))
    }
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <[u8] as Encode<Dm>>::encode_by_ref(self, buf)
    }
}
impl<'r> Decode<'r, Dm> for Vec<u8> {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::Bytes(b) => Ok(b.to_vec()),
            other => Err(dec_err(format!("expected bytes, got {other:?}"))),
        })
    }
}

impl Type<Dm> for Decimal {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(DECIMAL)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        ty.type_code() == DECIMAL
    }
}
impl Encode<'_, Dm> for Decimal {
    fn encode(self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Decimal(self))
    }
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Decimal(self.clone()))
    }
}
impl<'r> Decode<'r, Dm> for Decimal {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::Decimal(d) => Ok(d.clone()),
            other => Err(dec_err(format!("expected DECIMAL, got {other:?}"))),
        })
    }
}

fn date_of(d: NaiveDate) -> Date {
    Date {
        year: d.year(),
        month: d.month() as u8,
        day: d.day() as u8,
    }
}

fn naive_date(d: &Date) -> Result<NaiveDate, BoxDynError> {
    NaiveDate::from_ymd_opt(d.year, d.month as u32, d.day as u32)
        .ok_or_else(|| dec_err(format!("invalid date {d}")))
}

fn time_of(t: NaiveTime, tz: Option<i16>) -> Time {
    Time {
        hour: t.hour() as u8,
        minute: t.minute() as u8,
        second: t.second() as u8,
        nanos: t.nanosecond(),
        tz,
    }
}

fn naive_time(t: &Time) -> Result<NaiveTime, BoxDynError> {
    NaiveTime::from_hms_nano_opt(t.hour as u32, t.minute as u32, t.second as u32, t.nanos)
        .ok_or_else(|| dec_err(format!("invalid time {t}")))
}

fn naive_datetime(dt: &DmDateTime) -> Result<NaiveDateTime, BoxDynError> {
    Ok(NaiveDateTime::new(
        naive_date(&dt.date)?,
        naive_time(&dt.time)?,
    ))
}

fn dm_datetime(date: NaiveDate, time: NaiveTime, tz: Option<i16>) -> DmDateTime {
    DmDateTime {
        date: date_of(date),
        time: time_of(time, tz),
    }
}

fn fixed_offset(mins: i16) -> Result<FixedOffset, BoxDynError> {
    let secs = (mins as i32)
        .checked_mul(60)
        .ok_or_else(|| dec_err("time zone offset overflow"))?;
    FixedOffset::east_opt(secs)
        .ok_or_else(|| dec_err(format!("time zone offset {mins} minutes is out of range")))
}

impl Type<Dm> for NaiveDate {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(DATE)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        ty.type_code() == DATE
    }
}
impl Encode<'_, Dm> for NaiveDate {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Date(date_of(*self)))
    }
}
impl<'r> Decode<'r, Dm> for NaiveDate {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::Date(d) => naive_date(d),
            other => Err(dec_err(format!("expected DATE, got {other:?}"))),
        })
    }
}

impl Type<Dm> for NaiveTime {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(TIME)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        matches!(ty.type_code(), TIME | TIME_TZ)
    }
}
impl Encode<'_, Dm> for NaiveTime {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(buf, Value::Time(time_of(*self, None)))
    }
}
impl<'r> Decode<'r, Dm> for NaiveTime {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        // 带时区的 TIME 只取墙上时间，偏移不进入 NaiveTime。
        expect(value, |v| match v {
            Value::Time(t) => naive_time(t),
            other => Err(dec_err(format!("expected TIME, got {other:?}"))),
        })
    }
}

impl Type<Dm> for NaiveDateTime {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(DATETIME)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_datetime(ty.type_code())
    }
}
impl Encode<'_, Dm> for NaiveDateTime {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        put(
            buf,
            Value::DateTime(dm_datetime(self.date(), self.time(), None)),
        )
    }
}
impl<'r> Decode<'r, Dm> for NaiveDateTime {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::DateTime(dt) => naive_datetime(dt),
            other => Err(dec_err(format!("expected DATETIME, got {other:?}"))),
        })
    }
}

impl Type<Dm> for DateTime<Utc> {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(DATETIME_TZ)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_datetime(ty.type_code())
    }
}
impl Encode<'_, Dm> for DateTime<Utc> {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        let naive = self.naive_utc();
        put(
            buf,
            Value::DateTime(dm_datetime(naive.date(), naive.time(), Some(0))),
        )
    }
}
impl<'r> Decode<'r, Dm> for DateTime<Utc> {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::DateTime(dt) => {
                let naive = naive_datetime(dt)?;
                match dt.time.tz {
                    None => Ok(Utc.from_utc_datetime(&naive)),
                    Some(mins) => {
                        let local = fixed_offset(mins)?
                            .from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| dec_err(format!("invalid local datetime {dt}")))?;
                        Ok(local.with_timezone(&Utc))
                    }
                }
            }
            other => Err(dec_err(format!("expected DATETIME, got {other:?}"))),
        })
    }
}

impl Type<Dm> for DateTime<FixedOffset> {
    fn type_info() -> DmTypeInfo {
        DmTypeInfo::new(DATETIME_TZ)
    }
    fn compatible(ty: &DmTypeInfo) -> bool {
        is_datetime(ty.type_code())
    }
}
impl Encode<'_, Dm> for DateTime<FixedOffset> {
    fn encode_by_ref(&self, buf: &mut DmArgumentBuffer) -> Result<IsNull, BoxDynError> {
        let local = self.naive_local();
        let mins = i16::try_from(self.offset().local_minus_utc() / 60)
            .map_err(|_| dec_err("time zone offset does not fit in i16 minutes"))?;
        put(
            buf,
            Value::DateTime(dm_datetime(local.date(), local.time(), Some(mins))),
        )
    }
}
impl<'r> Decode<'r, Dm> for DateTime<FixedOffset> {
    fn decode(value: DmValueRef<'r>) -> Result<Self, BoxDynError> {
        expect(value, |v| match v {
            Value::DateTime(dt) => {
                let naive = naive_datetime(dt)?;
                let mins = dt.time.tz.unwrap_or(0);
                let off = fixed_offset(mins)?;
                if dt.time.tz.is_some() {
                    off.from_local_datetime(&naive)
                        .single()
                        .ok_or_else(|| dec_err(format!("invalid local datetime {dt}")))
                } else {
                    Ok(off.from_utc_datetime(&naive))
                }
            }
            other => Err(dec_err(format!("expected DATETIME, got {other:?}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_info::DmTypeInfo;
    use sqlx_core::decode::Decode;
    use sqlx_core::types::Type;

    fn vref(value: &Value, code: i32) -> DmValueRef<'_> {
        DmValueRef {
            value,
            type_info: DmTypeInfo::new(code),
        }
    }

    #[test]
    fn integers_are_compatible_and_range_checked() {
        assert!(<i64 as Type<Dm>>::compatible(&DmTypeInfo::new(INT)));
        assert!(<i32 as Type<Dm>>::compatible(&DmTypeInfo::new(BIGINT)));
        assert!(!<i32 as Type<Dm>>::compatible(&DmTypeInfo::new(VARCHAR)));
        // sqlx 的整数解码是 `try_into().map_err(Into::into)`，错误就是
        // `TryFromIntError`。rustc 1.99 的文案是 "number too large/small to fit
        // in target type"，更早的版本才写 "out of range"。
        let big = Value::BigInt(1000);
        let err = <i8 as Decode<Dm>>::decode(vref(&big, BIGINT)).unwrap_err();
        let expected = i8::try_from(1000_i64).unwrap_err();
        assert_eq!(err.to_string(), expected.to_string());
        let n = Value::Null;
        assert!(<i32 as Decode<Dm>>::decode(vref(&n, INT)).is_err());
        assert_eq!(
            <Option<i32> as Decode<Dm>>::decode(vref(&n, VARCHAR)).unwrap(),
            None
        );
    }

    #[test]
    fn unsigned_widens_and_u64_max_fails() {
        let mut buf = DmArgumentBuffer::default();
        let _ = Encode::<Dm>::encode(200_u8, &mut buf).unwrap();
        assert_eq!(buf.value, Some(Value::SmallInt(200)));
        let _ = Encode::<Dm>::encode(40_000_u16, &mut buf).unwrap();
        assert_eq!(buf.value, Some(Value::Int(40_000)));
        let _ = Encode::<Dm>::encode(3_000_000_000_u32, &mut buf).unwrap();
        assert_eq!(buf.value, Some(Value::BigInt(3_000_000_000)));
        assert!(Encode::<Dm>::encode_by_ref(&u64::MAX, &mut buf).is_err());
    }

    #[test]
    fn chrono_utc_keeps_offset_zero() {
        let naive = NaiveDate::from_ymd_opt(2026, 10, 27)
            .unwrap()
            .and_hms_micro_opt(16, 30, 59, 123_456)
            .unwrap();
        let utc = Utc.from_utc_datetime(&naive);
        let mut buf = DmArgumentBuffer::default();
        let _ = Encode::<Dm>::encode(utc, &mut buf).unwrap();
        let Value::DateTime(dt) = buf.value.unwrap() else {
            panic!("not datetime")
        };
        assert_eq!(dt.time.tz, Some(0));
        assert_eq!(dt.date.year, 2026);
        assert_eq!(dt.time.nanos, 123_456_000);
        let back =
            <DateTime<Utc> as Decode<Dm>>::decode(vref(&Value::DateTime(dt), DATETIME_TZ)).unwrap();
        assert_eq!(back, utc);
    }
}
