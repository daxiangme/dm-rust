//! cargo run -p dm-pool --example pool
use dm_conn::{ConnectOptions, Value};
use dm_pool::{Pool, PoolConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DM_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    let pool = Pool::new(ConnectOptions::from_url(&url)?, PoolConfig { max_size: 4, ..Default::default() });
    let mut tasks = Vec::new();
    for i in 0..16i32 {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            let mut c = pool.get().await?;
            let r = c.query("select ? * ? from dual", &[Value::Int(i), Value::Int(i)]).await?;
            Ok::<_, dm_pool::PoolError>(r.rows[0].values[0].clone())
        }));
    }
    for t in tasks {
        println!("{:?}", t.await??);
    }
    println!("{:?}", pool.status());
    Ok(())
}
