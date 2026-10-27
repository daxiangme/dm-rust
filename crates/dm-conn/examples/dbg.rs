use dm_conn::{Connection, ConnectOptions, Value};
#[tokio::main]
async fn main() {
    let o = ConnectOptions::from_url("dm://SYSDBA:SYSDBA001@127.0.0.1:5236").unwrap();
    let mut c = Connection::connect(&o).await.unwrap();
    let mut args = std::env::args().skip(1);
    let sql = args.next().unwrap();
    let params: Vec<Value> = args.map(|a| a.parse::<i32>().map(Value::Int).unwrap_or(Value::String(a))).collect();
    match c.query(&sql, &params).await {
        Ok(r) => { println!("cols={:?}", r.columns.iter().map(|c| (&c.name, c.col_type, c.prec, c.scale)).collect::<Vec<_>>()); for row in r.rows { println!("{:?}", row.values); } }
        Err(e) => println!("ERR {e}"),
    }
}
