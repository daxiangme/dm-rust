//! 真机集成测试。需要 DM8 服务：`DM_TEST_URL=dm://SYSDBA:SYSDBA001@127.0.0.1:5236 cargo test --test integration`
//! 未设置 DM_TEST_URL 时默认连 127.0.0.1:5236；连不上则整体跳过。
use dm_rust::*;

fn opts() -> ConnectOptions {
    let url = std::env::var("DM_TEST_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    ConnectOptions::from_url(&url).unwrap()
}

async fn connect() -> Option<Connection> {
    match Connection::connect(&opts()).await {
        Ok(c) => Some(c),
        Err(Error::Io(e)) => {
            eprintln!("SKIP: cannot reach DM server: {e}");
            None
        }
        Err(e) => panic!("connect failed: {e}"),
    }
}

macro_rules! conn {
    () => {
        match connect().await {
            Some(c) => c,
            None => return,
        }
    };
}

async fn recreate(c: &mut Connection, table: &str, ddl: &str) {
    let _ = c.execute(&format!("drop table {table}"), &[]).await;
    c.execute(&format!("create table {table}({ddl})"), &[]).await.unwrap();
}

#[tokio::test]
async fn select_basic_and_unicode() {
    let mut c = conn!();
    assert!(c.server_version().unwrap().starts_with('8'));
    let r = c.query("select 1, 'héllo 世界', 2.5e0, cast('abc' as char(5)) from dual", &[]).await.unwrap();
    let v = &r.rows[0].values;
    assert_eq!(v[0], Value::Int(1));
    assert_eq!(v[1], Value::String("héllo 世界".into()));
    c.ping().await.unwrap();
}

#[tokio::test]
async fn types_roundtrip() {
    let mut c = conn!();
    recreate(&mut c, "T_RS_TYPES", "id int primary key, b1 bit, ti tinyint, si smallint, i int, bi bigint, r real, d double, dec1 decimal(20,5), c char(8), vc varchar(100), bin binary(4), vb varbinary(100), dt date, tm time, ts timestamp(6), tstz timestamp(6) with time zone, tmtz time with time zone").await;
    let dec: Decimal = "-12345.67891".parse().unwrap();
    let ts = DateTime { date: Date { year: 2026, month: 10, day: 27 }, time: Time { hour: 16, minute: 30, second: 59, nanos: 123_456_000, tz: None } };
    let tstz = DateTime { date: ts.date, time: Time { tz: Some(480), ..ts.time } };
    let tm = Time { hour: 1, minute: 2, second: 3, nanos: 0, tz: None };
    let tmtz = Time { tz: Some(-300), ..tm };
    let n = c
        .execute(
            "insert into T_RS_TYPES values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            &[
                1.into(), true.into(), (-5i8).into(), (-300i16).into(), 123456i32.into(), i64::MIN.into(), 1.5f32.into(), 2.25f64.into(), dec.clone().into(),
                "abc".into(), "变长字符串".into(), vec![1u8, 2, 3, 4].into(), vec![9u8; 50].into(), ts.date.into(), tm.into(), ts.into(), tstz.into(), tmtz.into(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(n.rows_affected, 1);
    // 全 NULL 行
    let mut nulls = vec![Value::Int(2)];
    nulls.extend(std::iter::repeat(Value::Null).take(17));
    c.execute("insert into T_RS_TYPES values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", &nulls).await.unwrap();
    let r = c.query("select * from T_RS_TYPES order by id", &[]).await.unwrap();
    assert_eq!(r.rows.len(), 2);
    let v = &r.rows[0].values;
    assert_eq!(v[1], Value::Bool(true));
    assert_eq!(v[2], Value::TinyInt(-5));
    assert_eq!(v[3], Value::SmallInt(-300));
    assert_eq!(v[4], Value::Int(123456));
    assert_eq!(v[5], Value::BigInt(i64::MIN));
    assert_eq!(v[6], Value::Float(1.5));
    assert_eq!(v[7], Value::Double(2.25));
    assert_eq!(v[8], Value::Decimal(dec));
    assert_eq!(v[9].as_str().unwrap().trim_end(), "abc");
    assert_eq!(v[10], Value::String("变长字符串".into()));
    assert_eq!(v[11].as_bytes().unwrap(), &[1, 2, 3, 4]);
    assert_eq!(v[12].as_bytes().unwrap(), &[9u8; 50][..]);
    assert_eq!(v[13], Value::Date(ts.date));
    assert_eq!(v[14], Value::Time(tm));
    assert_eq!(v[15], Value::DateTime(ts));
    assert_eq!(v[16], Value::DateTime(tstz));
    assert_eq!(v[17], Value::Time(tmtz));
    assert!(r.rows[1].values[1..].iter().all(|x| x.is_null()), "{:?}", r.rows[1].values);
    c.execute("drop table T_RS_TYPES", &[]).await.unwrap();
}

#[tokio::test]
async fn decimals() {
    let mut c = conn!();
    for s in ["0", "1", "-1", "100", "0.5", "-0.0001", "12345678901234567890.123456789", "9999999999999999999999999999.9999999999", "1e10"] {
        let d: Decimal = s.parse().unwrap();
        let r = c.query("select cast(? as decimal(38,10)) from dual", &[Value::Decimal(d.clone())]).await.unwrap();
        let Value::Decimal(back) = &r.rows[0].values[0] else { panic!("{:?}", r.rows[0].values) };
        assert_eq!(back.to_f64(), d.to_f64(), "{s}");
    }
    let r = c.query("select 123.456 from dual", &[]).await.unwrap();
    assert_eq!(r.rows[0].values[0].to_text().unwrap(), "123.456");
}

#[tokio::test]
async fn large_fetch() {
    let mut c = conn!();
    recreate(&mut c, "T_RS_BIG", "id int, s varchar(40)").await;
    let rows: Vec<Vec<Value>> = (0..5000).map(|i| vec![Value::Int(i), Value::String(format!("row-{i}-{}", "x".repeat((i % 30) as usize)))]).collect();
    let r = c.execute_batch("insert into T_RS_BIG values(?,?)", &rows).await.unwrap();
    assert_eq!(r.rows_affected, 5000);
    let q = c.query("select id, s from T_RS_BIG order by id", &[]).await.unwrap();
    assert_eq!(q.rows.len(), 5000);
    assert_eq!(q.rows[4999].values[0], Value::Int(4999));
    assert_eq!(q.rows[123].values[1].as_str().unwrap(), format!("row-123-{}", "x".repeat(123 % 30)));
    // 小 fetch_size
    let mut o = opts();
    o.fetch_size = 7;
    let mut c2 = Connection::connect(&o).await.unwrap();
    let q = c2.query("select id from T_RS_BIG order by id", &[]).await.unwrap();
    assert_eq!(q.rows.len(), 5000);
    for (i, r) in q.rows.iter().enumerate() {
        assert_eq!(r.values[0], Value::Int(i as i32));
    }
    c.execute("drop table T_RS_BIG", &[]).await.unwrap();
}

#[tokio::test]
async fn transactions() {
    let mut c = conn!();
    recreate(&mut c, "T_RS_TX", "id int primary key").await;
    c.begin().await.unwrap();
    c.execute("insert into T_RS_TX values(1)", &[]).await.unwrap();
    assert!(c.in_transaction());
    c.rollback().await.unwrap();
    assert_eq!(c.query("select count(*) from T_RS_TX", &[]).await.unwrap().rows[0].values[0].as_i64(), Some(0));
    c.begin().await.unwrap();
    c.execute("insert into T_RS_TX values(2)", &[]).await.unwrap();
    // 另一连接在提交前看不到
    let mut c2 = Connection::connect(&opts()).await.unwrap();
    assert_eq!(c2.query("select count(*) from T_RS_TX", &[]).await.unwrap().rows[0].values[0].as_i64(), Some(0));
    c.commit().await.unwrap();
    assert!(!c.in_transaction());
    assert_eq!(c2.query("select count(*) from T_RS_TX", &[]).await.unwrap().rows[0].values[0].as_i64(), Some(1));
    // 隔离级别
    c.set_isolation(IsolationLevel::Serializable).await.unwrap();
    c.set_isolation(IsolationLevel::ReadCommitted).await.unwrap();
    c.execute("drop table T_RS_TX", &[]).await.unwrap();
}

#[tokio::test]
async fn errors() {
    let mut c = conn!();
    let e = c.query("selec 1", &[]).await.unwrap_err();
    match e {
        Error::Server { code, message } => {
            assert!(code < 0);
            assert!(!message.is_empty());
            eprintln!("syntax error: {code} {message}");
        }
        e => panic!("{e:?}"),
    }
    // 连接仍可用
    c.ping().await.unwrap();
    recreate(&mut c, "T_RS_ERR", "id int primary key").await;
    c.execute("insert into T_RS_ERR values(1)", &[]).await.unwrap();
    let e = c.execute("insert into T_RS_ERR values(1)", &[]).await.unwrap_err();
    assert!(matches!(e, Error::Server { .. }), "{e:?}");
    c.ping().await.unwrap();
    c.execute("drop table T_RS_ERR", &[]).await.unwrap();
}

#[tokio::test]
async fn bad_login() {
    if connect().await.is_none() {
        return;
    }
    let mut o = opts();
    o.user = "NO_SUCH_USER_X".into();
    let e = Connection::connect(&o).await.err().unwrap();
    assert!(matches!(e, Error::Server { .. }), "{e:?}");
}

#[tokio::test]
async fn batch_with_error() {
    let mut c = conn!();
    recreate(&mut c, "T_RS_BATCH", "id int primary key").await;
    let rows: Vec<Vec<Value>> = vec![vec![1.into()], vec![2.into()], vec![2.into()], vec![3.into()]];
    let e = c.execute_batch("insert into T_RS_BATCH values(?)", &rows).await;
    eprintln!("batch error result: {e:?}");
    assert!(e.is_err());
    c.execute("drop table T_RS_BATCH", &[]).await.unwrap();
}

#[tokio::test]
async fn lobs() {
    let mut c = conn!();
    recreate(&mut c, "T_RS_LOB", "id int primary key, b blob, c clob").await;
    let small_b = vec![7u8; 100];
    let big_b: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let big_c: String = "你好 world ".repeat(20_000);
    c.execute("insert into T_RS_LOB values(?,?,?)", &[1.into(), small_b.clone().into(), "small clob".into()]).await.unwrap();
    c.execute("insert into T_RS_LOB values(?,?,?)", &[2.into(), big_b.clone().into(), big_c.clone().into()]).await.unwrap();
    c.execute("insert into T_RS_LOB values(?,?,?)", &[3.into(), Value::Null, Value::Null]).await.unwrap();
    let r = c.query("select id, b, c from T_RS_LOB order by id", &[]).await.unwrap();
    assert_eq!(r.rows[0].values[1].as_bytes().unwrap(), &small_b[..]);
    assert_eq!(r.rows[0].values[2].as_str().unwrap(), "small clob");
    assert_eq!(r.rows[1].values[1].as_bytes().unwrap().len(), big_b.len());
    assert!(r.rows[1].values[1].as_bytes().unwrap() == &big_b[..]);
    assert!(r.rows[1].values[2].as_str().unwrap() == big_c);
    assert!(r.rows[2].values[1].is_null());
    c.execute("drop table T_RS_LOB", &[]).await.unwrap();
}

#[tokio::test]
async fn compression_modes() {
    for (comp, algo) in [(Compression::Always, CompressAlgo::Zlib), (Compression::Always, CompressAlgo::Snappy), (Compression::Auto, CompressAlgo::Zlib)] {
        let mut o = opts();
        o.compression = comp;
        o.compress_algo = algo;
        let Ok(mut c) = Connection::connect(&o).await else { eprintln!("SKIP compression"); return };
        let r = c.query("select level, rpad('x', 100, 'y') from dual connect by level <= 2000", &[]).await.unwrap();
        assert_eq!(r.rows.len(), 2000, "{comp:?} {algo:?}");
    }
}

#[tokio::test]
async fn stmt_cache_and_reuse() {
    let mut c = conn!();
    for i in 0..200 {
        let r = c.query("select ? + 1 from dual", &[Value::Int(i)]).await.unwrap();
        assert_eq!(r.rows[0].values[0].as_i64(), Some(i as i64 + 1));
    }
    // 淘汰：超过缓存容量的不同语句
    let mut o = opts();
    o.stmt_cache_capacity = 3;
    let mut c = Connection::connect(&o).await.unwrap();
    for i in 0..20 {
        let r = c.query(&format!("select {i} from dual"), &[]).await.unwrap();
        assert_eq!(r.rows[0].values[0].as_i64(), Some(i));
    }
    let mut o = opts();
    o.stmt_cache_capacity = 0;
    let mut c = Connection::connect(&o).await.unwrap();
    for _ in 0..5 {
        c.query("select 1 from dual", &[]).await.unwrap();
    }
}

#[tokio::test]
async fn intervals() {
    let mut c = conn!();
    // 字面量的 Display 与服务端 cast 一致，同时核对解码出的分量。
    let literals: &[(&str, Value)] = &[
        ("INTERVAL '1-2' YEAR TO MONTH", IntervalYm::new(1, 2).into()),
        (
            "INTERVAL '-100-11' YEAR(3) TO MONTH",
            IntervalYm { years: -100, months: -11, lead_prec: 3, frac_prec: 6, qualifier: IntervalQual::YearToMonth }.into(),
        ),
        ("INTERVAL '1' YEAR(2)", IntervalYm { lead_prec: 2, ..IntervalYm::year(1) }.into()),
        ("INTERVAL '13' MONTH(4)", IntervalYm { months: 13, lead_prec: 4, ..IntervalYm::month(0) }.into()),
        ("INTERVAL '-0-1' YEAR TO MONTH", IntervalYm { months: -1, ..IntervalYm::new(0, 0) }.into()),
        ("INTERVAL '3 04:05:06.789' DAY TO SECOND", IntervalDt::new(3, 4, 5, 6, 789_000_000).into()),
        (
            "INTERVAL '-3 04:05:06.789' DAY TO SECOND",
            IntervalDt { days: -3, hours: -4, minutes: -5, seconds: -6, nanos: -789_000_000, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        ),
        ("INTERVAL '-0.000001' SECOND", IntervalDt { nanos: -1_000, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) }.into()),
        ("INTERVAL '3' DAY(2)", IntervalDt { days: 3, lead_prec: 2, qualifier: IntervalQual::Day, ..IntervalDt::new(0, 0, 0, 0, 0) }.into()),
        ("INTERVAL '4' HOUR(3)", IntervalDt { hours: 4, lead_prec: 3, qualifier: IntervalQual::Hour, ..IntervalDt::new(0, 0, 0, 0, 0) }.into()),
        ("INTERVAL '5' MINUTE(4)", IntervalDt { minutes: 5, lead_prec: 4, qualifier: IntervalQual::Minute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into()),
        (
            "INTERVAL '6.789' SECOND(1,3)",
            IntervalDt { seconds: 6, nanos: 789_000_000, lead_prec: 1, frac_prec: 3, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        ),
        ("INTERVAL '3 04' DAY(2) TO HOUR", IntervalDt { days: 3, hours: 4, lead_prec: 2, qualifier: IntervalQual::DayToHour, ..IntervalDt::new(0, 0, 0, 0, 0) }.into()),
        (
            "INTERVAL '3 04:05' DAY(3) TO MINUTE",
            IntervalDt { days: 3, hours: 4, minutes: 5, lead_prec: 3, qualifier: IntervalQual::DayToMinute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        ),
        (
            "INTERVAL '04:05' HOUR(2) TO MINUTE",
            IntervalDt { hours: 4, minutes: 5, lead_prec: 2, qualifier: IntervalQual::HourToMinute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        ),
        (
            "INTERVAL '04:05:06.7' HOUR(2) TO SECOND(1)",
            IntervalDt { hours: 4, minutes: 5, seconds: 6, nanos: 700_000_000, lead_prec: 2, frac_prec: 1, qualifier: IntervalQual::HourToSecond, ..IntervalDt::new(0, 0, 0, 0, 0) }
                .into(),
        ),
        (
            "INTERVAL '05:06.78' MINUTE(2) TO SECOND(2)",
            IntervalDt { minutes: 5, seconds: 6, nanos: 780_000_000, lead_prec: 2, frac_prec: 2, qualifier: IntervalQual::MinuteToSecond, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        ),
        ("INTERVAL '1 2:3:4' DAY TO SECOND(0)", IntervalDt { days: 1, hours: 2, minutes: 3, seconds: 4, frac_prec: 0, ..IntervalDt::new(0, 0, 0, 0, 0) }.into()),
    ];
    for (expr, expect) in literals {
        let r = c.query(&format!("select {expr}, cast({expr} as varchar(80)) from dual"), &[]).await.unwrap();
        assert_eq!(&r.rows[0].values[0], expect, "{expr}");
        assert_eq!(r.rows[0].values[0].to_text().as_deref(), r.rows[0].values[1].as_str(), "{expr}");
        assert_eq!(r.columns[0].col_type, if expect.as_interval_ym().is_some() { 20 } else { 21 }, "{expr}");
    }
    let r = c.query("select cast(null as interval year to month), cast(null as interval day to second) from dual", &[]).await.unwrap();
    assert!(r.rows[0].values.iter().all(|v| v.is_null()));

    // 无类型占位：精度随值走，读回无损。
    let bound = IntervalDt { seconds: 6, nanos: 789_123_000, lead_prec: 2, frac_prec: 6, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) };
    let r = c.query("select ? from dual", &[bound.into()]).await.unwrap();
    assert_eq!(r.rows[0].values[0], Value::IntervalDt(bound));
    assert_eq!(r.columns[0].col_type, 21);
    assert_eq!(r.columns[0].scale, bound.scale_word());
    let r = c.query("select ? from dual", &[IntervalYm::new(-1, -2).into()]).await.unwrap();
    assert_eq!(r.rows[0].values[0], Value::IntervalYm(IntervalYm::new(-1, -2)));

    // cast 的目标精度会落在结果上（参数描述是推荐类型，绑定用值自己的 scale）。
    let r = c.query("select cast(? as interval year(3) to month) from dual", &[IntervalYm::new(1, 2).into()]).await.unwrap();
    assert_eq!(
        r.rows[0].values[0],
        Value::IntervalYm(IntervalYm { years: 1, months: 2, lead_prec: 3, frac_prec: 6, qualifier: IntervalQual::YearToMonth })
    );

    recreate(
        &mut c,
        "T_RS_INTERVAL",
        "id int primary key, \
         ym interval year to month, \
         y interval year(3), \
         m interval month(4), \
         dt interval day to second, \
         d interval day(2), \
         h interval hour(3), \
         mi interval minute(4), \
         s interval second(5,2), \
         dh interval day(2) to hour, \
         dm interval day(3) to minute, \
         ds interval day(4) to second(3), \
         hm interval hour(2) to minute, \
         hs interval hour(3) to second(1), \
         ms interval minute(4) to second(5)",
    )
    .await;
    let row = vec![
        Value::Int(1),
        IntervalYm { years: 1, months: 2, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::YearToMonth }.into(),
        IntervalYm { years: -100, months: 0, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::Year }.into(),
        IntervalYm { years: 0, months: 15, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::Month }.into(),
        IntervalDt { days: -3, hours: -4, minutes: -5, seconds: -6, nanos: -789_123_000, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::DayToSecond }.into(),
        IntervalDt { days: 3, lead_prec: 9, qualifier: IntervalQual::Day, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { hours: 4, lead_prec: 9, qualifier: IntervalQual::Hour, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { minutes: 5, lead_prec: 9, qualifier: IntervalQual::Minute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { seconds: -6, nanos: -780_000_000, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { days: 3, hours: 4, lead_prec: 9, qualifier: IntervalQual::DayToHour, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { days: 3, hours: 4, minutes: 5, lead_prec: 9, qualifier: IntervalQual::DayToMinute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { days: 3, hours: 4, minutes: 5, seconds: 6, nanos: 789_000_000, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::DayToSecond }.into(),
        IntervalDt { hours: 4, minutes: 5, lead_prec: 9, qualifier: IntervalQual::HourToMinute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { hours: 4, minutes: 5, seconds: 6, nanos: 700_000_000, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::HourToSecond, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { minutes: 5, seconds: 6, nanos: 789_120_000, lead_prec: 9, frac_prec: 6, qualifier: IntervalQual::MinuteToSecond, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
    ];
    // 写入时值自带的前导精度是 9；列定义会把它收成列的精度。
    let expect = vec![
        Value::Int(1),
        IntervalYm { years: 1, months: 2, lead_prec: 2, frac_prec: 6, qualifier: IntervalQual::YearToMonth }.into(),
        IntervalYm { years: -100, months: 0, lead_prec: 3, frac_prec: 6, qualifier: IntervalQual::Year }.into(),
        IntervalYm { years: 0, months: 15, lead_prec: 4, frac_prec: 6, qualifier: IntervalQual::Month }.into(),
        IntervalDt { days: -3, hours: -4, minutes: -5, seconds: -6, nanos: -789_123_000, lead_prec: 2, frac_prec: 6, qualifier: IntervalQual::DayToSecond }.into(),
        IntervalDt { days: 3, lead_prec: 2, qualifier: IntervalQual::Day, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { hours: 4, lead_prec: 3, qualifier: IntervalQual::Hour, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { minutes: 5, lead_prec: 4, qualifier: IntervalQual::Minute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { seconds: -6, nanos: -780_000_000, lead_prec: 5, frac_prec: 2, qualifier: IntervalQual::Second, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { days: 3, hours: 4, lead_prec: 2, qualifier: IntervalQual::DayToHour, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { days: 3, hours: 4, minutes: 5, lead_prec: 3, qualifier: IntervalQual::DayToMinute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { days: 3, hours: 4, minutes: 5, seconds: 6, nanos: 789_000_000, lead_prec: 4, frac_prec: 3, qualifier: IntervalQual::DayToSecond }.into(),
        IntervalDt { hours: 4, minutes: 5, lead_prec: 2, qualifier: IntervalQual::HourToMinute, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { hours: 4, minutes: 5, seconds: 6, nanos: 700_000_000, lead_prec: 3, frac_prec: 1, qualifier: IntervalQual::HourToSecond, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
        IntervalDt { minutes: 5, seconds: 6, nanos: 789_120_000, lead_prec: 4, frac_prec: 5, qualifier: IntervalQual::MinuteToSecond, ..IntervalDt::new(0, 0, 0, 0, 0) }.into(),
    ];
    c.execute("insert into T_RS_INTERVAL values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", &row).await.unwrap();
    let mut nulls = vec![Value::Int(2)];
    nulls.extend(std::iter::repeat(Value::Null).take(14));
    c.execute("insert into T_RS_INTERVAL values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", &nulls).await.unwrap();
    let r = c.query("select * from T_RS_INTERVAL order by id", &[]).await.unwrap();
    assert_eq!(r.rows[0].values, expect);
    assert!(r.rows[1].values[1..].iter().all(|v| v.is_null()), "{:?}", r.rows[1].values);
    // 列描述的 scale 与值内 scale 字相同。
    let scales: Vec<i32> = expect.iter().skip(1).map(|v| if let Some(i) = v.as_interval_ym() { i.scale_word() } else { v.as_interval_dt().unwrap().scale_word() }).collect();
    assert_eq!(r.columns.iter().skip(1).map(|c| c.scale).collect::<Vec<_>>(), scales);

    // 前导精度放不下时服务端拒绝。
    let overflow = c
        .execute(
            "insert into T_RS_INTERVAL(id, y) values(?, ?)",
            &[Value::Int(3), IntervalYm { years: 1000, months: 0, lead_prec: 3, frac_prec: 6, qualifier: IntervalQual::Year }.into()],
        )
        .await;
    assert!(overflow.is_err(), "{overflow:?}");
    c.execute("drop table T_RS_INTERVAL", &[]).await.unwrap();
}

#[tokio::test]
async fn schema_option_and_meta() {
    let mut o = opts();
    o.schema = Some("SYSDBA".into());
    let Ok(mut c) = Connection::connect(&o).await else { return };
    let info = c.prepare("select ?, ? from dual").await.unwrap();
    assert_eq!(info.params.len(), 2);
    assert_eq!(info.columns.len(), 2);
}

async fn drop_call_objs(c: &mut Connection) {
    for sql in [
        "drop procedure P_RS_CALL_IO",
        "drop procedure P_RS_CALL_CUR",
        "drop function F_RS_CALL_ADD",
        "drop procedure P_RS_PROBE",
        "drop procedure P_RS_CUR",
        "drop procedure P_RS_CUR2",
        "drop function F_RS_ADD",
        "drop table T_RS_CURSRC",
    ] {
        let _ = c.execute(sql, &[]).await;
    }
}

fn assert_out(v: &OutValue, expect: &Value) {
    assert_eq!(v.as_value(), Some(expect), "{v:?}");
}

/// OUT / INOUT（数字、字符串、日期、NULL）、函数返回值、SYS_REFCURSOR。
#[tokio::test]
async fn call_out_inout_and_refcursor() {
    let mut c = conn!();
    drop_call_objs(&mut c).await;
    let created = async {
        c.execute(
            "create or replace procedure P_RS_CALL_IO(a in int, b out int, c in out int, s in varchar(100), so out varchar(200), si in out varchar(200), d in date, dout out date, di in out date, nout out int) as begin b := a + 10; c := c + a; if s is null then so := null; else so := s || '-out'; end if; if si is null then si := null; else si := si || '-io'; end if; dout := d + 1; di := di + 1; nout := null; end",
            &[],
        )
        .await?;
        c.execute(
            "create or replace function F_RS_CALL_ADD(a int, b int) return int as begin return a + b; end",
            &[],
        )
        .await?;
        c.execute(
            "create or replace procedure P_RS_CALL_CUR(n in int, c1 out sys_refcursor, c2 out sys_refcursor, c0 out sys_refcursor) as begin open c1 for select level as id, '行' || level as nm from dual connect by level <= n; open c2 for select level as id from dual connect by level <= 3; open c0 for select 1 as id from dual where 2 = 3; end",
            &[],
        )
        .await?;
        Ok::<(), Error>(())
    }
    .await;
    if let Err(e) = created {
        drop_call_objs(&mut c).await;
        panic!("ddl failed: {e}");
    }

    let io_sql = "begin P_RS_CALL_IO(?, ?, ?, ?, ?, ?, ?, ?, ?, ?); end;";
    let info = c.prepare(io_sql).await.unwrap();
    let meta: Vec<_> = info
        .params
        .iter()
        .map(|p| format!("{} ty={} prec={} scale={} io={} flag={} tname={}", p.name, p.col_type, p.prec, p.scale, p.io_type, p.item_flag, p.type_name))
        .collect();
    assert_eq!(info.params.iter().map(|p| p.io_type).collect::<Vec<_>>(), vec![0, 1, 2, 0, 1, 2, 0, 1, 2, 1], "{meta:?}");
    let kinds: Vec<_> = info.params.iter().map(|p| (p.col_type, p.prec, p.scale)).collect();
    assert_eq!(
        kinds,
        vec![(7, 4, 0), (7, 4, 0), (7, 4, 0), (2, 32767, 0), (2, 200, 0), (2, 200, 0), (14, 3, 0), (14, 3, 0), (14, 3, 0), (7, 4, 0)],
        "{meta:?}"
    );

    let d0 = Date { year: 2024, month: 2, day: 28 };
    let d1 = Date { year: 2024, month: 2, day: 29 };
    let params = |c_in: Value, s_in: Value, si: Value, di: Value| {
        vec![
            Param::In(Value::Int(3)),
            Param::out_int(),
            Param::InOut(c_in),
            Param::In(s_in),
            Param::Out(TypeHint::varchar(200)),
            Param::InOut(si),
            Param::In(d0.into()),
            Param::Out(TypeHint::date()),
            Param::InOut(di),
            Param::out_int(),
        ]
    };
    let check = |r: &CallResult, c_out: Value, so: Value, si: Value, di: Value| {
        assert_eq!(r.outs.len(), 7, "{r:?}");
        assert_out(&r.outs[0], &Value::Int(13));
        assert_out(&r.outs[1], &c_out);
        assert_out(&r.outs[2], &so);
        assert_out(&r.outs[3], &si);
        assert_out(&r.outs[4], &Value::Date(d1));
        assert_out(&r.outs[5], &di);
        assert_out(&r.outs[6], &Value::Null);
    };

    let r = c.call("{call P_RS_CALL_IO(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)}", &params(Value::Int(4), "你好".into(), "xy".into(), d0.into())).await.unwrap();
    check(&r, Value::Int(7), "你好-out".into(), "xy-io".into(), d1.into());

    let r = c.call("call P_RS_CALL_IO(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", &params(Value::Null, Value::Null, Value::Null, Value::Null)).await.unwrap();
    check(&r, Value::Null, Value::Null, Value::Null, Value::Null);

    let r = c.call(io_sql, &params(Value::Int(4), "你好".into(), "xy".into(), d0.into())).await.unwrap();
    check(&r, Value::Int(7), "你好-out".into(), "xy-io".into(), d1.into());

    let finfo = c.prepare("begin ? := F_RS_CALL_ADD(?, ?); end;").await.unwrap();
    let fmeta: Vec<_> = finfo.params.iter().map(|p| format!("ty={} prec={} io={} flag={} name={:?}", p.col_type, p.prec, p.io_type, p.item_flag, p.name)).collect();
    assert_eq!(finfo.params.iter().map(|p| (p.col_type, p.prec, p.io_type)).collect::<Vec<_>>(), vec![(7, 4, 1), (7, 4, 0), (7, 4, 0)], "{fmeta:?}");
    let r = c.call("{?=call F_RS_CALL_ADD(?, ?)}", &[Param::out_int(), Param::In(Value::Int(20)), Param::In(Value::Int(22))]).await.unwrap();
    assert_eq!(r.outs.len(), 1);
    assert_out(&r.outs[0], &Value::Int(42));
    let r = c.call("begin ? := F_RS_CALL_ADD(?, ?); end;", &[Param::out_int(), Param::In(Value::Null), Param::In(Value::Int(1))]).await.unwrap();
    assert_out(&r.outs[0], &Value::Null);
    let q = c.query("select F_RS_CALL_ADD(?, ?) from dual", &[Value::Int(2), Value::Int(3)]).await.unwrap();
    assert_eq!(q.rows[0].values[0], Value::Int(5));

    let cinfo = c.prepare("begin P_RS_CALL_CUR(?, ?, ?, ?); end;").await.unwrap();
    let cmeta: Vec<_> = cinfo.params.iter().map(|p| format!("{} ty={} prec={} scale={} io={} tname={}", p.name, p.col_type, p.prec, p.scale, p.io_type, p.type_name)).collect();
    assert_eq!(cinfo.params.iter().map(|p| p.io_type).collect::<Vec<_>>(), vec![0, 1, 1, 1], "{cmeta:?}");
    assert!(cinfo.params[1..].iter().all(|p| p.col_type == 120 && p.prec == 32768), "{cmeta:?}");
    let r = c
        .call(
            "begin P_RS_CALL_CUR(?, ?, ?, ?); end;",
            &[Param::In(Value::Int(120)), Param::out_cursor(), Param::out_cursor(), Param::out_cursor()],
        )
        .await
        .unwrap();
    assert_eq!(r.outs.len(), 3, "{cmeta:?} print={}", r.print_msg);
    let c1 = r.outs[0].as_cursor().expect("c1");
    assert_eq!(c1.columns.len(), 2, "{:?}", c1.columns);
    assert_eq!(c1.rows.len(), 120);
    for (i, row) in c1.rows.iter().enumerate() {
        assert_eq!(row.get_by_name("id"), Some(&Value::Int(i as i32 + 1)));
        assert_eq!(row.get_by_name("nm").and_then(|v| v.as_str()), Some(format!("行{}", i + 1)).as_deref());
    }
    let c2 = r.outs[1].as_cursor().expect("c2");
    assert_eq!(c2.rows.len(), 3);
    assert_eq!(c2.rows[2].get_by_name("id"), Some(&Value::Int(3)));
    let c0 = r.outs[2].as_cursor().expect("c0");
    assert!(c0.rows.is_empty(), "{c0:?}");
    assert_eq!(c0.columns.len(), 1, "{:?}", c0.columns);

    let anon = c.prepare("begin ? := ? + 5; end;").await.unwrap();
    let ameta: Vec<_> = anon.params.iter().map(|p| format!("ty={} prec={} io={} flag={}", p.col_type, p.prec, p.io_type, p.item_flag)).collect();
    assert_eq!(anon.params.iter().map(|p| (p.col_type, p.prec, p.io_type)).collect::<Vec<_>>(), vec![(7, 10, 1), (7, 10, 0)], "{ameta:?}");
    let r = c.call("begin ? := ? + 5; end;", &[Param::out_int(), Param::In(Value::Int(10))]).await.unwrap();
    assert_out(&r.outs[0], &Value::Int(15));
    let declared = c.prepare("declare v int; begin v := ?; ? := v + 5; end;").await.unwrap();
    let dmeta: Vec<_> = declared.params.iter().map(|p| format!("ty={} prec={} io={} flag={}", p.col_type, p.prec, p.io_type, p.item_flag)).collect();
    assert_eq!(declared.params.iter().map(|p| (p.col_type, p.prec, p.io_type)).collect::<Vec<_>>(), vec![(7, 4, 0), (7, 4, 1)], "{dmeta:?}");
    let r = c.call("declare v int; begin v := ?; ? := v + 5; end;", &[Param::In(Value::Int(10)), Param::out_int()]).await.unwrap();
    assert_out(&r.outs[0], &Value::Int(15));

    let guard = c.execute(io_sql, &[]).await.unwrap_err();
    assert!(guard.to_string().contains("Connection::call"), "{guard}");
    let guard = c.query("begin P_RS_CALL_CUR(?, ?, ?, ?); end;", &[Value::Int(1)]).await.unwrap_err();
    assert!(guard.to_string().contains("Connection::call"), "{guard}");
    let bad = vec![Param::In(Value::Int(1)); 10];
    let guard = c.call(io_sql, &bad).await.unwrap_err();
    assert!(guard.to_string().contains("direction"), "{guard}");
    c.query("select 1 from dual", &[]).await.unwrap();

    drop_call_objs(&mut c).await;
}

/// 多语句结果集：SELECT 批走 NEXT_RESULT；DML 不拆成多项；空结果集保留；宽行触发 FETCH。
#[tokio::test]
async fn multi_result_sets() {
    let mut c = conn!();
    let _ = c.execute("drop table T_RS_MULTI", &[]).await;
    c.execute("create table T_RS_MULTI(id int, s varchar(20))", &[]).await.unwrap();

    let one = c.query_multi("select 1 as a from dual", &[]).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one.results[0].as_query().unwrap().rows[0].get_by_name("a"), Some(&Value::Int(1)));

    let sql = "select 1 as a from dual; select 2 as b, '雪' as c from dual; select 3 as d from dual where 1 = 0";
    let q = c.query(sql, &[]).await.unwrap();
    assert_eq!(q.rows.len(), 1);
    assert_eq!(q.rows[0].get_by_name("a"), Some(&Value::Int(1)));
    assert_eq!(q.columns.len(), 1);

    let m = c.query_multi(sql, &[]).await.unwrap();
    assert_eq!(m.len(), 3, "{m:?}");
    assert_eq!(m.results[0].as_query().unwrap().rows[0].values[0], Value::Int(1));
    let b = m.results[1].as_query().unwrap();
    assert_eq!(b.rows[0].get_by_name("b"), Some(&Value::Int(2)));
    assert_eq!(b.rows[0].get_by_name("c"), Some(&Value::String("雪".into())));
    let empty = m.results[2].as_query().expect("empty select is still a result set");
    assert!(empty.rows.is_empty(), "{empty:?}");
    assert_eq!(empty.columns.len(), 1);
    assert!(m.results.iter().all(|r| r.as_update().is_none()));

    let m = c.query_multi("select ? as a from dual; select ? as b from dual", &[Value::Int(7), Value::Int(8)]).await.unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m.results[0].as_query().unwrap().rows[0].values[0], Value::Int(7));
    assert_eq!(m.results[1].as_query().unwrap().rows[0].values[0], Value::Int(8));

    // 第二结果集约 80×400 个汉字，首包放不下，剩下的靠 FETCH（结果集 id 0）补齐。
    let wide = "select 1 as a from dual; select level as id, rpad('汉', 400, '汉') as s from dual connect by level <= 80";
    let m = c.query_multi(wide, &[]).await.unwrap();
    assert_eq!(m.len(), 2, "{:?}", m.results.iter().map(|r| r.as_query().map(|q| q.rows.len())).collect::<Vec<_>>());
    let rows = &m.results[1].as_query().unwrap().rows;
    assert_eq!(rows.len(), 80);
    assert_eq!(rows[0].get_by_name("id"), Some(&Value::Int(1)));
    assert_eq!(rows[79].get_by_name("id"), Some(&Value::Int(80)));
    // 本实例 LENGTH_IN_CHAR=0：rpad('汉', 400, '汉') 的 LENGTH 是 200、LENGTHB 是 600，仍会把第二结果集拆成两包。
    let s = rows[40].get_by_name("s").and_then(|v| v.as_str()).unwrap();
    assert_eq!(s.chars().count(), 200);
    assert!(s.chars().all(|ch| ch == '汉'));

    let n = c.execute("insert into T_RS_MULTI values(1, 'a'); insert into T_RS_MULTI values(2, 'b'); update T_RS_MULTI set s = 'u'", &[]).await.unwrap();
    assert_eq!(n.rows_affected, 2, "last statement updates 2 rows, got {n:?}");
    let seen = c.query("select id, s from T_RS_MULTI order by id", &[]).await.unwrap();
    assert_eq!(seen.rows.len(), 2);
    assert!(seen.rows.iter().all(|r| r.get_by_name("s") == Some(&Value::String("u".into()))), "{:?}", seen.rows);

    let m = c.query_multi("insert into T_RS_MULTI values(3, 'c'); update T_RS_MULTI set s = 'z' where id >= 1", &[]).await.unwrap();
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m.results[0].as_update().unwrap().rows_affected, 3);

    let m = c.query_multi("select 9 as n from dual; update T_RS_MULTI set s = 'q' where id = 1; select id, s from T_RS_MULTI where id = 1", &[]).await.unwrap();
    assert_eq!(m.len(), 2, "{m:?}");
    assert_eq!(m.results[0].as_query().unwrap().rows[0].values[0], Value::Int(9));
    assert_eq!(m.results[1].as_query().unwrap().rows[0].get_by_name("s"), Some(&Value::String("q".into())));

    let m = c.query_multi("select 4 as a from dual; insert into T_RS_MULTI values(4, 'i')", &[]).await.unwrap();
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m.results[0].as_query().unwrap().rows[0].values[0], Value::Int(4));
    let chk = c.query("select s from T_RS_MULTI where id = 4", &[]).await.unwrap();
    assert_eq!(chk.rows[0].values[0], Value::String("i".into()));

    let m = c.query_multi("begin null; end;", &[]).await.unwrap();
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m.results[0].as_update().map(|e| e.rows_affected), Some(0));

    let batch = c.execute_batch("insert into T_RS_MULTI values(?, ?)", &[vec![10.into(), "b".into()], vec![11.into(), "b".into()]]).await.unwrap();
    assert_eq!(batch.rows_affected, 2);
    assert_eq!(batch.batch_counts, vec![1, 1]);

    let e = c.execute("declare c sys_refcursor; begin open c for select 1 as a from dual; dbms_sql.return_result(c); end;", &[]).await.unwrap_err();
    match e {
        Error::Server { code, message } => {
            assert_eq!(code, -2193, "{message}");
            assert!(message.contains("RETURN_RESULT"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    c.ping().await.unwrap();
    c.execute("drop table T_RS_MULTI", &[]).await.unwrap();
}
