//! ShellAX 对象树元数据 SQL 的真机测试（见 docs/metadata-queries.md）。连不上 127.0.0.1:5236 时跳过。
use dm_rust::{ConnectOptions, Connection, Value};

async fn connect() -> Option<Connection> {
    let url = std::env::var("DM_TEST_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    match Connection::connect(&ConnectOptions::from_url(&url).ok()?).await {
        Ok(c) => Some(c),
        Err(e) => { eprintln!("skip: {e}"); None }
    }
}

fn texts(r: &dm_rust::QueryResult, col: usize) -> Vec<String> {
    r.rows.iter().map(|row| row.values[col].to_text().unwrap_or_default()).collect()
}
fn s(x: &str) -> Value { x.into() }

#[tokio::test]
async fn metadata_queries() {
    let Some(mut c) = connect().await else { return };
    let _ = c.execute("drop schema MDQ_S2 cascade", &[]).await;
    let _ = c.execute("drop user MDQ_U cascade", &[]).await;
    for ddl in [
        "create user MDQ_U identified by \"Mdq_U_123456\"",
        "create table MDQ_U.EMP(ID int primary key, NAME varchar(50) not null, DEPT_ID int, SAL decimal(10,2) default 0)",
        "create index MDQ_U.IDX_EMP_DEPT on MDQ_U.EMP(DEPT_ID, NAME)",
        "create unique index MDQ_U.UX_EMP_NAME on MDQ_U.EMP(NAME)",
        "comment on table MDQ_U.EMP is '员工表'",
        "comment on column MDQ_U.EMP.NAME is '姓名'",
        "create view MDQ_U.V_EMP as select ID, NAME from MDQ_U.EMP",
        "create schema MDQ_S2 authorization MDQ_U",
        "create table MDQ_S2.T2(A int)",
    ] {
        c.execute(ddl, &[]).await.unwrap_or_else(|e| panic!("{ddl}: {e}"));
    }

    // 模式：SYSOBJECTS TYPE$='SCH'（含非用户同名模式 MDQ_S2）；ALL_USERS 只有用户
    let r = c.query("select s.NAME as SCHEMA_NAME, u.NAME as OWNER from SYSOBJECTS s join SYSOBJECTS u on s.PID=u.ID where s.TYPE$='SCH' order by s.NAME", &[]).await.unwrap();
    let names = texts(&r, 0);
    assert!(names.contains(&"MDQ_U".into()) && names.contains(&"MDQ_S2".into()));
    let i = names.iter().position(|n| n == "MDQ_S2").unwrap();
    assert_eq!(r.rows[i].values[1].to_text().unwrap(), "MDQ_U");
    let r = c.query("select USERNAME from ALL_USERS order by USERNAME", &[]).await.unwrap();
    assert!(!texts(&r, 0).contains(&"MDQ_S2".into()));

    // 表 / 视图
    let r = c.query("select TABLE_NAME from ALL_TABLES where OWNER=? order by TABLE_NAME", &[s("MDQ_U")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["EMP"]);
    let r = c.query("select TABLE_NAME from ALL_TABLES where OWNER=?", &[s("MDQ_S2")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["T2"]);
    let r = c.query("select VIEW_NAME, TEXT from ALL_VIEWS where OWNER=? order by VIEW_NAME", &[s("MDQ_U")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["V_EMP"]);
    assert!(texts(&r, 1)[0].to_uppercase().contains("FROM MDQ_U.EMP"));

    // 字段
    let r = c.query("select COLUMN_ID, COLUMN_NAME, DATA_TYPE, DATA_LENGTH, DATA_PRECISION, DATA_SCALE, NULLABLE, DATA_DEFAULT from ALL_TAB_COLUMNS where OWNER=? and TABLE_NAME=? order by COLUMN_ID", &[s("MDQ_U"), s("EMP")]).await.unwrap();
    assert_eq!(texts(&r, 1), vec!["ID", "NAME", "DEPT_ID", "SAL"]);
    assert_eq!(texts(&r, 2), vec!["INT", "VARCHAR", "INT", "DECIMAL"]);
    assert_eq!(texts(&r, 6), vec!["N", "N", "Y", "Y"]);
    assert_eq!(r.rows[3].values[4].to_text().unwrap(), "10");
    assert_eq!(r.rows[3].values[7].to_text().unwrap(), "0");

    // 索引（排除聚簇索引和约束自动建的索引）
    let r = c.query("select i.INDEX_NAME, i.UNIQUENESS from ALL_INDEXES i where i.TABLE_OWNER=? and i.TABLE_NAME=? and i.INDEX_TYPE<>'CLUSTER' and not exists (select 1 from ALL_CONSTRAINTS c where c.OWNER=i.OWNER and c.INDEX_NAME=i.INDEX_NAME) order by i.INDEX_NAME", &[s("MDQ_U"), s("EMP")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["IDX_EMP_DEPT", "UX_EMP_NAME"]);
    assert_eq!(texts(&r, 1), vec!["NONUNIQUE", "UNIQUE"]);
    let r = c.query("select COLUMN_NAME, COLUMN_POSITION, DESCEND from ALL_IND_COLUMNS where TABLE_OWNER=? and INDEX_NAME=? order by COLUMN_POSITION", &[s("MDQ_U"), s("IDX_EMP_DEPT")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["DEPT_ID", "NAME"]);
    assert_eq!(texts(&r, 2), vec!["ASC", "ASC"]);

    // 主键
    let r = c.query("select c.CONSTRAINT_NAME, cc.COLUMN_NAME, cc.POSITION from ALL_CONSTRAINTS c join ALL_CONS_COLUMNS cc on c.OWNER=cc.OWNER and c.CONSTRAINT_NAME=cc.CONSTRAINT_NAME where c.OWNER=? and c.TABLE_NAME=? and c.CONSTRAINT_TYPE='P' order by cc.POSITION", &[s("MDQ_U"), s("EMP")]).await.unwrap();
    assert_eq!(texts(&r, 1), vec!["ID"]);

    // 注释
    let r = c.query("select TABLE_NAME, TABLE_TYPE, COMMENTS from ALL_TAB_COMMENTS where OWNER=? order by TABLE_NAME", &[s("MDQ_U")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["EMP", "V_EMP"]);
    assert_eq!(r.rows[0].values[2].to_text().unwrap(), "员工表");
    let r = c.query("select COMMENTS from ALL_COL_COMMENTS where OWNER=? and TABLE_NAME=? and COLUMN_NAME=?", &[s("MDQ_U"), s("EMP"), s("NAME")]).await.unwrap();
    assert_eq!(texts(&r, 0), vec!["姓名"]);

    // DDL（CLOB）
    for (ty, name, needle) in [("TABLE", "EMP", "CREATE TABLE \"MDQ_U\".\"EMP\""), ("VIEW", "V_EMP", "VIEW MDQ_U.V_EMP"), ("INDEX", "IDX_EMP_DEPT", "INDEX \"MDQ_U\".\"IDX_EMP_DEPT\"")] {
        let r = c.query("select DBMS_METADATA.GET_DDL(?, ?, ?) from dual", &[s(ty), s(name), s("MDQ_U")]).await.unwrap();
        let ddl = r.rows[0].values[0].to_text().unwrap();
        assert!(ddl.to_uppercase().contains(&needle.to_uppercase()), "{ty}: {ddl}");
    }
    let e = c.query("select DBMS_METADATA.GET_DDL('TABLE','NOPE','MDQ_U') from dual", &[]).await.unwrap_err();
    assert!(e.to_string().contains("-20008"), "{e}");

    c.execute("drop schema MDQ_S2 cascade", &[]).await.unwrap();
    c.execute("drop user MDQ_U cascade", &[]).await.unwrap();
}
