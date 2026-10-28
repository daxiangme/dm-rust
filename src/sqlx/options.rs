use std::str::FromStr;
use std::time::Duration;

use crate::sqlx::connection::DmConnection;
use crate::sqlx::error::{map_err, ConfigError};
use futures_core::future::BoxFuture;
use log::LevelFilter;
use sqlx_core::connection::{ConnectOptions, LogSettings};
use sqlx_core::error::Error;
use sqlx_core::Url;

/// 连接参数。密码不会出现在 `Debug` 输出里。
///
/// URL 形式：`dm://user:password@host:port[/schema]`。
/// 用户名和密码里的 `%XX` 会解码。默认端口 5236。
pub struct DmConnectOptions {
    inner: crate::conn::ConnectOptions,
    log_settings: LogSettings,
}

impl Clone for DmConnectOptions {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            log_settings: self.log_settings.clone(),
        }
    }
}

impl std::fmt::Debug for DmConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmConnectOptions")
            .field("host", &self.inner.host)
            .field("port", &self.inner.port)
            .field("user", &self.inner.user)
            .field("password", &"********")
            .field("schema", &self.inner.schema)
            .finish()
    }
}

impl DmConnectOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self::from_inner(crate::conn::ConnectOptions::new(host, port, user, password))
    }

    pub fn from_inner(inner: crate::conn::ConnectOptions) -> Self {
        Self {
            inner,
            log_settings: LogSettings::default(),
        }
    }

    /// 底层 [`crate::ConnectOptions`]。可用来改 `fetch_size`、语句缓存容量、超时、压缩等。
    pub fn inner(&self) -> &crate::conn::ConnectOptions {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut crate::conn::ConnectOptions {
        &mut self.inner
    }
}

impl From<crate::conn::ConnectOptions> for DmConnectOptions {
    fn from(inner: crate::conn::ConnectOptions) -> Self {
        Self::from_inner(inner)
    }
}

impl FromStr for DmConnectOptions {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(s).map_err(|e| Error::Configuration(Box::new(e)))?;
        <Self as ConnectOptions>::from_url(&url)
    }
}

impl ConnectOptions for DmConnectOptions {
    type Connection = DmConnection;

    fn from_url(url: &Url) -> Result<Self, Error> {
        crate::conn::ConnectOptions::from_url(url.as_str())
            .map(Self::from_inner)
            .map_err(|message| Error::Configuration(Box::new(ConfigError(message))))
    }

    fn to_url_lossy(&self) -> Url {
        let mut url = Url::parse("dm://127.0.0.1:5236").expect("static dm url");
        let _ = url.set_username(&self.inner.user);
        let _ = url.set_password(Some(&self.inner.password));
        let _ = url.set_host(Some(&self.inner.host));
        let _ = url.set_port(Some(self.inner.port));
        match &self.inner.schema {
            Some(schema) => url.set_path(&format!("/{schema}")),
            None => url.set_path(""),
        }
        url
    }

    fn connect(&self) -> BoxFuture<'_, Result<Self::Connection, Error>>
    where
        Self::Connection: Sized,
    {
        Box::pin(async move {
            let inner = crate::conn::Connection::connect(&self.inner)
                .await
                .map_err(map_err)?;
            Ok(DmConnection::new(inner, self.log_settings.clone()))
        })
    }

    fn log_statements(mut self, level: LevelFilter) -> Self {
        self.log_settings.log_statements(level);
        self
    }

    fn log_slow_statements(mut self, level: LevelFilter, duration: Duration) -> Self {
        self.log_settings.log_slow_statements(level, duration);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_roundtrip_redacts_password() {
        let opts: DmConnectOptions = "dm://SYSDBA:p%40ss@127.0.0.1:5237/MYSCHEMA"
            .parse()
            .unwrap();
        assert_eq!(opts.inner().user, "SYSDBA");
        assert_eq!(opts.inner().password, "p@ss");
        assert_eq!(opts.inner().host, "127.0.0.1");
        assert_eq!(opts.inner().port, 5237);
        assert_eq!(opts.inner().schema.as_deref(), Some("MYSCHEMA"));
        let again = DmConnectOptions::from_url(&opts.to_url_lossy()).unwrap();
        assert_eq!(again.inner().password, "p@ss");
        assert_eq!(again.inner().schema.as_deref(), Some("MYSCHEMA"));
        let rendered = format!("{opts:?}");
        assert!(!rendered.contains("p@ss"));
        assert!(rendered.contains("SYSDBA"));
        assert!("mysql://a:b@h".parse::<DmConnectOptions>().is_err());
    }
}
