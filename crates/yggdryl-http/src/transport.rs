//! The optional async HTTP/2 and HTTP/3 transports (`http2` / `http3` features).
//!
//! The default client speaks HTTP/1.1 over the blocking `ureq` transport. When a
//! request negotiates a newer protocol — pinned [`HttpVersion::Http2`] /
//! [`HttpVersion::Http3`], or [`HttpVersion::Auto`] picking `h2` through TLS ALPN —
//! the blocking layer funnels it here instead. This module owns a small
//! **multi-threaded tokio runtime** and drives [`hyper`]'s h2 client (over TCP/TLS)
//! or the `quinn` + `h3` client (over QUIC/UDP) on it, presenting the same buffered
//! request/response shape (`RawResponse`) the rest of the crate consumes, so
//! redirects, cookies and retries in [`HttpSession::send`](crate::HttpSession::send)
//! work identically whatever the protocol.
//!
//! Transport selection (see [`route_for`]):
//! - `https` + pinned `h2` → TLS offering only `h2` (errors if the server refuses);
//! - `https` + `Auto` → TLS offering `h2`,`http/1.1` and uses whichever the server
//!   picks (a real ALPN fallback, reported via [`RawResponse::version`]);
//! - `http` (cleartext) + pinned `h2` → h2c (HTTP/2 with prior knowledge);
//! - `https` + pinned `h3` → QUIC with ALPN `h3` (HTTP/3 is TLS-only).
//!
//! Each path **buffers** the response body into memory (the returned body is a
//! seekable [`BytesIO`](yggdryl_core::BytesIO)); streaming a body — and a
//! per-response size cap (none yet, as on the buffered HTTP/1.1 path) — are later
//! refinements. Request bodies are buffered too, so they are replayable on a retry.
//! A coarse [`REQUEST_TIMEOUT`] bounds the whole round-trip so a stalled server
//! cannot pin the calling thread indefinitely. TLS certificate verification follows
//! the session's [`verify`](crate::HttpSession::verify) flag. No HTTP `CONNECT`
//! proxy support yet: the transport connects directly, so it needs direct egress.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio_rustls::rustls::ClientConfig;
use yggdryl_core::Url;

use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
use crate::version::HttpVersion;

#[cfg(feature = "http2")]
use http_body_util::{BodyExt, Full};
#[cfg(feature = "http2")]
use hyper::body::Bytes;
#[cfg(feature = "http2")]
use hyper_util::rt::{TokioExecutor, TokioIo};
#[cfg(feature = "http2")]
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "http2")]
use tokio::net::TcpStream;

/// Decides whether the async transport should handle a request for `requested`
/// against `url`, returning the protocol preference to negotiate, or `None` to
/// leave it on the blocking HTTP/1.1 (`ureq`) path.
pub(crate) fn route_for(requested: HttpVersion, url: &Url) -> Option<HttpVersion> {
    match requested {
        // Pinned HTTP/2: h2c for cleartext, ALPN-`h2`-only for TLS.
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => Some(HttpVersion::Http2),
        // Pinned HTTP/3 is QUIC, which is TLS-only — handled here for `https`.
        #[cfg(feature = "http3")]
        HttpVersion::Http3 if url.scheme().eq_ignore_ascii_case("https") => {
            Some(HttpVersion::Http3)
        }
        // Auto over TLS negotiates h2/http1 by ALPN; cleartext Auto stays on h1.
        #[cfg(feature = "http2")]
        HttpVersion::Auto if url.scheme().eq_ignore_ascii_case("https") => Some(HttpVersion::Auto),
        _ => None,
    }
}

/// A request handed to the async transport: the resolved URL, merged headers and a
/// **buffered** (replayable) body, plus the `prefer`red protocol to negotiate.
pub(crate) struct AsyncRequest<'a> {
    pub(crate) method: Method,
    pub(crate) url: &'a Url,
    pub(crate) headers: &'a HttpHeaders,
    pub(crate) body: Vec<u8>,
    pub(crate) prefer: HttpVersion,
    /// Whether to verify the server's TLS certificate (mirrors the session's
    /// [`verify`](crate::HttpSession::verify); `false` accepts any certificate).
    pub(crate) verify: bool,
    /// Installed CA certificates (DER) that replace the default trust store when
    /// non-empty (mirrors the session's CA installer).
    pub(crate) ca_certs: &'a [Vec<u8>],
}

/// The buffered result of an async round-trip: status, response headers, the whole
/// body, and the protocol [`version`](RawResponse::version) actually spoken.
pub(crate) struct RawResponse {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
    pub(crate) version: HttpVersion,
}

/// The process-wide tokio runtime the async transport runs on, built on first use.
///
/// A **multi-threaded** runtime (a small worker pool) is used rather than a
/// current-thread one so spawned connection tasks keep being driven *between*
/// `block_on` calls: that lets the connection task close its socket promptly when
/// aborted after the body is buffered, and lets concurrent
/// [`send_many`](crate::HttpSession::send_many) requests each `block_on` the shared
/// runtime without serialising.
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build the async transport runtime")
    })
}

/// A coarse safety backstop on a single round-trip, so a black-hole server (accepts
/// the socket but never completes TLS / never answers) cannot pin the calling
/// thread forever. It bounds the whole connect→TLS→request→body sequence.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs one async request to completion on the transport runtime, blocking the
/// calling thread until the response is buffered (or [`REQUEST_TIMEOUT`] elapses).
pub(crate) fn send(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
    log_event!(
        debug,
        "async transport: {} {} (prefer {})",
        request.method.as_str(),
        request.url,
        request.prefer
    );
    runtime().block_on(async {
        match tokio::time::timeout(REQUEST_TIMEOUT, send_async(request)).await {
            Ok(result) => result,
            Err(_) => Err(HttpError::Transport(format!(
                "request timed out after {}s",
                REQUEST_TIMEOUT.as_secs()
            ))),
        }
    })
}

/// Dispatches to the protocol-specific path based on the request's preference.
async fn send_async(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
    match request.prefer {
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => h3_request(request).await,
        #[cfg(feature = "http2")]
        _ => h2_send(request).await,
        #[cfg(not(feature = "http2"))]
        _ => Err(HttpError::Unsupported(
            "no async transport for the requested version is compiled in".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Shared TLS configuration.
// ---------------------------------------------------------------------------

/// A rustls certificate verifier that accepts **any** certificate — used only when
/// the session sets `verify=false`. Signatures are still checked (so the handshake
/// is well-formed); only the certificate *trust chain* is skipped.
#[derive(Debug)]
struct NoVerify(Arc<tokio_rustls::rustls::crypto::CryptoProvider>);

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        tokio_rustls::rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        tokio_rustls::rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The shared webpki root certificate store, built once (assembling it copies the
/// whole Mozilla root set) and handed out as a cheap `Arc` clone.
fn root_store() -> Arc<tokio_rustls::rustls::RootCertStore> {
    static ROOTS: OnceLock<Arc<tokio_rustls::rustls::RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut roots = tokio_rustls::rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(roots)
        })
        .clone()
}

/// Builds a rustls [`ClientConfig`] offering `alpn`. When `verify` is on the
/// certificate is checked against the installed `ca_certs` if any are given
/// (replacing the defaults, like the session's CA installer), else the shared
/// [`root_store`]; `verify` off trusts any certificate (and logs a warning). The
/// shared root store keeps the common path cheap per connection.
fn client_config(alpn: Vec<Vec<u8>>, verify: bool, ca_certs: &[Vec<u8>]) -> ClientConfig {
    use tokio_rustls::rustls::pki_types::CertificateDer;
    use tokio_rustls::rustls::RootCertStore;

    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("rustls ring provider supports the default protocol versions");
    let mut config = if verify {
        let roots = if ca_certs.is_empty() {
            root_store()
        } else {
            let mut store = RootCertStore::empty();
            for der in ca_certs {
                if store.add(CertificateDer::from(der.clone())).is_err() {
                    log_event!(warn, "an installed CA certificate was rejected by rustls");
                }
            }
            if store.is_empty() {
                log_event!(
                    warn,
                    "all installed CA certificates were rejected; no roots are trusted, so \
                     every TLS connection will fail"
                );
            }
            Arc::new(store)
        };
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        log_event!(
            warn,
            "TLS certificate verification is DISABLED (verify=false) for the async \
             transport; the connection is insecure"
        );
        if !ca_certs.is_empty() {
            log_event!(
                warn,
                "verify=false ignores the {} installed CA certificate(s); keep verify=true \
                 to validate against them",
                ca_certs.len()
            );
        }
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth()
    };
    config.alpn_protocols = alpn;
    config
}

/// The `host[:port]` authority (never userinfo) and the `path?query` (never the
/// fragment) of `url` — the two wire components both transports build a request
/// from. A fragment must not go on the wire and userinfo is illegal in an HTTP/2 or
/// HTTP/3 `:authority`.
fn wire_parts(url: &Url) -> (String, String) {
    let authority = match url.port() {
        Some(port) => format!("{}:{}", url.host(), port),
        None => url.host().to_string(),
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let path_and_query = match url.query() {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_string(),
    };
    (authority, path_and_query)
}

/// Whether `name` is a connection-specific (hop-by-hop) header that is **malformed**
/// over HTTP/2 and HTTP/3 (RFC 9113 §8.2.2 / RFC 9114 §4.2) — a compliant server
/// treats such a header as a stream error — so it must not be forwarded on those
/// protocols. `te` is only legal with the value `trailers`, which we never send, so
/// it is dropped wholesale.
fn is_hop_by_hop(name: &str) -> bool {
    [
        "connection",
        "keep-alive",
        "proxy-connection",
        "transfer-encoding",
        "upgrade",
        "te",
    ]
    .iter()
    .any(|header| name.eq_ignore_ascii_case(header))
}

// ---------------------------------------------------------------------------
// HTTP/2 (and the Auto http/1.1 fallback) over hyper.
// ---------------------------------------------------------------------------

/// Opens the connection (h2c / TLS+ALPN), speaks the negotiated protocol and
/// buffers the response.
#[cfg(feature = "http2")]
async fn h2_send(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
    let url = request.url;
    let host = url.host().to_string();
    if host.is_empty() {
        return Err(HttpError::InvalidUrl(
            "missing host for http2 request".into(),
        ));
    }
    let is_tls = url.scheme().eq_ignore_ascii_case("https");
    let port = url.port().unwrap_or(if is_tls { 443 } else { 80 });

    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    tcp.set_nodelay(true).ok();

    if is_tls {
        // Offer h2 alone when pinned, h2+http/1.1 when negotiating (Auto).
        let alpn = if request.prefer == HttpVersion::Http2 {
            vec![b"h2".to_vec()]
        } else {
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        };
        let tls = tls_connect(tcp, &host, alpn, request.verify, request.ca_certs).await?;
        let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
        if h2 {
            h2_request(tls, request).await
        } else if request.prefer == HttpVersion::Http2 {
            Err(HttpError::Unsupported(
                "server refused HTTP/2 over ALPN; retry with HttpVersion::Auto or Http11".into(),
            ))
        } else {
            // Auto fallback: the server chose http/1.1 — speak it over the same TLS.
            h1_request(tls, request).await
        }
    } else {
        // Cleartext: h2c (prior knowledge) when h2 is pinned, else HTTP/1.1.
        match request.prefer {
            HttpVersion::Http2 => h2_request(tcp, request).await,
            _ => h1_request(tcp, request).await,
        }
    }
}

/// Performs an HTTP/2 request over an established `stream`, returning the buffered
/// response (its [`version`](RawResponse::version) is [`Http2`](HttpVersion::Http2)).
///
/// The connection future is spawned to drive the request, then **aborted** once the
/// body is buffered: the runtime's worker drops the aborted task promptly, closing
/// the socket instead of leaving it open for a connection we will not reuse.
#[cfg(feature = "http2")]
async fn h2_request<S>(stream: S, request: AsyncRequest<'_>) -> Result<RawResponse, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let hyper_request = build_request(&request, false)?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let conn = tokio::spawn(async move {
        let _ = conn.await;
    });
    let result = async {
        sender
            .ready()
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        let response = sender
            .send_request(hyper_request)
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        collect(response, HttpVersion::Http2).await
    }
    .await;
    conn.abort();
    result
}

/// Performs an HTTP/1.1 request over an established `stream` (the Auto ALPN
/// fallback when a TLS server declines `h2`). Like [`h2_request`], the connection
/// task is aborted once the body is buffered.
#[cfg(feature = "http2")]
async fn h1_request<S>(stream: S, request: AsyncRequest<'_>) -> Result<RawResponse, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let hyper_request = build_request(&request, true)?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let conn = tokio::spawn(async move {
        let _ = conn.await;
    });
    let result = async {
        sender
            .ready()
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        let response = sender
            .send_request(hyper_request)
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        collect(response, HttpVersion::Http11).await
    }
    .await;
    conn.abort();
    result
}

/// Builds the `hyper` request from our prepared one. `with_host` adds a `Host`
/// header (HTTP/1.1 needs it; HTTP/2 derives `:authority` from the URI instead).
#[cfg(feature = "http2")]
fn build_request(
    request: &AsyncRequest<'_>,
    with_host: bool,
) -> Result<hyper::Request<Full<Bytes>>, HttpError> {
    let (authority, path_and_query) = wire_parts(request.url);
    let uri = hyper::Uri::builder()
        .scheme(request.url.scheme())
        .authority(authority.as_str())
        .path_and_query(path_and_query)
        .build()
        .map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
    let mut builder = hyper::Request::builder()
        .method(request.method.as_str())
        .uri(uri);
    let mut has_host = false;
    for (name, value) in request.headers.iter() {
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
            // HTTP/2 derives `:authority` from the URI; a literal Host is redundant.
            // HTTP/1.1 (with_host) needs it, so keep it there.
            if !with_host {
                continue;
            }
        } else if !with_host && is_hop_by_hop(name) {
            // Hop-by-hop headers are malformed over HTTP/2; never forward them.
            continue;
        }
        builder = builder.header(name, value);
    }
    if with_host && !has_host {
        builder = builder.header("host", &authority);
    }
    builder
        .body(Full::new(Bytes::from(request.body.clone())))
        .map_err(|err| HttpError::InvalidHeader(err.to_string()))
}

/// Drains a hyper response into a [`RawResponse`], tagging it with the protocol
/// `version` it was delivered over.
#[cfg(feature = "http2")]
async fn collect(
    response: hyper::Response<hyper::body::Incoming>,
    version: HttpVersion,
) -> Result<RawResponse, HttpError> {
    let status = response.status().as_u16();
    let mut headers = HttpHeaders::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str(), value);
        }
    }
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?
        .to_bytes()
        .to_vec();
    Ok(RawResponse {
        status,
        headers,
        body,
        version,
    })
}

/// Wraps `stream` in a rustls TLS session for `host`, offering `alpn` and verifying
/// the certificate unless `verify` is `false`. The negotiated ALPN id is read back
/// by the caller.
#[cfg(feature = "http2")]
async fn tls_connect<S>(
    stream: S,
    host: &str,
    alpn: Vec<Vec<u8>>,
    verify: bool,
    ca_certs: &[Vec<u8>],
) -> Result<tokio_rustls::client::TlsStream<S>, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::TlsConnector;

    let connector = TlsConnector::from(Arc::new(client_config(alpn, verify, ca_certs)));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
    connector.connect(server_name, stream).await.map_err(|err| {
        // A TLS handshake failure with verification on is usually an untrusted cert;
        // give the same actionable hint the h1 path does.
        if verify {
            HttpError::Transport(format!(
                "tls error: {err}; if this host uses a self-signed or internal certificate, \
                 install its CA or set verify=false (insecure) to skip verification"
            ))
        } else {
            HttpError::Transport(err.to_string())
        }
    })
}

// ---------------------------------------------------------------------------
// HTTP/3 over QUIC (quinn + h3).
// ---------------------------------------------------------------------------

/// Performs an HTTP/3 request over a fresh QUIC connection, buffering the response.
/// HTTP/3 is TLS-only, so the URL must be `https`; the connection offers ALPN `h3`.
/// The connection driver is spawned and aborted once the body is buffered.
#[cfg(feature = "http3")]
async fn h3_request(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
    use bytes::Buf;

    let url = request.url;
    let host = url.host().to_string();
    if host.is_empty() {
        return Err(HttpError::InvalidUrl(
            "missing host for http3 request".into(),
        ));
    }
    let port = url.port().unwrap_or(443);
    // QUIC connects to a resolved socket address (UDP).
    let addr = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?
        .next()
        .ok_or_else(|| HttpError::Transport(format!("could not resolve {host}")))?;

    // A QUIC client endpoint bound to an ephemeral local UDP port, using a rustls
    // config that offers only ALPN `h3`.
    let tls = client_config(vec![b"h3".to_vec()], request.verify, request.ca_certs);
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let bind: std::net::SocketAddr = if addr.is_ipv6() {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    };
    let mut endpoint =
        quinn::Endpoint::client(bind).map_err(|err| HttpError::Transport(err.to_string()))?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic)));

    let connecting = endpoint
        .connect(addr, &host)
        .map_err(|err| h3_tls_hint(err.to_string(), request.verify))?;
    let connection = connecting
        .await
        .map_err(|err| h3_tls_hint(err.to_string(), request.verify))?;

    let (mut driver, mut send_request) = h3::client::new(h3_quinn::Connection::new(connection))
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let driver = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
    });

    let result = async {
        let req = build_h3_request(&request)?;
        let mut stream = send_request
            .send_request(req)
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        if !request.body.is_empty() {
            stream
                .send_data(bytes::Bytes::from(request.body.clone()))
                .await
                .map_err(|err| HttpError::Transport(err.to_string()))?;
        }
        stream
            .finish()
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        let response = stream
            .recv_response()
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?;
        let status = response.status().as_u16();
        let mut headers = HttpHeaders::new();
        for (name, value) in response.headers() {
            if let Ok(value) = value.to_str() {
                headers.insert(name.as_str(), value);
            }
        }
        let mut body = Vec::new();
        while let Some(mut chunk) = stream
            .recv_data()
            .await
            .map_err(|err| HttpError::Transport(err.to_string()))?
        {
            while chunk.has_remaining() {
                let bytes = chunk.chunk();
                body.extend_from_slice(bytes);
                let len = bytes.len();
                chunk.advance(len);
            }
        }
        Ok(RawResponse {
            status,
            headers,
            body,
            version: HttpVersion::Http3,
        })
    }
    .await;

    driver.abort();
    // Queue a CONNECTION_CLOSE and drop the endpoint (we do not pool QUIC
    // connections yet). This is best-effort: the body is already fully buffered, so
    // we do not `wait_idle()` to flush the close frame — a server learns of the
    // close via its idle timeout at worst, and we avoid adding teardown latency to
    // every request.
    endpoint.close(0u32.into(), b"done");
    result
}

/// Wraps a QUIC connect error, adding the self-signed-certificate hint only when
/// verification is on **and** the failure looks certificate/TLS-related (a timeout
/// or a refused/unreachable peer is reported plainly, not as a cert problem).
#[cfg(feature = "http3")]
fn h3_tls_hint(message: String, verify: bool) -> HttpError {
    let lower = message.to_ascii_lowercase();
    let cert_related =
        lower.contains("cert") || lower.contains("tls") || lower.contains("unknownissuer");
    if verify && cert_related {
        HttpError::Transport(format!(
            "quic/tls error: {message}; if this host uses a self-signed or internal \
             certificate, install its CA or set verify=false (insecure) to skip verification"
        ))
    } else {
        HttpError::Transport(format!("quic error: {message}"))
    }
}

/// Builds the `http::Request<()>` for the h3 client (the body is sent separately
/// over the stream). The URI is assembled from explicit parts — no userinfo, no
/// fragment — exactly like the h2 path.
#[cfg(feature = "http3")]
fn build_h3_request(request: &AsyncRequest<'_>) -> Result<http::Request<()>, HttpError> {
    let (authority, path_and_query) = wire_parts(request.url);
    let uri = http::Uri::builder()
        .scheme(request.url.scheme())
        .authority(authority.as_str())
        .path_and_query(path_and_query)
        .build()
        .map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
    let mut builder = http::Request::builder()
        .method(request.method.as_str())
        .uri(uri);
    for (name, value) in request.headers.iter() {
        // HTTP/3 derives `:authority` from the URI (a literal Host is redundant and
        // rejected by some servers), and hop-by-hop headers are malformed over h3.
        if name.eq_ignore_ascii_case("host") || is_hop_by_hop(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
        .body(())
        .map_err(|err| HttpError::InvalidHeader(err.to_string()))
}
