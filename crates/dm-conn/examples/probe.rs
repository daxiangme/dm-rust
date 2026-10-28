//! 握手原始报文探测：`RUST_LOG=dm_conn=trace cargo run -p dm-conn --example probe -- dm://SYSDBA:pwd@host:port`
use dm_conn::*;
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let url = std::env::args().nth(1).unwrap_or_else(|| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    match Connection::connect(&ConnectOptions::from_url(&url).unwrap()).await {
        Ok(_) => println!("connected OK"),
        Err(e) => println!("connect failed: {e}"),
    }
}
