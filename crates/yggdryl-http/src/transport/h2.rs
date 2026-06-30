//! HTTP/2 transport via [`hyper`] + [`tokio`] + [`tokio-rustls`].
//!
//! `H2Transport` hides an internal tokio runtime behind the blocking
//! `Transport` trait so the public `HttpSession` API stays synchronous.
//! Each request opens a new TLS+H2 connection (connection pooling is a future
//! optimization — the correctness properties are identical). ALPN is set to
//! `["h2"]`.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex, OnceLock};

use bytes::Bytes;
use http::Uri;
use http_body_util::{BodyExt, Collected, Full};
use hyper::client::conn::http2;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;

use crate::error::HttpError;
use crate::HttpVersion;

use super::{RawResponse, SendConfig, Transport};

/// Shared TLS config for HTTP/2 (ALPN: "h2").
fn default_tls_config() -> Arc<rustls::ClientConfig> {
    static CFG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let roots = webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .cloned()
            .collect::<rustls::RootCertStore>();
        let mut cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.alpn_protocols = vec![b"h2".to_vec()];
        Arc::new(cfg)
    })
    .clone()
}

/// HTTP/2 transport.
pub(crate) struct H2Transport {
    tls_config: Arc<rustls::ClientConfig>,
    /// Connection pool: host:port → reusable H2 sender.
    pool: Arc<Mutex<HashMap<String, http2::SendRequest<Full<Bytes>>>>>,
}

impl H2Transport {
    pub(crate) fn new() -> Self {
        H2Transport {
            tls_config: default_tls_config(),
            pool: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_tls(tls_config: Arc<rustls::ClientConfig>) -> Self {
        H2Transport {
            tls_config,
            pool: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Transport for H2Transport {
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        config: &SendConfig,
    ) -> Result<RawResponse, HttpError> {
        crate::log_event!(debug, "H2 send method={method} url={url}");
        let tls_config = Arc::clone(&self.tls_config);
        let pool = Arc::clone(&self.pool);
        let method = method.to_string();
        let url = url.to_string();
        let headers: Vec<(String, String)> = headers.to_vec();
        let body_bytes: Bytes = body.map(|b| Bytes::copy_from_slice(b)).unwrap_or_default();
        let timeout = config.timeout;

        super::runtime().block_on(async move {
            send_h2(
                tls_config, pool, &method, &url, &headers, body_bytes, timeout,
            )
            .await
        })
    }
}

async fn send_h2(
    tls_config: Arc<rustls::ClientConfig>,
    pool: Arc<Mutex<HashMap<String, http2::SendRequest<Full<Bytes>>>>>,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body_bytes: Bytes,
    timeout: Option<std::time::Duration>,
) -> Result<RawResponse, HttpError> {
    let uri: Uri = url
        .parse()
        .map_err(|e| HttpError::InvalidUrl(format!("{e}")))?;
    let scheme = uri.scheme_str().unwrap_or("https");
    let host = uri
        .host()
        .ok_or_else(|| HttpError::InvalidUrl("URL has no host".to_string()))?;
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    let pool_key = format!("{host}:{port}");

    // Try to reuse a pooled connection.
    let maybe_sender: Option<http2::SendRequest<Full<Bytes>>> =
        pool.lock().ok().and_then(|mut g| g.remove(&pool_key));

    let mut sender = match maybe_sender {
        Some(s) if s.is_ready() => s,
        _ => connect_h2(Arc::clone(&tls_config), host, port, scheme).await?,
    };

    let http_method = http::Method::from_bytes(method.as_bytes())
        .map_err(|e| HttpError::Transport(format!("invalid HTTP method: {e}")))?;

    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let uri_ref: Uri = path_and_query
        .parse()
        .map_err(|e| HttpError::InvalidUrl(format!("{e}")))?;

    let mut builder = http::Request::builder()
        .method(http_method)
        .uri(uri_ref)
        .header(http::header::HOST, host);

    for (k, v) in headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    let req = builder
        .body(Full::new(body_bytes))
        .map_err(|e| HttpError::Transport(format!("request build: {e}")))?;

    let fut = sender.send_request(req);

    let response = if let Some(dur) = timeout {
        tokio::time::timeout(dur, fut)
            .await
            .map_err(|_| HttpError::Timeout)?
            .map_err(|e| HttpError::Transport(e.to_string()))?
    } else {
        fut.await.map_err(|e| HttpError::Transport(e.to_string()))?
    };

    let status = response.status().as_u16();
    let resp_headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();

    let content_length: Option<u64> = resp_headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<u64>().ok());

    let body_collected: Collected<Bytes> = response
        .into_body()
        .collect()
        .await
        .map_err(|e| HttpError::Transport(e.to_string()))?;
    let body_bytes_out = body_collected.to_bytes();

    // Return the sender to the pool for reuse.
    if sender.is_ready() {
        if let Ok(mut g) = pool.lock() {
            g.insert(pool_key, sender);
        }
    }

    let body_reader: Box<dyn Read + Send + 'static> =
        Box::new(Cursor::new(body_bytes_out.to_vec()));

    Ok(RawResponse {
        status,
        headers: resp_headers,
        version: HttpVersion::Http2,
        content_length,
        body: body_reader,
    })
}

/// Opens a new HTTP/2 connection to `host:port` and returns the sender.
/// The connection driver is spawned on the tokio runtime.
async fn connect_h2(
    tls_config: Arc<rustls::ClientConfig>,
    host: &str,
    port: u16,
    scheme: &str,
) -> Result<http2::SendRequest<Full<Bytes>>, HttpError> {
    let addr = format!("{host}:{port}");
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| HttpError::Transport(format!("DNS resolve {addr}: {e}")))?
        .next()
        .ok_or_else(|| HttpError::Transport(format!("no addresses for {addr}")))?;

    let tcp = tokio::net::TcpStream::connect(sock_addr)
        .await
        .map_err(|e| HttpError::Transport(format!("connect {addr}: {e}")))?;
    tcp.set_nodelay(true)
        .map_err(|e| HttpError::Transport(format!("set_nodelay: {e}")))?;

    if scheme == "https" {
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|e| HttpError::Tls(format!("invalid server name {host:?}: {e}")))?;
        let tls = TlsConnector::from(tls_config)
            .connect(server_name, tcp)
            .await
            .map_err(|e| HttpError::Tls(format!("TLS handshake: {e}")))?;
        let (sender, conn) = http2::Builder::new(TokioExecutor::new())
            .handshake(TokioIo::new(tls))
            .await
            .map_err(|e| HttpError::Transport(format!("H2 handshake: {e}")))?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                crate::log_event!(debug, "H2 connection error: {e}");
            }
        });
        Ok(sender)
    } else {
        // h2c: cleartext HTTP/2 with prior knowledge
        let (sender, conn) = http2::Builder::new(TokioExecutor::new())
            .handshake(TokioIo::new(tcp))
            .await
            .map_err(|e| HttpError::Transport(format!("H2 handshake: {e}")))?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                crate::log_event!(debug, "H2 connection error: {e}");
            }
        });
        Ok(sender)
    }
}
