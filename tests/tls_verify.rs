//! TLS 证书校验真机测试。需要一个 ENABLE_ENCRYPT=1、使用 tests/tls/server-cert.pem 的实例（见 tests/tls/README.md）。
//! 默认 127.0.0.1:5237，可用 DM_TEST_TLS_ADDR=host:port 覆盖；实例不可达或未开 TLS 时跳过。
#![cfg(feature = "tls")]
use dm_rust::{ConnectOptions, Connection, Error};

fn dir() -> String {
    format!("{}/tests/tls", env!("CARGO_MANIFEST_DIR"))
}

async fn try_url(q: &str) -> Result<Connection, Error> {
    let addr = std::env::var("DM_TEST_TLS_ADDR").unwrap_or_else(|_| "127.0.0.1:5237".into());
    let o = ConnectOptions::from_url(&format!("dm://SYSDBA:SYSDBA001@{addr}?{q}")).unwrap();
    Connection::connect(&o).await
}

#[tokio::test]
async fn tls_certificate_verification() {
    let ca = format!("{}/ca.pem", dir());
    let other = format!("{}/other-ca.pem", dir());
    if !std::path::Path::new(&ca).is_file() || !std::path::Path::new(&other).is_file() {
        eprintln!("SKIP: cert files under tests/tls are not present");
        return;
    }
    // 先确认实例可达且是 TLS 实例：不校验时能连上
    let mut c = match try_url("ssl_verify=false").await {
        Ok(c) => c,
        Err(e) => return eprintln!("skip: {e}"),
    };
    c.query("select 1 from dual", &[]).await.unwrap();
    if !matches!(try_url(&format!("ssl_ca={other}")).await, Err(Error::Tls(_))) {
        return eprintln!("skip: instance is not using TLS with the test certificate");
    }

    // 正确 CA + IP SAN
    let mut c = try_url(&format!("ssl_ca={ca}")).await.expect("custom CA should verify");
    assert_eq!(c.query("select 1 from dual", &[]).await.unwrap().rows.len(), 1);
    // 正确 CA + DNS 服务器名
    try_url(&format!("ssl_ca={ca}&ssl_server_name=localhost")).await.expect("server name localhost");
    // 服务器名不匹配
    let e = try_url(&format!("ssl_ca={ca}&ssl_server_name=db.example.com")).await.err().expect("name mismatch must fail");
    assert!(matches!(e, Error::Tls(_)), "{e}");
    // 无关 CA
    let e = try_url(&format!("ssl_ca={other}")).await.err().expect("wrong CA must fail");
    assert!(matches!(e, Error::Tls(_)), "{e}");
    // 默认：系统根证书，自签证书必须被拒绝
    let e = try_url("").await.err().expect("default verification must reject self-signed chain");
    assert!(matches!(e, Error::Tls(_)), "{e}");
    // CA 文件不存在
    let e = try_url("ssl_ca=/nonexistent/ca.pem").await.err().unwrap();
    assert!(e.to_string().contains("read CA file"), "{e}");
}
