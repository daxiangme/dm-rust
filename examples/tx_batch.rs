//! 事务 + 批量插入 + 错误处理示例：cargo run --example tx_batch
use dm_rust::{ConnectOptions, Connection, Error, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DM_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    let mut c = Connection::connect(&ConnectOptions::from_url(&url)?).await?;
    let _ = c.execute("drop table DEMO_TX", &[]).await;
    c.execute("create table DEMO_TX(id int primary key, name varchar(50))", &[]).await?;

    // 批量插入：一次往返提交多行参数
    let rows: Vec<Vec<Value>> = (1..=5).map(|i| vec![Value::Int(i), Value::String(format!("用户{i}"))]).collect();
    let r = c.execute_batch("insert into DEMO_TX values(?,?)", &rows).await?;
    println!("批量插入 {} 行", r.rows_affected);

    // 显式事务：回滚
    c.begin().await?;
    c.execute("delete from DEMO_TX where id > ?", &[Value::Int(2)]).await?;
    println!("事务内剩余 {:?}", c.query_one("select count(*) from DEMO_TX", &[]).await?.unwrap().values[0]);
    c.rollback().await?;
    println!("回滚后 {:?}", c.query_one("select count(*) from DEMO_TX", &[]).await?.unwrap().values[0]);

    // 服务端错误（主键冲突）以 Error::Server 返回，连接仍可用
    match c.execute("insert into DEMO_TX values(1, 'dup')", &[]).await {
        Err(Error::Server { code, message }) => println!("预期的错误 {code}: {message}"),
        other => println!("意外: {other:?}"),
    }
    c.execute("drop table DEMO_TX", &[]).await?;
    c.close().await?;
    Ok(())
}
