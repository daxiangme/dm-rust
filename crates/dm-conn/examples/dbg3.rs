use dm_conn::{Connection, ConnectOptions, Value};
#[tokio::main]
async fn main() {
    // 日志由 RUST_LOG 控制，例如 RUST_LOG=dm_conn=trace
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let o = ConnectOptions::from_url("dm://SYSDBA:SYSDBA001@127.0.0.1:5236").unwrap();
    let mut c = Connection::connect(&o).await.unwrap();
    let _ = c.execute("drop table T_DBG3", &[]).await;
    c.execute("create table T_DBG3(id int, b blob, c clob)", &[]).await.unwrap();
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let r = c.execute("insert into T_DBG3 values(1, ?, ?)", &[Value::Bytes(vec![7u8; n].into()), Value::String("é".repeat(n))]).await;
    println!("{:?}", r);
    let q = c.query("select id, b, c from T_DBG3", &[]).await;
    println!("{:?}", q.map(|q| q.rows.iter().map(|r| r.values.iter().map(|v| match v { Value::Bytes(b) => format!("bytes{}", b.len()), Value::String(s) => format!("str{}", s.len()), o => format!("{o:?}") }).collect::<Vec<_>>()).collect::<Vec<_>>()));
    let _ = c.execute("drop table T_DBG3", &[]).await;
}
