use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

pub enum Stream {
    /// 升级 TLS 期间的占位（不可读写）
    #[allow(dead_code)]
    Empty,
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Stream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Empty => Poll::Ready(Err(io::Error::new(io::ErrorKind::NotConnected, "stream unavailable"))),
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Stream::Empty => Poll::Ready(Err(io::Error::new(io::ErrorKind::NotConnected, "stream unavailable"))),
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Empty => Poll::Ready(Err(io::Error::new(io::ErrorKind::NotConnected, "stream unavailable"))),
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Empty => Poll::Ready(Err(io::Error::new(io::ErrorKind::NotConnected, "stream unavailable"))),
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

#[cfg(feature = "tls")]
pub(crate) mod tls {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerifier, ServerCertVerified};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};
    use std::sync::Arc;

    /// 接受任意服务端证书（DM 默认自签；与官方驱动行为一致）。
    #[derive(Debug)]
    struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
        }
        fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    /// `verify=false`：接受任意证书（DM 默认自签证书时用）。
    /// `verify=true`：校验证书链与服务器名；`ca_file` 给出时只信任其中的 CA，否则用系统根证书。
    pub fn connector(verify: bool, ca_file: Option<&std::path::Path>) -> crate::conn::error::Result<tokio_rustls::TlsConnector> {
        use crate::conn::error::Error;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| Error::Tls(e.to_string()))?;
        let cfg = if !verify {
            builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider))).with_no_client_auth()
        } else {
            let mut roots = rustls::RootCertStore::empty();
            if let Some(path) = ca_file {
                let pem = std::fs::read(path).map_err(|e| Error::Tls(format!("read CA file {}: {e}", path.display())))?;
                let mut n = 0;
                for c in rustls_pemfile::certs(&mut &pem[..]) {
                    let c = c.map_err(|e| Error::Tls(format!("parse CA file {}: {e}", path.display())))?;
                    roots.add(c).map_err(|e| Error::Tls(format!("bad CA certificate in {}: {e}", path.display())))?;
                    n += 1;
                }
                if n == 0 {
                    return Err(Error::Tls(format!("no certificate found in {}", path.display())));
                }
            } else {
                let r = rustls_native_certs::load_native_certs();
                for c in r.certs {
                    let _ = roots.add(c);
                }
                if roots.is_empty() {
                    return Err(Error::Tls(format!("no system root certificates available ({:?}); set ssl_ca or ssl_verify=false", r.errors)));
                }
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        };
        Ok(tokio_rustls::TlsConnector::from(Arc::new(cfg)))
    }
}
