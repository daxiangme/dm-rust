//! 用法：DM_URL=dm://SYSDBA:SYSDBA001@127.0.0.1:5236 cargo run -p dm-conn --example basic
use dm_conn::{Connection, ConnectOptions, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DM_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    let opts = ConnectOptions::from_url(&url)?;
    let mut conn = Connection::connect(&opts).await?;
    println!("server version: {:?}, schema: {}", conn.server_version(), conn.current_schema());

    let r = conn.query("select 1 as a, 'héllo' as b, 3.14 as c, sysdate as d from dual", &[]).await?;
    for row in &r.rows {
        for (c, v) in r.columns.iter().zip(&row.values) {
            println!("{} ({}) = {:?}", c.name, c.type_name, v);
        }
    }

    conn.execute("create table t_demo(id int primary key, name varchar(50), amount decimal(12,2))", &[]).await.ok();
    conn.execute("delete from t_demo", &[]).await?;
    let n = conn.execute("insert into t_demo values(?, ?, ?)", &[Value::Int(1), "张三".into(), "12.50".parse::<dm_conn::Decimal>()?.into()]).await?;
    println!("inserted {}", n.rows_affected);
    let r = conn.query("select * from t_demo where id = ?", &[Value::Int(1)]).await?;
    println!("{:?}", r.rows[0].values);
    conn.execute("drop table t_demo", &[]).await?;
    conn.close().await?;
    Ok(())
}
