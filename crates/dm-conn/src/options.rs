use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    pub(crate) fn wire(self) -> i32 {
        match self {
            IsolationLevel::ReadUncommitted => 0,
            IsolationLevel::ReadCommitted => 1,
            IsolationLevel::RepeatableRead => 2,
            IsolationLevel::Serializable => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Off,
    /// 总是压缩
    Always,
    /// 大于 8 KiB 的非本机报文才压缩
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressAlgo {
    Zlib,
    Snappy,
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub schema: Option<String>,
    pub app_name: String,
    pub connect_timeout: Duration,
    /// 单次请求-响应超时（None = 不限）
    pub request_timeout: Option<Duration>,
    /// 登录凭证 / 会话加密（DH）。默认开启。
    pub login_encrypt: bool,
    pub compression: Compression,
    pub compress_algo: CompressAlgo,
    pub isolation: Option<IsolationLevel>,
    pub read_only: bool,
    /// 客户端时区（分钟）；None = 使用本机时区
    pub timezone_min: Option<i16>,
    /// 语句缓存容量（0 = 关闭）
    pub stmt_cache_capacity: usize,
    /// 每次 FETCH 的行数
    pub fetch_size: i64,
    /// 服务端为 TLS 模式时是否校验证书（当前仅支持不校验）
    pub tls_verify: bool,
}

impl ConnectOptions {
    pub fn new(host: impl Into<String>, port: u16, user: impl Into<String>, password: impl Into<String>) -> Self {
        ConnectOptions {
            host: host.into(),
            port,
            user: user.into(),
            password: password.into(),
            schema: None,
            app_name: "dm-rust".into(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: None,
            login_encrypt: true,
            compression: Compression::Off,
            compress_algo: CompressAlgo::Zlib,
            isolation: None,
            read_only: false,
            timezone_min: None,
            stmt_cache_capacity: 64,
            fetch_size: 1000,
            tls_verify: false,
        }
    }

    /// 解析 `dm://user:password@host:port[/schema]`（用户名/密码支持 %XX 转义）。
    pub fn from_url(url: &str) -> Result<Self, String> {
        let rest = url.strip_prefix("dm://").ok_or("url must start with dm://")?;
        let (auth, hostpart) = rest.rsplit_once('@').ok_or("missing '@'")?;
        let (user, pass) = auth.split_once(':').ok_or("missing ':' in credentials")?;
        let (hostport, schema) = match hostpart.split_once('/') {
            Some((h, s)) if !s.is_empty() => (h, Some(pct_decode(s))),
            Some((h, _)) => (h, None),
            None => (hostpart, None),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|e| e.to_string())?),
            None => (hostport, 5236),
        };
        let mut o = ConnectOptions::new(host, port, pct_decode(user), pct_decode(pass));
        o.schema = schema;
        Ok(o)
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url() {
        let o = ConnectOptions::from_url("dm://SYSDBA:p%40ss@127.0.0.1:5237/MYSCHEMA").unwrap();
        assert_eq!((o.user.as_str(), o.password.as_str(), o.host.as_str(), o.port), ("SYSDBA", "p@ss", "127.0.0.1", 5237));
        assert_eq!(o.schema.as_deref(), Some("MYSCHEMA"));
        assert_eq!(ConnectOptions::from_url("dm://a:b@h").unwrap().port, 5236);
        assert!(ConnectOptions::from_url("mysql://a:b@h").is_err());
    }
}
