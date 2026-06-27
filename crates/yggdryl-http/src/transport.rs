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
//! Each path returns the response **headers immediately** while the body is
//! delivered lazily via an [`AsyncBodyStream`]: bytes flow from a background tokio
//! task into a synchronous [`mpsc`] channel, so callers can start reading before
//! the body is fully received. Request bodies are buffered (the whole upload is in
//! memory before the request fires) so they stay replayable on a retry. A coarse
//! [`REQUEST_TIMEOUT`] bounds the whole header-receiving leg; the body read is
//! bounded by the caller's own read timeout. No HTTP `CONNECT` proxy support yet:
//! the transport connects directly, so it needs direct egress.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio_rustls::rustls::ClientConfig;
use yggdryl_core::{Io, IoError, IoStats, Url, Whence};

use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
use crate::time::Instant;
use crate::version::HttpVersion;

#[cfg(any(feature = "http2", feature = "http3"))]
use bytes::Bytes;
#[cfg(feature = "http2")]
use http_body_util::{BodyExt, Full};
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
    pub(crate) body: &'a [u8],
    pub(crate) prefer: HttpVersion,
    /// Whether to verify the server's TLS certificate (mirrors the session's
    /// [`verify`](crate::HttpSession::verify); `false` accepts any certificate).
    pub(crate) verify: bool,
    /// Installed CA certificates (DER) that replace the default trust store when
    /// non-empty (mirrors the session's CA installer).
    pub(crate) ca_certs: &'a [Vec<u8>],
}

/// The result of an async round-trip: status, response headers, a streaming body,
/// the protocol [`version`](RawResponse::version) actually spoken, and the shared
/// [`received_at`](RawResponse::received_at) instant stamped when the body reaches
/// EOF (mirroring the `HttpStream` pattern on the HTTP/1.1 path).
pub(crate) struct RawResponse {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    /// The response body as a streaming [`Io`] handle — an [`AsyncBodyStream`] for
    /// HTTP/2 and HTTP/3. Draining it reads lazily from the network.
    pub(crate) body: Box<dyn Io>,
    pub(crate) version: HttpVersion,
    /// Shared with the [`AsyncBodyStream`] body; stamped when the body reaches EOF.
    pub(crate) received_at: Instant,
}

// ---------------------------------------------------------------------------
// Async body → sync Io bridge.
// ---------------------------------------------------------------------------

/// A streaming [`Io`] body backed by an `mpsc` channel fed by a background tokio
/// task (the body reader). Returned by the HTTP/2 and HTTP/3 transports so the
/// response headers are available immediately while the body is read lazily.
///
/// Seeking **forward** (discarding bytes) is supported; seeking backward or from
/// the end requires buffering the whole body first — call
/// [`into_bytesio`](crate::HttpResponse::into_bytesio) or
/// [`bytes`](crate::HttpResponse::bytes) for that.
///
/// When this handle is **dropped before the body is fully consumed**, the
/// background task is aborted, which drops the underlying h2/h3 stream and
/// closes the connection cleanly.
#[derive(Debug)]
pub(crate) struct AsyncBodyStream {
    /// Body chunks arriving from the background task.
    receiver: Mutex<mpsc::Receiver<Result<Bytes, IoError>>>,
    /// Bytes received but not yet handed to the caller.
    buffer: VecDeque<u8>,
    /// Whether the channel reached EOF (the body is fully received).
    done: bool,
    /// Absolute byte position of the logical cursor.
    position: u64,
    /// Declared body size from `Content-Length`, or `None` if chunked/unknown.
    size: Option<u64>,
    url: Url,
    /// Stamped when the body reaches EOF — shared with the response's
    /// `received_at` so the timing reflects the full download.
    received_at: Instant,
    /// The background body-reader task; aborted on drop to close the connection
    /// when the body is abandoned before EOF.
    _task: tokio::task::JoinHandle<()>,
}

impl AsyncBodyStream {
    fn new(
        receiver: mpsc::Receiver<Result<Bytes, IoError>>,
        size: Option<u64>,
        url: Url,
        received_at: Instant,
        task: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self {
            receiver: Mutex::new(receiver),
            buffer: VecDeque::new(),
            done: false,
            position: 0,
            size,
            url,
            received_at,
            _task: task,
        }
    }

    /// Fills the internal buffer from the channel until either `buf.len()` bytes
    /// are available or the channel is exhausted.
    fn fill_buffer(&mut self) -> Result<(), IoError> {
        if self.done || !self.buffer.is_empty() {
            return Ok(());
        }
        let rx = self
            .receiver
            .lock()
            .expect("AsyncBodyStream receiver poisoned");
        match rx.recv() {
            Ok(Ok(chunk)) => {
                if chunk.is_empty() {
                    self.done = true;
                } else {
                    self.buffer.extend(chunk.iter());
                }
            }
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                // Channel closed by the sender — body is fully received.
                self.done = true;
                self.received_at.stamp_once();
            }
        }
        Ok(())
    }
}

impl Drop for AsyncBodyStream {
    fn drop(&mut self) {
        // Aborting the background task drops the underlying connection stream,
        // sending RST_STREAM / QUIC stream reset so the server learns the body
        // is unwanted — cleaner than letting the connection time out.
        self._task.abort();
    }
}

impl Io for AsyncBodyStream {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(self.size.unwrap_or(0)))
    }

    fn stream_position(&self) -> u64 {
        self.position
    }

    fn stream_len(&self) -> Option<u64> {
        self.size
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let target = match whence {
            Whence::Start => {
                if offset < 0 {
                    return Err(IoError::Invalid(format!(
                        "seek to negative position {offset}"
                    )));
                }
                offset as u64
            }
            Whence::Current => {
                let pos = self.position as i64 + offset;
                if pos < 0 {
                    return Err(IoError::Invalid(format!("seek to negative position {pos}")));
                }
                pos as u64
            }
            Whence::End => {
                return Err(IoError::Unsupported(
                    "seek from end on a streaming h2/h3 body requires the whole body; \
                     call into_bytesio() first"
                        .into(),
                ))
            }
        };
        if target < self.position {
            return Err(IoError::Unsupported(
                "backward seek on a streaming h2/h3 body is unsupported; \
                 call into_bytesio() first"
                    .into(),
            ));
        }
        // Forward seek: drain and discard bytes up to the target.
        let mut to_skip = target - self.position;
        while to_skip > 0 {
            self.fill_buffer()?;
            if self.buffer.is_empty() {
                // Seeking past EOF: clamp to current position.
                break;
            }
            let take = to_skip.min(self.buffer.len() as u64) as usize;
            self.buffer.drain(..take);
            self.position += take as u64;
            to_skip -= take as u64;
        }
        Ok(self.position)
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.fill_buffer()?;
        let n = {
            let contiguous = self.buffer.make_contiguous();
            let n = contiguous.len().min(buf.len());
            buf[..n].copy_from_slice(&contiguous[..n]);
            n
        };
        self.buffer.drain(..n);
        self.position += n as u64;
        // Stamp received_at on the first read that reaches EOF (empty buffer +
        // done) so the timing reflects the last byte landing.
        if n == 0 && self.done {
            self.received_at.stamp_once();
        }
        Ok(n)
    }

    /// Positional read: `Whence::Current` acts like [`read`](Io::read) (advances
    /// the cursor). `Whence::Start` at a position ahead of the cursor drains up
    /// to that offset, then reads — the cursor ends at offset + count (not
    /// restored). Backward seeks and `Whence::End` return [`IoError::Unsupported`];
    /// call [`into_bytesio`](crate::HttpResponse::into_bytesio) first for those.
    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        match whence {
            Whence::Current => {
                if offset != 0 {
                    self.seek(offset, Whence::Current)?;
                }
                self.read(buf)
            }
            Whence::Start => {
                let target = if offset < 0 {
                    return Err(IoError::Invalid(format!(
                        "seek to negative position {offset}"
                    )));
                } else {
                    offset as u64
                };
                if target < self.position {
                    return Err(IoError::Unsupported(
                        "positional pread before the current position on a streaming \
                         h2/h3 body is unsupported; call into_bytesio() first"
                            .into(),
                    ));
                }
                self.seek(offset, Whence::Start)?;
                self.read(buf)
            }
            Whence::End => Err(IoError::Unsupported(
                "positional pread from end on a streaming h2/h3 body requires the \
                 whole body; call into_bytesio() first"
                    .into(),
            )),
        }
    }

    fn close(&mut self) -> Result<(), IoError> {
        self.done = true;
        self._task.abort();
        Ok(())
    }
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

/// Dispatches one async request, blocking the calling thread until the response
/// **headers** are available (or [`REQUEST_TIMEOUT`] elapses). The body is
/// returned as a streaming [`AsyncBodyStream`] read lazily by the caller.
pub(crate) fn send(request: AsyncRequest<'_>) -> Result<RawResponse, HttpError> {
    log_event!(
        debug,
        "async transport: {} {} (prefer {})",
        request.method.as_str(),
        request.url,
        request.prefer
    );
    let received_at = Instant::new();
    runtime().block_on(async {
        match tokio::time::timeout(REQUEST_TIMEOUT, send_async(request, received_at.clone())).await
        {
            Ok(result) => result,
            Err(_) => Err(HttpError::Transport(format!(
                "request timed out after {}s — the server sent no response headers; \
                 raise the read timeout for slow endpoints",
                REQUEST_TIMEOUT.as_secs()
            ))),
        }
    })
}

/// Dispatches to the protocol-specific path based on the request's preference.
async fn send_async(
    request: AsyncRequest<'_>,
    received_at: Instant,
) -> Result<RawResponse, HttpError> {
    match request.prefer {
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => h3_request(request, received_at).await,
        #[cfg(feature = "http2")]
        _ => h2_send(request, received_at).await,
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
/// returns a streaming response.
#[cfg(feature = "http2")]
async fn h2_send(
    request: AsyncRequest<'_>,
    received_at: Instant,
) -> Result<RawResponse, HttpError> {
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
            h2_request(tls, request, received_at).await
        } else if request.prefer == HttpVersion::Http2 {
            Err(HttpError::Unsupported(
                "server refused HTTP/2 over ALPN; retry with HttpVersion::Auto or Http11".into(),
            ))
        } else {
            // Auto fallback: the server chose http/1.1 — speak it over the same TLS.
            h1_request(tls, request, received_at).await
        }
    } else {
        // Cleartext: h2c (prior knowledge) when h2 is pinned, else HTTP/1.1.
        match request.prefer {
            HttpVersion::Http2 => h2_request(tcp, request, received_at).await,
            _ => h1_request(tcp, request, received_at).await,
        }
    }
}

/// Performs an HTTP/2 request over an established `stream`, returning the response
/// headers immediately with a streaming body (its
/// [`version`](RawResponse::version) is [`Http2`](HttpVersion::Http2)).
///
/// The connection driver task is spawned and kept alive by the body reader task,
/// which feeds chunks into an [`mpsc`] channel. The driver is aborted when the
/// body reader exits (EOF or error) or when the caller drops the
/// [`AsyncBodyStream`] (early abandon).
#[cfg(feature = "http2")]
async fn h2_request<S>(
    stream: S,
    request: AsyncRequest<'_>,
    received_at: Instant,
) -> Result<RawResponse, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let hyper_request = build_request(&request, false)?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let conn_handle = tokio::spawn(async move {
        let _ = conn.await;
    });
    sender
        .ready()
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let response = sender
        .send_request(hyper_request)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;

    let status = response.status().as_u16();
    let mut headers = HttpHeaders::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str(), value);
        }
    }
    let size = headers.content_size();
    let url = request.url.clone();

    let (tx, rx) = mpsc::sync_channel::<Result<Bytes, IoError>>(8);
    let mut incoming = response.into_body();
    let ra = received_at.clone();

    let task = tokio::spawn(async move {
        loop {
            match incoming.frame().await {
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        // Skip empty data frames (e.g. END_STREAM-only) — an empty
                        // chunk on the channel would prematurely EOF the consumer.
                        if !data.is_empty() && tx.send(Ok(data)).is_err() {
                            break; // receiver dropped — body abandoned
                        }
                    }
                    // Non-data frames (trailers) are silently skipped.
                }
                Some(Err(err)) => {
                    let _ = tx.send(Err(IoError::Io(err.to_string())));
                    break;
                }
                None => {
                    ra.stamp_once(); // body fully received from the network
                    break; // EOF
                }
            }
        }
        conn_handle.abort();
    });

    Ok(RawResponse {
        status,
        headers,
        body: Box::new(AsyncBodyStream::new(
            rx,
            size,
            url,
            received_at.clone(),
            task,
        )),
        version: HttpVersion::Http2,
        received_at,
    })
}

/// Performs an HTTP/1.1 request over an established `stream` (the Auto ALPN
/// fallback when a TLS server declines `h2`). Returns the response headers
/// immediately with a streaming body, identical in shape to [`h2_request`].
#[cfg(feature = "http2")]
async fn h1_request<S>(
    stream: S,
    request: AsyncRequest<'_>,
    received_at: Instant,
) -> Result<RawResponse, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let hyper_request = build_request(&request, true)?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let conn_handle = tokio::spawn(async move {
        let _ = conn.await;
    });
    sender
        .ready()
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    let response = sender
        .send_request(hyper_request)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;

    let status = response.status().as_u16();
    let mut headers = HttpHeaders::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str(), value);
        }
    }
    let size = headers.content_size();
    let url = request.url.clone();

    let (tx, rx) = mpsc::sync_channel::<Result<Bytes, IoError>>(8);
    let mut incoming = response.into_body();
    let ra = received_at.clone();

    let task = tokio::spawn(async move {
        loop {
            match incoming.frame().await {
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        if !data.is_empty() && tx.send(Ok(data)).is_err() {
                            break; // receiver dropped — body abandoned
                        }
                    }
                    // Non-data frames (trailers) are silently skipped.
                }
                Some(Err(err)) => {
                    let _ = tx.send(Err(IoError::Io(err.to_string())));
                    break;
                }
                None => {
                    ra.stamp_once(); // body fully received from the network
                    break; // EOF
                }
            }
        }
        conn_handle.abort();
    });

    Ok(RawResponse {
        status,
        headers,
        body: Box::new(AsyncBodyStream::new(
            rx,
            size,
            url,
            received_at.clone(),
            task,
        )),
        version: HttpVersion::Http11,
        received_at,
    })
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
        .body(Full::new(Bytes::copy_from_slice(request.body)))
        .map_err(|err| HttpError::InvalidHeader(err.to_string()))
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

/// Performs an HTTP/3 request over a fresh QUIC connection, returning the response
/// headers immediately with a streaming body. HTTP/3 is TLS-only, so the URL must
/// be `https`; the connection offers ALPN `h3`. A background body-reader task feeds
/// chunks into an [`mpsc`] channel — the connection driver and QUIC endpoint are
/// closed by that task once the body is fully read (or the stream is abandoned).
#[cfg(feature = "http3")]
async fn h3_request(
    request: AsyncRequest<'_>,
    received_at: Instant,
) -> Result<RawResponse, HttpError> {
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

    let (mut h3_conn, mut send_request) = h3::client::new(h3_quinn::Connection::new(connection))
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    // The driver task keeps the h3 connection alive (polls it); it is aborted by
    // the body-reader task once the body stream is done or abandoned.
    let driver_task = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| h3_conn.poll_close(cx)).await;
    });

    // Send the request and receive the response headers before handing off to the
    // background body-reader task — headers are available immediately.
    let req = build_h3_request(&request)?;
    let mut stream = send_request
        .send_request(req)
        .await
        .map_err(|err| HttpError::Transport(err.to_string()))?;
    if !request.body.is_empty() {
        stream
            .send_data(bytes::Bytes::copy_from_slice(request.body))
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
    let size = headers.content_size();
    let url_clone = url.clone();

    let (tx, rx) = mpsc::sync_channel::<Result<Bytes, IoError>>(8);
    let ra = received_at.clone();

    let task = tokio::spawn(async move {
        loop {
            match stream.recv_data().await {
                Ok(Some(mut chunk)) => {
                    let data = chunk.copy_to_bytes(chunk.remaining());
                    if !data.is_empty() && tx.send(Ok(data)).is_err() {
                        break; // receiver dropped — body abandoned
                    }
                }
                Ok(None) => {
                    ra.stamp_once(); // body fully received from the network
                    break; // EOF
                }
                Err(err) => {
                    let _ = tx.send(Err(IoError::Io(err.to_string())));
                    break;
                }
            }
        }
        driver_task.abort();
        // Best-effort CONNECTION_CLOSE; the body is either fully read or abandoned
        // so we do not `wait_idle()` — avoid adding teardown latency.
        endpoint.close(0u32.into(), b"done");
    });

    Ok(RawResponse {
        status,
        headers,
        body: Box::new(AsyncBodyStream::new(
            rx,
            size,
            url_clone,
            received_at.clone(),
            task,
        )),
        version: HttpVersion::Http3,
        received_at,
    })
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
