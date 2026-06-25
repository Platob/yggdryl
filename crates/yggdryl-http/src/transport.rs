//! The optional async HTTP/2 transport (the `http2` feature).
//!
//! The default client speaks HTTP/1.1 over the blocking `ureq` transport. When a
//! request negotiates HTTP/2 — pinned [`HttpVersion::Http2`], or
//! [`HttpVersion::Auto`] picking `h2` through TLS ALPN — the blocking layer funnels
//! it here instead. This module owns a small **multi-threaded tokio runtime** and
//! drives [`hyper`]'s h2 client over it, presenting the same buffered
//! request/response shape (`RawResponse`) the rest of the crate consumes, so
//! redirects, cookies and retries in [`HttpSession::send`](crate::HttpSession::send)
//! work identically whatever the protocol.
//!
//! Transport selection (see [`route_for`]):
//! - `https` + pinned `h2` → TLS offering only `h2` (errors if the server refuses);
//! - `https` + `Auto` → TLS offering `h2`,`http/1.1` and uses whichever the server
//!   picks (a real ALPN fallback, reported via [`RawResponse::version`]);
//! - `http` (cleartext) + pinned `h2` → h2c (HTTP/2 with prior knowledge).
//!
//! The h2 path **buffers** the response body into memory (the returned body is a
//! seekable [`BytesIO`](yggdryl_core::BytesIO)); streaming an h2 body — and a
//! per-response size cap (none yet, as on the buffered HTTP/1.1 path) — are later
//! refinements. Request bodies are buffered too, so they are replayable on a retry.
//! A coarse [`REQUEST_TIMEOUT`] bounds the whole round-trip so a stalled server
//! cannot pin the calling thread indefinitely. No HTTP `CONNECT` proxy support yet:
//! the transport connects directly, so it needs direct outbound egress.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use yggdryl_core::Url;

use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
use crate::version::HttpVersion;

/// Decides whether the async transport should handle a request for `requested`
/// against `url`, returning the protocol preference to negotiate, or `None` to
/// leave it on the blocking HTTP/1.1 (`ureq`) path.
pub(crate) fn route_for(requested: HttpVersion, url: &Url) -> Option<HttpVersion> {
    match requested {
        // Pinned HTTP/2: h2c for cleartext, ALPN-`h2`-only for TLS.
        HttpVersion::Http2 => Some(HttpVersion::Http2),
        // Auto over TLS negotiates h2/http1 by ALPN; cleartext Auto stays on h1.
        HttpVersion::Auto if url.scheme().eq_ignore_ascii_case("https") => Some(HttpVersion::Auto),
        _ => None,
    }
}

/// A request handed to the async transport: the resolved URL, merged headers and a
/// **buffered** (replayable) body, plus the `prefer`red protocol to negotiate
/// ([`Http2`](HttpVersion::Http2) pins h2, [`Auto`](HttpVersion::Auto) negotiates).
pub(crate) struct AsyncRequest<'a> {
    pub(crate) method: Method,
    pub(crate) url: &'a Url,
    pub(crate) headers: &'a HttpHeaders,
    pub(crate) body: Vec<u8>,
    pub(crate) prefer: HttpVersion,
    /// Whether to verify the server's TLS certificate (mirrors the session's
    /// [`verify`](crate::HttpSession::verify); `false` accepts any certificate).
    pub(crate) verify: bool,
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
            .expect("failed to build the http2 transport runtime")
    })
}

/// A coarse safety backstop on a single h2 round-trip, so a black-hole server
/// (accepts the socket but never completes TLS / never answers) cannot pin the
/// calling thread forever. It bounds the whole connect→TLS→request→body sequence.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs one async request to completion on the transport runtime, blocking the
/// calling thread until the response is buffered (or [`REQUEST_TIMEOUT`] elapses).
pub(crate) fn send(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
    log_event!(
        debug,
        "http2 transport: {} {} (prefer {})",
        request.method.as_str(),
        request.url,
        request.prefer
    );
    runtime().block_on(async {
        match tokio::time::timeout(REQUEST_TIMEOUT, send_async(request)).await {
            Ok(result) => result,
            Err(_) => Err(HttpError::Transport(format!(
                "http2 request timed out after {}s",
                REQUEST_TIMEOUT.as_secs()
            ))),
        }
    })
}

/// Opens the connection (h2c / TLS+ALPN), speaks the negotiated protocol and
/// buffers the response.
async fn send_async(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
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
        let h2_only = request.prefer == HttpVersion::Http2;
        let tls = tls_connect(tcp, &host, h2_only, request.verify).await?;
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

/// Builds the `hyper` request from our prepared one, assembling the URI from
/// explicit parts so it carries **no userinfo and no fragment**: a fragment must
/// never go on the wire, and userinfo in an HTTP/2 `:authority` is illegal.
/// `with_host` adds a `Host` header (HTTP/1.1 needs it; HTTP/2 derives `:authority`
/// from the URI instead).
fn build_request(
    request: &AsyncRequest<'_>,
    with_host: bool,
) -> Result<hyper::Request<Full<Bytes>>, HttpError> {
    let url = request.url;
    // `host[:port]` only — never `user:pw@`.
    let authority = match url.port() {
        Some(port) => format!("{}:{}", url.host(), port),
        None => url.host().to_string(),
    };
    // Path (defaulting to `/`) plus query, but never the fragment.
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let path_and_query = match url.query() {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_string(),
    };
    let uri = hyper::Uri::builder()
        .scheme(url.scheme())
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

/// The rustls [`ClientConfig`](tokio_rustls::rustls::ClientConfig) for a given ALPN
/// offer and verification mode. The four combinations are built once and reused:
/// assembling the webpki root store is not free. A `verify=false` config (which
/// trusts any certificate) logs a warning the first time it is built.
fn tls_config(h2_only: bool, verify: bool) -> Arc<tokio_rustls::rustls::ClientConfig> {
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};

    static CONFIGS: [OnceLock<Arc<ClientConfig>>; 4] = [
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
    ];
    let slot = &CONFIGS[(h2_only as usize) | ((verify as usize) << 1)];
    slot.get_or_init(|| {
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("rustls ring provider supports the default protocol versions");
        let mut config = if verify {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder.with_root_certificates(roots).with_no_client_auth()
        } else {
            log_event!(
                warn,
                "TLS certificate verification is DISABLED (verify=false) for the http2 \
                 transport; connections are insecure"
            );
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
                .with_no_client_auth()
        };
        config.alpn_protocols = if h2_only {
            vec![b"h2".to_vec()]
        } else {
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        };
        Arc::new(config)
    })
    .clone()
}

/// Wraps `stream` in a rustls TLS session for `host`, offering `h2` alone (when
/// `h2_only`) or `h2`,`http/1.1` (when negotiating), verifying the certificate
/// unless `verify` is `false`. The negotiated ALPN id is read back by the caller.
async fn tls_connect<S>(
    stream: S,
    host: &str,
    h2_only: bool,
    verify: bool,
) -> Result<tokio_rustls::client::TlsStream<S>, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::TlsConnector;

    let connector = TlsConnector::from(tls_config(h2_only, verify));
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
