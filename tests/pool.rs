//! 需要本机 DM8（默认 127.0.0.1:5236 SYSDBA/SYSDBA001，可用 DM_TEST_URL 覆盖）；连不上则跳过。
use std::time::Duration;

use dm_rust::{ConnectOptions, Value};
use dm_rust::{Pool, PoolConfig, PoolError};

fn opts() -> ConnectOptions {
    let url = std::env::var("DM_TEST_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5236".into());
    ConnectOptions::from_url(&url).unwrap()
}

async fn pool(cfg: PoolConfig) -> Option<Pool> {
    let p = Pool::new(opts(), cfg);
    match p.get().await {
        Ok(c) => {
            drop(c);
            Some(p)
        }
        Err(e) => {
            eprintln!("SKIP: {e}");
            None
        }
    }
}

#[tokio::test]
async fn reuse_connection() {
    let Some(p) = pool(PoolConfig { max_size: 2, ..Default::default() }).await else { return };
    for i in 0..20 {
        let mut c = p.get().await.unwrap();
        let r = c.query("select ? + 1 from dual", &[Value::Int(i)]).await.unwrap();
        assert_eq!(r.rows[0].values[0], Value::Int(i + 1));
    }
    let s = p.status();
    assert_eq!(s.created_total, 1, "{s:?}");
    assert_eq!(s.idle, 1);
    assert_eq!(s.in_use, 0);
}

#[tokio::test]
async fn concurrency_limited_by_max_size() {
    let Some(p) = pool(PoolConfig { max_size: 3, ..Default::default() }).await else { return };
    let mut hs = vec![];
    for i in 0..12i32 {
        let p = p.clone();
        hs.push(tokio::spawn(async move {
            let mut c = p.get().await.unwrap();
            assert!(p.status().in_use <= 3);
            let r = c.query("select ? from dual", &[Value::Int(i)]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            r.rows[0].values[0].clone()
        }));
    }
    for (i, h) in hs.into_iter().enumerate() {
        assert_eq!(h.await.unwrap(), Value::Int(i as i32));
    }
    let s = p.status();
    assert!(s.created_total <= 3, "{s:?}");
    assert_eq!(s.in_use, 0);
}

#[tokio::test]
async fn acquire_timeout() {
    let Some(p) = pool(PoolConfig { max_size: 1, acquire_timeout: Duration::from_millis(200), ..Default::default() }).await else { return };
    let held = p.get().await.unwrap();
    let e = p.get().await.err().unwrap();
    assert!(matches!(e, PoolError::Timeout), "{e:?}");
    drop(held);
    p.get().await.unwrap();
}

#[tokio::test]
async fn transaction_connection_is_discarded() {
    let Some(p) = pool(PoolConfig { max_size: 2, ..Default::default() }).await else { return };
    {
        let mut c = p.get().await.unwrap();
        c.begin().await.unwrap();
        c.execute("select 1 from dual", &[]).await.unwrap();
        // 未提交就归还 → 被丢弃
    }
    let s = p.status();
    assert_eq!(s.size, 0, "{s:?}");
    let mut c = p.get().await.unwrap();
    assert!(c.auto_commit());
    c.query("select 1 from dual", &[]).await.unwrap();
}

#[tokio::test]
async fn committed_connection_is_reused() {
    let Some(p) = pool(PoolConfig { max_size: 2, ..Default::default() }).await else { return };
    let created = p.status().created_total;
    {
        let mut c = p.get().await.unwrap();
        c.begin().await.unwrap();
        c.execute("select 1 from dual", &[]).await.unwrap();
        c.commit().await.unwrap();
    }
    assert_eq!(p.status().idle, 1);
    drop(p.get().await.unwrap());
    assert_eq!(p.status().created_total, created);
}

#[tokio::test]
async fn idle_timeout_and_ping() {
    let Some(p) = pool(PoolConfig {
        max_size: 2,
        idle_timeout: Some(Duration::from_millis(50)),
        test_after_idle: Some(Duration::from_millis(10)),
        ..Default::default()
    })
    .await
    else {
        return;
    };
    tokio::time::sleep(Duration::from_millis(120)).await;
    let before = p.status().created_total;
    drop(p.get().await.unwrap());
    assert_eq!(p.status().created_total, before + 1, "idle-expired connection must be replaced");
    // 未过期但需要 ping
    tokio::time::sleep(Duration::from_millis(20)).await;
    let before = p.status().created_total;
    let mut c = p.get().await.unwrap();
    c.query("select 1 from dual", &[]).await.unwrap();
    drop(c);
    assert_eq!(p.status().created_total, before);
}

#[tokio::test]
async fn closed_pool_rejects() {
    let Some(p) = pool(PoolConfig::default()).await else { return };
    p.close();
    assert!(matches!(p.get().await.err().unwrap(), PoolError::Closed));
    assert_eq!(p.status().idle, 0);
}

#[tokio::test]
async fn bad_credentials_error_not_hang() {
    let mut o = opts();
    o.user = "NO_SUCH_USER_X".into();
    let p = Pool::new(o, PoolConfig { acquire_timeout: Duration::from_secs(5), ..Default::default() });
    match p.get().await {
        Err(PoolError::Conn(_)) => {}
        Err(PoolError::Timeout) => panic!("hang"),
        Ok(_) => panic!("should fail"),
        Err(e) => eprintln!("SKIP-ish: {e}"),
    }
    assert_eq!(p.status().size, 0);
    // 失败不应泄漏许可：仍可再次尝试
    assert!(p.get().await.is_err());
}
