//! dm-pool：dm-conn 的异步连接池。
//!
//! - 信号量限制并发连接数（`max_size`），获取连接带超时。
//! - 空闲连接 LIFO 复用；超过 `idle_timeout` / `max_lifetime` 的连接被丢弃；
//!   空闲超过 `test_after_idle` 的连接在取出时先 `ping`。
//! - 归还时若连接已损坏、仍处于事务中或自动提交被关闭，则直接丢弃而不是放回池中。
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dm_conn::{ConnectOptions, Connection};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub use dm_conn;

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("timed out waiting for a connection from the pool")]
    Timeout,
    #[error("pool is closed")]
    Closed,
    #[error(transparent)]
    Conn(#[from] dm_conn::Error),
}

pub type Result<T> = std::result::Result<T, PoolError>;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 最大连接数（含借出的）。
    pub max_size: usize,
    /// 取连接的最长等待时间（排队 + 建连）。
    pub acquire_timeout: Duration,
    /// 空闲连接最长保留时间（None = 不限）。
    pub idle_timeout: Option<Duration>,
    /// 连接最长存活时间（None = 不限）。
    pub max_lifetime: Option<Duration>,
    /// 空闲超过该时长的连接在取出时先 ping（None = 不检查）。
    pub test_after_idle: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_size: 10,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(600)),
            max_lifetime: Some(Duration::from_secs(1800)),
            test_after_idle: Some(Duration::from_secs(30)),
        }
    }
}

struct Idle {
    conn: Connection,
    created: Instant,
    last_used: Instant,
}

struct Inner {
    opts: ConnectOptions,
    cfg: PoolConfig,
    sem: Arc<Semaphore>,
    idle: Mutex<VecDeque<Idle>>,
    /// 当前存在的连接总数（空闲 + 借出）
    total: AtomicUsize,
    created_total: AtomicUsize,
}

impl Inner {
    fn discard(&self) {
        self.total.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 连接池（可 clone，内部共享）。
#[derive(Clone)]
pub struct Pool {
    inner: Arc<Inner>,
}

#[derive(Debug, Clone, Copy)]
pub struct PoolStatus {
    pub max_size: usize,
    pub size: usize,
    pub idle: usize,
    pub in_use: usize,
    /// 累计创建过的连接数
    pub created_total: usize,
}

impl Pool {
    pub fn new(opts: ConnectOptions, cfg: PoolConfig) -> Pool {
        let max = cfg.max_size.max(1);
        let mut cfg = cfg;
        cfg.max_size = max;
        Pool {
            inner: Arc::new(Inner {
                opts,
                sem: Arc::new(Semaphore::new(max)),
                cfg,
                idle: Mutex::new(VecDeque::new()),
                total: AtomicUsize::new(0),
                created_total: AtomicUsize::new(0),
            }),
        }
    }

    /// 使用默认配置，并立即建立一条连接以验证参数。
    pub async fn connect(opts: ConnectOptions) -> Result<Pool> {
        let p = Pool::new(opts, PoolConfig::default());
        drop(p.get().await?);
        Ok(p)
    }

    pub fn status(&self) -> PoolStatus {
        let idle = self.inner.idle.lock().unwrap().len();
        let size = self.inner.total.load(Ordering::Relaxed);
        PoolStatus {
            max_size: self.inner.cfg.max_size,
            size,
            idle,
            in_use: size.saturating_sub(idle),
            created_total: self.inner.created_total.load(Ordering::Relaxed),
        }
    }

    /// 关闭连接池：拒绝新的获取，丢弃空闲连接（借出的连接归还时被丢弃）。
    pub fn close(&self) {
        self.inner.sem.close();
        let mut g = self.inner.idle.lock().unwrap();
        let n = g.len();
        g.clear();
        self.inner.total.fetch_sub(n, Ordering::Relaxed);
    }

    /// 获取一条连接。
    pub async fn get(&self) -> Result<PooledConnection> {
        let deadline = Instant::now() + self.inner.cfg.acquire_timeout;
        match tokio::time::timeout(self.inner.cfg.acquire_timeout, self.get_inner(deadline)).await {
            Ok(r) => r,
            Err(_) => Err(PoolError::Timeout),
        }
    }

    async fn get_inner(&self, _deadline: Instant) -> Result<PooledConnection> {
        let permit = self.inner.sem.clone().acquire_owned().await.map_err(|_| PoolError::Closed)?;
        loop {
            let cand = self.inner.idle.lock().unwrap().pop_back();
            match cand {
                Some(mut it) => {
                    let cfg = &self.inner.cfg;
                    let now = Instant::now();
                    let expired = cfg.idle_timeout.is_some_and(|t| now.duration_since(it.last_used) > t)
                        || cfg.max_lifetime.is_some_and(|t| now.duration_since(it.created) > t)
                        || it.conn.is_broken();
                    if expired {
                        self.inner.discard();
                        continue;
                    }
                    if cfg.test_after_idle.is_some_and(|t| now.duration_since(it.last_used) > t) && it.conn.ping().await.is_err() {
                        self.inner.discard();
                        continue;
                    }
                    return Ok(PooledConnection { conn: Some(it.conn), created: it.created, pool: self.inner.clone(), _permit: permit });
                }
                None => {
                    let conn = Connection::connect(&self.inner.opts).await?;
                    self.inner.total.fetch_add(1, Ordering::Relaxed);
                    self.inner.created_total.fetch_add(1, Ordering::Relaxed);
                    return Ok(PooledConnection { conn: Some(conn), created: Instant::now(), pool: self.inner.clone(), _permit: permit });
                }
            }
        }
    }
}

/// 借出的连接；Drop 时自动归还（或在不可复用时丢弃）。
pub struct PooledConnection {
    conn: Option<Connection>,
    created: Instant,
    pool: Arc<Inner>,
    _permit: OwnedSemaphorePermit,
}

impl PooledConnection {
    /// 不归还连接，直接关闭（例如调用方知道会话状态已被污染）。
    pub fn discard(mut self) {
        if self.conn.take().is_some() {
            self.pool.discard();
        }
    }
}

impl Deref for PooledConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("connection taken")
    }
}

impl DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn.as_mut().expect("connection taken")
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else { return };
        let cfg = &self.pool.cfg;
        let reusable = !conn.is_broken()
            && !conn.in_transaction()
            && conn.auto_commit()
            && !self.pool.sem.is_closed()
            && !cfg.max_lifetime.is_some_and(|t| self.created.elapsed() > t);
        if reusable {
            self.pool.idle.lock().unwrap().push_back(Idle { conn, created: self.created, last_used: Instant::now() });
        } else {
            self.pool.discard();
        }
    }
}
