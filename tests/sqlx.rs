//! 打开 `sqlx` feature 时连本机 DM8。连不上则跳过。
//! `cargo test --features sqlx --test sqlx`
#![cfg(feature = "sqlx")]

use std::sync::atomic::{AtomicU64, Ordering};

use dm_rust::sqlx::driver_test::{
    query, DatabaseError, DateTime, ErrorKind, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
    TryStreamExt, Utc,
};
use dm_rust::sqlx::{
    Acquire, Connection, Decimal, DmConnectOptions, DmConnection, DmPoolOptions, DmQueryResult,
    Error, Executor, Row,
};

fn test_url() -> String {
    std::env::var("DM_TEST_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into())
}

async fn connect() -> Option<DmConnection> {
    match DmConnection::connect(&test_url()).await {
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

fn unique(suffix: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("T_SQLX_{}_{}_{}", std::process::id(), n, suffix)
}

async fn exec(c: &mut DmConnection, sql: &str) -> Result<DmQueryResult, Error> {
    query(sql).execute(&mut *c).await
}

async fn drop_table(c: &mut DmConnection, table: &str) {
    let _ = exec(c, &format!("drop table {table}")).await;
}

async fn count(c: &mut DmConnection, table: &str) -> i64 {
    let row = query(&format!("select count(*) from {table}"))
        .fetch_one(&mut *c)
        .await
        .unwrap();
    row.get(0)
}

#[tokio::test]
async fn connect_and_bind() {
    let mut c = conn!();
    let row = query("select ? + 1 from dual")
        .bind(1)
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>(0), 2);
    let row = query("select ? from dual")
        .bind(None::<i32>)
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(row.get::<Option<i32>, _>(0), None);
    c.close().await.unwrap();
}

#[tokio::test]
async fn roundtrip_types() {
    let mut c = conn!();
    let table = unique("RT");
    drop_table(&mut c, &table).await;
    exec(
        &mut c,
        &format!(
            "create table {table}(\
                id int primary key, b bit, ti tinyint, si smallint, i int, bi bigint, \
                u8c smallint, u16c int, u32c bigint, u64c bigint, \
                r real, d double, dec1 decimal(20,5), vc varchar(100), vb varbinary(100), bl blob, \
                dt date, tm time, ts timestamp(6), tstz timestamp(6) with time zone, \
                ni int, ns varchar(20))"
        ),
    )
    .await
    .unwrap();

    let dec: Decimal = "12345.67891".parse().unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
    let time = NaiveTime::from_hms_micro_opt(1, 2, 3, 0).unwrap();
    let ts = date.and_hms_micro_opt(16, 30, 59, 123_456).unwrap();
    let utc: DateTime<Utc> = Utc.from_utc_datetime(&ts);
    let blob: Vec<u8> = (0..3000).map(|i| (i % 251) as u8).collect();
    let bytes = vec![9u8, 8, 7, 6];

    let inserted = query(&format!(
        "insert into {table} values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
    ))
    .bind(1_i32)
    .bind(true)
    .bind(-5_i8)
    .bind(-300_i16)
    .bind(123_456_i32)
    .bind(i64::MIN)
    .bind(200_u8)
    .bind(40_000_u16)
    .bind(3_000_000_000_u32)
    .bind(i64::MAX as u64)
    .bind(1.5_f32)
    .bind(2.25_f64)
    .bind(dec.clone())
    .bind("变长字符串")
    .bind(bytes.clone())
    .bind(blob.clone())
    .bind(date)
    .bind(time)
    .bind(ts)
    .bind(utc)
    .bind(None::<i32>)
    .bind(None::<String>)
    .execute(&mut c)
    .await
    .unwrap();
    assert_eq!(inserted.rows_affected(), 1);

    query(&format!("insert into {table}(id, ni, ns) values(?,?,?)"))
        .bind(2_i32)
        .bind(Some(7_i32))
        .bind(Some("ok".to_string()))
        .execute(&mut c)
        .await
        .unwrap();

    let row = query(&format!("select * from {table} where id = 1"))
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(row.get::<bool, _>("b"), true);
    assert_eq!(row.get::<i8, _>("ti"), -5);
    assert_eq!(row.get::<i16, _>("si"), -300);
    assert_eq!(row.get::<i32, _>("i"), 123_456);
    assert_eq!(row.get::<i64, _>("bi"), i64::MIN);
    assert_eq!(row.get::<u8, _>("u8c"), 200);
    assert_eq!(row.get::<u16, _>("u16c"), 40_000);
    assert_eq!(row.get::<u32, _>("u32c"), 3_000_000_000);
    assert_eq!(row.get::<u64, _>("u64c"), i64::MAX as u64);
    assert_eq!(row.get::<f32, _>("r"), 1.5);
    assert_eq!(row.get::<f64, _>("d"), 2.25);
    assert_eq!(row.get::<Decimal, _>("dec1").to_string(), dec.to_string());
    assert_eq!(row.get::<String, _>("vc"), "变长字符串");
    assert_eq!(row.get::<Vec<u8>, _>("vb"), bytes);
    assert_eq!(row.get::<Vec<u8>, _>("bl"), blob);
    assert_eq!(row.get::<NaiveDate, _>("dt"), date);
    assert_eq!(row.get::<NaiveTime, _>("tm"), time);
    assert_eq!(row.get::<NaiveDateTime, _>("ts"), ts);
    assert_eq!(row.get::<DateTime<Utc>, _>("tstz"), utc);
    assert_eq!(row.get::<Option<i32>, _>("ni"), None);
    assert_eq!(row.get::<Option<String>, _>("ns"), None);

    let row = query(&format!("select ni, ns from {table} where id = 2"))
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(row.get::<Option<i32>, _>(0), Some(7));
    assert_eq!(row.get::<Option<String>, _>(1), Some("ok".into()));

    let mut ids = Vec::new();
    let sql = format!("select id from {table} order by id");
    let mut stream = query(&sql).fetch(&mut c);
    while let Some(row) = stream.try_next().await.unwrap() {
        ids.push(row.get::<i32, _>(0));
    }
    drop(stream);
    assert_eq!(ids, vec![1, 2]);

    drop_table(&mut c, &table).await;
    c.close().await.unwrap();
}

#[tokio::test]
async fn transactions_commit_rollback_and_savepoint() {
    let mut c = conn!();
    let mut other = match connect().await {
        Some(c) => c,
        None => return,
    };
    let table = unique("TX");
    drop_table(&mut c, &table).await;
    exec(&mut c, &format!("create table {table}(id int primary key)"))
        .await
        .unwrap();

    let mut tx = Connection::begin(&mut c).await.unwrap();
    query(&format!("insert into {table} values(1)"))
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count(&mut other, &table).await, 0);
    tx.commit().await.unwrap();
    assert_eq!(count(&mut c, &table).await, 1);
    assert_eq!(count(&mut other, &table).await, 1);

    let mut tx = Connection::begin(&mut c).await.unwrap();
    query(&format!("insert into {table} values(2)"))
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(count(&mut other, &table).await, 1);

    {
        let mut tx = Connection::begin(&mut c).await.unwrap();
        query(&format!("insert into {table} values(3)"))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    assert_eq!(count(&mut c, &table).await, 1);

    let mut tx = Connection::begin(&mut c).await.unwrap();
    query(&format!("insert into {table} values(4)"))
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut sp = Acquire::begin(&mut tx).await.unwrap();
    query(&format!("insert into {table} values(5)"))
        .execute(&mut *sp)
        .await
        .unwrap();
    sp.rollback().await.unwrap();
    assert_eq!(count(&mut tx, &table).await, 2);
    tx.commit().await.unwrap();
    assert_eq!(count(&mut other, &table).await, 2);

    let err = c.begin_with("BEGIN").await.unwrap_err();
    assert!(matches!(err, Error::Protocol(_)), "{err}");
    assert_eq!(count(&mut c, &table).await, 2);

    drop_table(&mut c, &table).await;
    c.close().await.unwrap();
    other.close().await.unwrap();
}

#[tokio::test]
async fn identity_and_constraints() {
    let mut c = conn!();
    let table = unique("ID");
    drop_table(&mut c, &table).await;
    // 直接给 IDENTITY 列赋值会得到 -2723（须 SET IDENTITY_INSERT），不是唯一约束。
    // 唯一约束用普通 UNIQUE 列，非空约束用省略 NOT NULL 列。
    exec(
        &mut c,
        &format!(
            "create table {table}(id int identity(1,1) primary key, s varchar(20) not null, n int, u int unique)"
        ),
    )
    .await
    .unwrap();
    let done = query(&format!("insert into {table}(s, u) values(?, ?)"))
        .bind("a")
        .bind(1_i32)
        .execute(&mut c)
        .await
        .unwrap();
    assert_eq!(done.rows_affected(), 1);
    assert_eq!(done.last_insert_id(), Some(1));

    let dup = query(&format!("insert into {table}(s, u) values('b', 1)"))
        .execute(&mut c)
        .await
        .unwrap_err();
    let db = dup.as_database_error().unwrap();
    assert_eq!(DatabaseError::kind(db), ErrorKind::UniqueViolation, "{dup}");
    assert!(DatabaseError::is_unique_violation(db));

    let null = query(&format!("insert into {table}(n) values(2)"))
        .execute(&mut c)
        .await
        .unwrap_err();
    assert_eq!(
        DatabaseError::kind(null.as_database_error().unwrap()),
        ErrorKind::NotNullViolation
    );

    // sqlx 的 Describe::nullable 指结果列。INSERT 没有结果列，所以这里描述 SELECT。
    let described = Executor::describe(&mut c, &format!("select id, s, n from {table}"))
        .await
        .unwrap();
    assert_eq!(described.columns().len(), 3);
    assert_eq!(described.nullable(0), Some(false));
    assert_eq!(described.nullable(1), Some(false));
    assert_eq!(described.nullable(2), Some(true));

    drop_table(&mut c, &table).await;
    c.close().await.unwrap();
}

#[tokio::test]
async fn pool_acquire() {
    let opts: DmConnectOptions = match test_url().parse() {
        Ok(o) => o,
        Err(e) => panic!("bad url: {e}"),
    };
    let pool = match DmPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
    {
        Ok(p) => p,
        Err(Error::Io(e)) => {
            eprintln!("SKIP: cannot reach DM server: {e}");
            return;
        }
        Err(e) => panic!("pool connect failed: {e}"),
    };
    let mut a = pool.acquire().await.unwrap();
    let mut b = pool.acquire().await.unwrap();
    let row = query("select 1 from dual")
        .fetch_one(&mut *a)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>(0), 1);
    let row = query("select ? + 1 from dual")
        .bind(4)
        .fetch_one(&mut *b)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>(0), 5);
    drop(a);
    drop(b);
    pool.close().await;
}
