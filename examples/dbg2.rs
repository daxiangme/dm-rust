use dm_rust::{Connection, ConnectOptions, Value};
#[tokio::main]
async fn main() {
    // 日志由 RUST_LOG 控制，例如 RUST_LOG=dm_rust=trace
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let o = ConnectOptions::from_url("dm://SYSDBA:SYSDBA001@127.0.0.1:5236").unwrap();
    let mut c = Connection::connect(&o).await.unwrap();
    let _ = c.execute("drop table T_DBG", &[]).await;
    c.execute("create table T_DBG(id int, s varchar(40))", &[]).await.unwrap();
    for n in [5000usize] {
        let rows: Vec<Vec<Value>> = (0..n).map(|i| vec![Value::Int(i as i32), Value::String(format!("r{i}"))]).collect();
        println!("{n}: {:?}", c.execute_batch("insert into T_DBG values(?,?)", &rows).await.map(|r| r.rows_affected));
    }
    println!("{:?}", c.query("select id,s from T_DBG order by id", &[]).await.map(|r| r.rows.len()));
    println!("{:?}", c.query("select level, rpad('x', 100, 'y') from dual connect by level <= 2000", &[]).await.map(|r| r.rows.len()));
    let _ = c.execute("drop table T_DBG", &[]).await;
}
