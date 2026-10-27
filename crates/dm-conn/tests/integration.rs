//! 真机集成测试。需要 DM8 服务：`DM_TEST_URL=dm://SYSDBA:SYSDBA001@127.0.0.1:5236 cargo test -p dm-conn --test integration`
//! 未设置 DM_TEST_URL 时默认连 127.0.0.1:5236；连不上则整体跳过。
use dm_conn::*;

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
async fn schema_option_and_meta() {
    let mut o = opts();
    o.schema = Some("SYSDBA".into());
    let Ok(mut c) = Connection::connect(&o).await else { return };
    let info = c.prepare("select ?, ? from dual").await.unwrap();
    assert_eq!(info.params.len(), 2);
    assert_eq!(info.columns.len(), 2);
}
