//! The connection-pooling [`HttpSession`] and the concurrent [`send_many`] support.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use yggdryl_core::{Io, Url};
// Buffering a body into a `BytesIO` now only happens on the async (h2/h3) path.
#[cfg(any(feature = "http2", feature = "http3"))]
use yggdryl_core::BytesIO;

use crate::bridge::IoBridge;
use crate::cookies::{Cookie, HttpCookies};
use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
use crate::redirect::{self, DEFAULT_MAX_REDIRECTS};
use crate::request::{Body, HttpRequest, DEFAULT_KEEP_ALIVE};
use crate::response::HttpResponse;
use crate::retry::{RetryConfig, DEFAULT_POOL};
use crate::stream::HttpStream;
use crate::time::{now_secs, Instant};
use crate::version::HttpVersion;

/// The default read timeout: a request errors if the server sends no response (or
/// stalls mid-body) for this long. Two minutes — generous, but bounded so a dead
/// server cannot hang the caller forever. Raise it with
/// [`with_read_timeout`](HttpSession::with_read_timeout) for genuinely slow endpoints.
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Builds the pooled `ureq` agent: statuses are surfaced (not errors), the idle
/// pool is sized to `max_pool`, and **ureq's own redirect following is disabled**
/// (`max_redirects(0)`) so the 3xx surfaces to our [`redirect`](crate::redirect)
/// layer, which owns method/body/cookie/security semantics. `verify` toggles
/// TLS certificate verification (disabling it logs a warning — connections become
/// insecure), and `proxy` routes requests through an HTTP/SOCKS proxy when set.
fn build_agent(
    max_pool: usize,
    verify: bool,
    proxy: Option<ureq::Proxy>,
    ca_certs: &[Vec<u8>],
    read_timeout: Duration,
) -> ureq::Agent {
    // A zero TTL means "unbounded" — pass `None` so ureq applies no timeout (a
    // `Some(ZERO)` would instead time out immediately).
    let recv_timeout = (!read_timeout.is_zero()).then_some(read_timeout);
    let mut builder = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_idle_connections(max_pool)
        .max_idle_connections_per_host(max_pool)
        // Drop a pooled connection left idle longer than the keep-alive TTL.
        .max_idle_age(DEFAULT_KEEP_ALIVE)
        // Bound the wait for the response head and each body read, so a stalled
        // server surfaces a timeout instead of hanging the caller.
        .timeout_recv_response(recv_timeout)
        .timeout_recv_body(recv_timeout)
        .proxy(proxy);
    if !verify {
        log_event!(
            warn,
            "TLS certificate verification is DISABLED (verify=false); connections are insecure"
        );
        if !ca_certs.is_empty() {
            log_event!(
                warn,
                "verify=false ignores the {} installed CA certificate(s); keep verify=true \
                 to validate against them",
                ca_certs.len()
            );
        }
        builder = builder.tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        );
    } else if !ca_certs.is_empty() {
        // Installed CA certificates *replace* the default trust store (like
        // `requests`' `verify=<bundle>`): the session trusts exactly these.
        let certs: Vec<ureq::tls::Certificate> = ca_certs
            .iter()
            .map(|der| ureq::tls::Certificate::from_der(der).to_owned())
            .collect();
        builder = builder.tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::new_with_certs(&certs))
                .build(),
        );
    } else {
        // Default trust store: the **OS-native** certificate store (Windows
        // SChannel, macOS Security framework, Linux system bundle) via the platform
        // verifier, so the session honours certificates the host already trusts
        // (corporate roots, OS updates) instead of a bundled Mozilla snapshot.
        builder = builder.tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        );
    }
    builder.build().into()
}

/// Parses CA certificate input into DER blobs: a PEM bundle (one or more
/// `-----BEGIN CERTIFICATE-----` blocks) yields each certificate, while a raw DER
/// certificate is taken as a single blob. Each blob is structurally validated as a
/// DER X.509 certificate, so a malformed cert is rejected **here** (once) rather
/// than silently dropped by one transport's trust store. Errors on empty /
/// certificate-free / malformed input.
fn parse_ca_certs(input: &[u8]) -> Result<Vec<Vec<u8>>, HttpError> {
    const PEM_MARKER: &[u8] = b"-----BEGIN ";
    let ders: Vec<Vec<u8>> = if input
        .windows(PEM_MARKER.len())
        .any(|window| window == PEM_MARKER)
    {
        let mut reader = std::io::BufReader::new(input);
        rustls_pemfile::certs(&mut reader)
            .filter_map(Result::ok)
            .map(|der| der.as_ref().to_vec())
            .collect()
    } else if input.is_empty() {
        return Err(HttpError::InvalidHeader(
            "empty CA certificate input".into(),
        ));
    } else {
        vec![input.to_vec()]
    };
    if ders.is_empty() {
        return Err(HttpError::InvalidHeader(
            "no certificate found in the CA input".into(),
        ));
    }
    if let Some(index) = ders.iter().position(|der| !looks_like_der_cert(der)) {
        return Err(HttpError::InvalidHeader(format!(
            "CA certificate #{} is not a valid DER X.509 certificate",
            index + 1
        )));
    }
    Ok(ders)
}

/// Whether `der` is structurally a DER X.509 certificate: an ASN.1 `SEQUENCE`
/// (tag `0x30`) whose definite length covers the blob exactly. A cheap check that
/// rejects obvious garbage at install time (full validation is the TLS layer's job).
fn looks_like_der_cert(der: &[u8]) -> bool {
    if der.first() != Some(&0x30) {
        return false;
    }
    let rest = &der[1..];
    let (length, header) = match rest.first() {
        Some(&first) if first < 0x80 => (first as usize, 1), // short form
        Some(&first) => {
            // Long form: low 7 bits are the count of subsequent length bytes.
            let count = (first & 0x7f) as usize;
            if count == 0 || count > 4 || rest.len() < 1 + count {
                return false;
            }
            let length = rest[1..1 + count]
                .iter()
                .fold(0usize, |acc, &byte| (acc << 8) | byte as usize);
            (length, 1 + count)
        }
        None => return false,
    };
    rest.len() == header + length
}

/// Resolves a requested [`HttpVersion`] to the version the transport will actually
/// speak, or errors when a pinned version has no wired transport.
///
/// [`Auto`](HttpVersion::Auto) negotiates: today only the HTTP/1.1 transport
/// (`ureq`) is wired, so it resolves to [`Http11`](HttpVersion::Http11); once the
/// h2/h3 transports land, `Auto` will offer their ALPN ids and adopt the server's
/// choice. [`Http11`](HttpVersion::Http11) is used as-is. A pinned
/// [`Http2`](HttpVersion::Http2) / [`Http3`](HttpVersion::Http3) whose transport is
/// not yet [`available`](HttpVersion::is_available) returns
/// [`HttpError::Unsupported`] rather than silently downgrading.
fn negotiate_version(requested: HttpVersion) -> Result<HttpVersion, HttpError> {
    let negotiated = match requested {
        HttpVersion::Auto | HttpVersion::Http11 => HttpVersion::Http11,
        available if available.is_available() => available,
        pinned => {
            log_event!(
                warn,
                "pinned http version {pinned} has no wired transport; rejecting"
            );
            return Err(HttpError::Unsupported(format!(
                "{pinned} was requested but only HTTP/1.1 is wired today; its transport \
                 is not yet implemented — use HttpVersion::Auto or Http11"
            )));
        }
    };
    log_event!(debug, "negotiated http version {requested} -> {negotiated}");
    Ok(negotiated)
}

/// A connection-pooling HTTP client, like `requests.Session`: it reuses
/// connections across requests and carries default headers applied to each.
pub struct HttpSession {
    agent: ureq::Agent,
    headers: HttpHeaders,
    retry: RetryConfig,
    max_concurrency: usize,
    batch_size: usize,
    /// The idle-connection pool size — reused (keep-alive) connections skip the
    /// TLS handshake on the next request to the same host.
    max_pool: usize,
    /// The live count of open [`HttpStream`]s (held connections), so extra streams
    /// past the pool size can drop keep-alive and not starve the pool.
    held: Arc<AtomicUsize>,
    /// The maximum number of 3xx redirect hops followed before erroring.
    max_redirects: usize,
    /// The RFC 6265 cookie jar, consulted before every dispatch and fed every
    /// response's `Set-Cookie`. Behind a [`Mutex`] since the session is shared `&self`.
    cookies: Mutex<HttpCookies>,
    /// An optional base URL: a relative request target (a path, or a bare name) is
    /// resolved against it (like `requests`'s session prefix / `httpx`'s `base_url`),
    /// while an absolute URL is used unchanged. `None` requires absolute targets.
    base_url: Option<Url>,
    /// The default HTTP protocol [`version`](HttpVersion) for requests that do not
    /// pin one ([`Auto`](HttpVersion::Auto) negotiates the best available).
    http_version: HttpVersion,
    /// Whether TLS certificate verification is performed (default `true`). When
    /// `false`, certificates are not validated (insecure) and a warning is logged —
    /// for self-signed / internal hosts only.
    verify: bool,
    /// An optional HTTP/SOCKS proxy all requests route through. Defaults to the
    /// process environment (`HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`, honouring
    /// `NO_PROXY`); override with [`with_proxy`](HttpSession::with_proxy).
    proxy: Option<ureq::Proxy>,
    /// Installed CA certificates (DER), trusted **in place of** the default store
    /// when non-empty (see [`with_ca_cert`](HttpSession::with_ca_cert)).
    ca_certs: Vec<Vec<u8>>,
    /// The read timeout: a request errors if the server sends no response (or
    /// stalls mid-body) for this long (default 2 minutes). Set with
    /// [`with_read_timeout`](HttpSession::with_read_timeout).
    read_timeout: Duration,
}

impl HttpSession {
    /// Creates a session with a pooled connection (default 16 idle connections,
    /// reused without re-doing the TLS handshake), default retry policy, a
    /// concurrency of 8 and a batch size of 80 (`max_concurrency * 10`).
    pub fn new() -> HttpSession {
        HttpSession::with_config(RetryConfig::default(), DEFAULT_POOL)
    }

    /// An independent copy of this session — same configuration (default headers,
    /// retry policy, timeouts, pool size, base URL, protocol version, TLS settings)
    /// and a snapshot of the current cookie jar, but its **own** fresh connection
    /// pool. The two sessions share nothing afterwards.
    pub fn copy(&self) -> HttpSession {
        let cookies = self.cookies.lock().expect("cookie jar poisoned").clone();
        HttpSession {
            agent: build_agent(
                self.max_pool,
                self.verify,
                self.proxy.clone(),
                &self.ca_certs,
                self.read_timeout,
            ),
            headers: self.headers.clone(),
            retry: self.retry.clone(),
            max_concurrency: self.max_concurrency,
            batch_size: self.batch_size,
            max_pool: self.max_pool,
            held: Arc::new(AtomicUsize::new(0)),
            max_redirects: self.max_redirects,
            cookies: Mutex::new(cookies),
            base_url: self.base_url.clone(),
            http_version: self.http_version,
            verify: self.verify,
            proxy: self.proxy.clone(),
            ca_certs: self.ca_certs.clone(),
            read_timeout: self.read_timeout,
        }
    }

    /// The process-wide **shared** session, created on first use and reused
    /// thereafter — the singleton that backs the module-level [`get`] / [`post`] /
    /// … convenience functions, mirroring how Python's `requests` keeps a default
    /// session. It carries the default configuration (16-connection pool, default
    /// retry policy, a shared cookie jar); reach for an explicit [`new`](HttpSession::new)
    /// when you need per-client headers, cookies or tuning.
    ///
    /// Returns a clone of the shared `Arc`, so the same pooled session (and cookie
    /// jar) is reused across calls. Replace it with [`set_shared`](HttpSession::set_shared)
    /// — e.g. to give the module-level verbs a [`base_url`](HttpSession::base_url).
    ///
    /// ```no_run
    /// use yggdryl_http::HttpSession;
    /// // Two calls return the same pooled session (and share its cookie jar).
    /// let body = HttpSession::shared().get("https://example.com").unwrap();
    /// ```
    pub fn shared() -> Arc<HttpSession> {
        HttpSession::shared_slot()
            .read()
            .expect("shared session poisoned")
            .clone()
    }

    /// Replaces the process-wide [`shared`](HttpSession::shared) singleton, so the
    /// module-level [`get`] / [`post`] / … verbs use `session` from now on — the way
    /// to give them a [`base_url`](HttpSession::with_base_url) or default headers.
    /// In-flight requests holding the previous `Arc` are unaffected.
    ///
    /// ```no_run
    /// use yggdryl_http::HttpSession;
    /// use yggdryl_core::Url;
    /// HttpSession::set_shared(
    ///     HttpSession::new().with_base_url(Url::from_str("https://api.example.com").unwrap()),
    /// );
    /// // `yggdryl_http::get("/users")` now resolves against the base URL.
    /// ```
    pub fn set_shared(session: HttpSession) {
        *HttpSession::shared_slot()
            .write()
            .expect("shared session poisoned") = Arc::new(session);
    }

    /// The storage backing [`shared`](HttpSession::shared) — a replaceable `Arc`
    /// behind an `RwLock`, initialised with a default session on first access.
    fn shared_slot() -> &'static RwLock<Arc<HttpSession>> {
        static SHARED: OnceLock<RwLock<Arc<HttpSession>>> = OnceLock::new();
        SHARED.get_or_init(|| RwLock::new(Arc::new(HttpSession::new())))
    }

    fn with_config(retry: RetryConfig, max_pool: usize) -> HttpSession {
        // Plug http/https into the yggdryl-io factory the first time a session is
        // built, so `yggdryl_core::from_str("https://…")` works once this crate links.
        crate::factory::register();
        let max_pool = max_pool.max(1);
        let verify = true;
        // Pick up a proxy from the environment by default (HTTPS_PROXY / HTTP_PROXY
        // / ALL_PROXY, honouring NO_PROXY), like `requests` / curl.
        let proxy = ureq::Proxy::try_from_env();
        let ca_certs: Vec<Vec<u8>> = Vec::new();
        let read_timeout = DEFAULT_READ_TIMEOUT;
        let agent = build_agent(max_pool, verify, proxy.clone(), &ca_certs, read_timeout);
        let max_concurrency = 8;
        HttpSession {
            agent,
            headers: HttpHeaders::new(),
            retry,
            max_concurrency,
            batch_size: max_concurrency * 10,
            max_pool,
            held: Arc::new(AtomicUsize::new(0)),
            max_redirects: DEFAULT_MAX_REDIRECTS,
            cookies: Mutex::new(HttpCookies::new()),
            base_url: None,
            http_version: HttpVersion::Auto,
            verify,
            proxy,
            ca_certs,
            read_timeout,
        }
    }

    /// Adds a default header sent with every request from this session.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> HttpSession {
        self.headers.insert(name, value);
        self
    }

    /// Sets the default `User-Agent` header.
    pub fn with_user_agent(self, user_agent: impl Into<String>) -> HttpSession {
        self.with_header("user-agent", user_agent)
    }

    /// Sets a default `Authorization` header to HTTP Basic credentials
    /// (`Basic base64(username:password)`, RFC 7617) sent with every request,
    /// like `requests`' `Session.auth = (user, pass)`. As a default header it is
    /// merged under any per-request `Authorization` and stripped on a cross-origin
    /// redirect, so credentials never leak to another host.
    pub fn with_basic_auth(mut self, username: &str, password: &str) -> HttpSession {
        self.headers.set(
            "authorization",
            crate::auth::basic_auth_header(username, password),
        );
        self
    }

    /// Sets a default `Authorization` header to an HTTP Bearer token
    /// (`Bearer <token>`, RFC 6750) sent with every request. Like
    /// [`with_basic_auth`](HttpSession::with_basic_auth) it is a default header:
    /// per-request overrides win and a cross-origin redirect strips it.
    pub fn with_bearer_auth(mut self, token: &str) -> HttpSession {
        self.headers
            .set("authorization", crate::auth::bearer_auth_header(token));
        self
    }

    /// Sets a base URL that relative request targets resolve against (see
    /// [`resolve_url`](HttpSession::resolve_url) and [`base_url`](HttpSession::base_url)).
    pub fn with_base_url(mut self, base_url: Url) -> HttpSession {
        self.base_url = Some(base_url);
        self
    }

    /// The session's base URL, if one is set.
    pub fn base_url(&self) -> Option<&Url> {
        self.base_url.as_ref()
    }

    /// Sets the default HTTP protocol [`version`](HttpVersion) for requests that do
    /// not pin one of their own ([`HttpRequest::with_http_version`]). The default is
    /// [`Auto`](HttpVersion::Auto), which negotiates the best available transport.
    /// Pinning a version with no wired transport makes [`send`](HttpSession::send)
    /// error with [`HttpError::Unsupported`] rather than silently downgrade.
    pub fn with_http_version(mut self, http_version: HttpVersion) -> HttpSession {
        self.http_version = http_version;
        self
    }

    /// The session's default HTTP protocol [`version`](HttpVersion) (used by any
    /// request that does not pin its own).
    pub fn http_version(&self) -> HttpVersion {
        self.http_version
    }

    /// Sets whether TLS certificate verification is performed (default `true`),
    /// rebuilding the agent. Passing `false` **disables** verification (and logs a
    /// warning): connections to *any* host are accepted regardless of certificate —
    /// use it only for a self-signed or internal host you trust. When verification
    /// is left on and a certificate cannot be validated, the resulting
    /// [`HttpError`] carries a hint pointing here.
    pub fn with_verify(mut self, verify: bool) -> HttpSession {
        self.verify = verify;
        self.agent = build_agent(
            self.max_pool,
            verify,
            self.proxy.clone(),
            &self.ca_certs,
            self.read_timeout,
        );
        self
    }

    /// Whether TLS certificate verification is performed.
    pub fn verify(&self) -> bool {
        self.verify
    }

    /// Routes all requests through the proxy at `url` (e.g. `http://host:8080`,
    /// `socks5://host:1080`), rebuilding the agent. Returns
    /// [`HttpError::InvalidUrl`] if the proxy URL is malformed. By default a session
    /// already adopts the environment's proxy (`HTTPS_PROXY` / `HTTP_PROXY` /
    /// `ALL_PROXY`, honouring `NO_PROXY`); call [`without_proxy`](HttpSession::without_proxy)
    /// to ignore it.
    pub fn with_proxy(mut self, url: &str) -> Result<HttpSession, HttpError> {
        let proxy = ureq::Proxy::new(url).map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
        self.proxy = Some(proxy);
        self.agent = build_agent(
            self.max_pool,
            self.verify,
            self.proxy.clone(),
            &self.ca_certs,
            self.read_timeout,
        );
        Ok(self)
    }

    /// Clears any proxy (including one picked up from the environment), so requests
    /// connect directly. Rebuilds the agent.
    pub fn without_proxy(mut self) -> HttpSession {
        self.proxy = None;
        self.agent = build_agent(
            self.max_pool,
            self.verify,
            None,
            &self.ca_certs,
            self.read_timeout,
        );
        self
    }

    /// The proxy URL all requests route through, if one is set.
    pub fn proxy(&self) -> Option<String> {
        self.proxy.as_ref().map(|proxy| proxy.uri().to_string())
    }

    /// Installs trusted CA certificate(s) from `cert` — a PEM bundle (one or more
    /// `-----BEGIN CERTIFICATE-----` blocks) or a single DER certificate —
    /// rebuilding the agent. This is the **secure** way to reach a self-signed or
    /// internal host: the server's certificate is verified against the installed CA
    /// instead of turning verification off.
    ///
    /// Installed certificates **replace** the default trust store (the Mozilla root
    /// set), matching `requests`' `verify=<bundle>`: once any CA is installed the
    /// session trusts *only* the installed ones, so install the public bundle too if
    /// the session must also reach public hosts. Calls accumulate. Applies to every
    /// transport (HTTP/1.1, HTTP/2 and HTTP/3). Returns [`HttpError::InvalidHeader`]
    /// if no certificate is found in `cert`.
    pub fn with_ca_cert(mut self, cert: &[u8]) -> Result<HttpSession, HttpError> {
        let ders = parse_ca_certs(cert)?;
        log_event!(info, "installing {} CA certificate(s)", ders.len());
        self.ca_certs.extend(ders);
        self.agent = build_agent(
            self.max_pool,
            self.verify,
            self.proxy.clone(),
            &self.ca_certs,
            self.read_timeout,
        );
        Ok(self)
    }

    /// Installs trusted CA certificate(s) read from the file at `path` (PEM or DER),
    /// the file-based form of [`with_ca_cert`](HttpSession::with_ca_cert).
    pub fn with_ca_cert_file(self, path: &str) -> Result<HttpSession, HttpError> {
        let bytes = std::fs::read(path).map_err(|err| HttpError::Io(err.into()))?;
        self.with_ca_cert(&bytes)
    }

    /// The number of CA certificates installed on this session (`0` means the
    /// default trust store is in use).
    pub fn ca_cert_count(&self) -> usize {
        self.ca_certs.len()
    }

    /// Resolves a request `target` against the session's [`base_url`](HttpSession::base_url):
    /// an absolute URL (one with a scheme) is parsed and used unchanged, while a
    /// relative reference (`/path`, `name`, `//host/p`) is joined onto the base by
    /// the same RFC 3986 rules a `Location` redirect uses. With no base URL the
    /// target must be absolute, else [`HttpError::InvalidUrl`] is returned. This is
    /// the one place the verb helpers turn a target string into a [`Url`].
    pub fn resolve_url(&self, target: &str) -> Result<Url, HttpError> {
        match &self.base_url {
            Some(base) => redirect::resolve(base, target),
            None => Url::from_str(target).map_err(|err| HttpError::InvalidUrl(err.to_string())),
        }
    }

    /// Sets the [`RetryConfig`] for transient failures.
    pub fn with_retry(mut self, retry: RetryConfig) -> HttpSession {
        self.retry = retry;
        self
    }

    /// Sets the read timeout in `seconds` (default 120 — 2 minutes), rebuilding the
    /// agent. A request errors if the server sends no response head, or stalls
    /// mid-body, for this long. Raise it for genuinely slow endpoints (a large
    /// server-side computation, a slow file generation); `0` (or negative) removes
    /// the bound entirely (a stalled server can then hang the caller indefinitely).
    pub fn with_read_timeout(mut self, seconds: f64) -> HttpSession {
        self.read_timeout = if seconds > 0.0 {
            Duration::from_secs_f64(seconds)
        } else {
            Duration::ZERO
        };
        self.agent = build_agent(
            self.max_pool,
            self.verify,
            self.proxy.clone(),
            &self.ca_certs,
            self.read_timeout,
        );
        self
    }

    /// The read timeout in seconds (`0.0` means unbounded).
    pub fn read_timeout(&self) -> f64 {
        self.read_timeout.as_secs_f64()
    }

    /// Sets the maximum number of concurrent requests in [`send_many`](HttpSession::send_many)
    /// (and resets the batch size to `max_concurrency * 10`).
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> HttpSession {
        self.max_concurrency = max_concurrency.max(1);
        self.batch_size = self.max_concurrency * 10;
        self
    }

    /// Sets the [`send_many`](HttpSession::send_many) batch size.
    pub fn with_batch_size(mut self, batch_size: usize) -> HttpSession {
        self.batch_size = batch_size.max(1);
        self
    }

    /// Sets the idle-connection pool size (rebuilding the pooled agent). Larger
    /// pools keep more keep-alive connections warm (skipping TLS handshakes).
    pub fn with_pool_size(mut self, max_pool: usize) -> HttpSession {
        let max_pool = max_pool.max(1);
        self.agent = build_agent(
            max_pool,
            self.verify,
            self.proxy.clone(),
            &self.ca_certs,
            self.read_timeout,
        );
        self.max_pool = max_pool;
        self
    }

    /// The idle-connection pool size.
    pub fn pool_size(&self) -> usize {
        self.max_pool
    }

    /// Sets the maximum number of 3xx redirect hops [`send`](HttpSession::send)
    /// follows before raising [`HttpError::TooManyRedirects`] (default `10`). A
    /// per-request opt-out is [`HttpRequest::with_allow_redirect`].
    pub fn with_max_redirects(mut self, max_redirects: usize) -> HttpSession {
        self.max_redirects = max_redirects;
        self
    }

    /// The maximum number of 3xx redirect hops followed per request.
    pub fn max_redirects(&self) -> usize {
        self.max_redirects
    }

    /// A snapshot of the session's RFC 6265 cookie jar (cloned out from behind the
    /// mutex), so a caller can inspect the stored cookies.
    pub fn cookies(&self) -> HttpCookies {
        self.cookies.lock().expect("cookie jar poisoned").clone()
    }

    /// Seeds a cookie into the session jar, scoped to `url`'s host (host-only) and
    /// path `"/"`, so it is sent on matching requests. Ignores an empty `name`.
    pub fn set_cookie(
        &self,
        url: &yggdryl_core::Url,
        name: impl Into<String>,
        value: impl Into<String>,
    ) {
        if let Some(cookie) = Cookie::new(name, value, url) {
            self.cookies
                .lock()
                .expect("cookie jar poisoned")
                .set(cookie);
        }
    }

    /// The number of [`HttpStream`]s currently holding a connection open.
    pub fn open_streams(&self) -> usize {
        self.held.load(Ordering::SeqCst)
    }

    /// The session's default headers.
    pub fn headers(&self) -> &HttpHeaders {
        &self.headers
    }

    /// The maximum number of concurrent requests in [`send_many`](HttpSession::send_many).
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// The [`send_many`](HttpSession::send_many) batch size.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// `GET url` (raises on a 4xx/5xx status). `url` is resolved against the
    /// session's [`base_url`](HttpSession::base_url) when one is set.
    pub fn get(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.send(
            HttpRequest::from_url(Method::Get, self.resolve_url(url)?),
            true,
        )
    }

    /// `HEAD url` (raises on a 4xx/5xx status).
    pub fn head(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.send(
            HttpRequest::from_url(Method::Head, self.resolve_url(url)?),
            true,
        )
    }

    /// `DELETE url` (raises on a 4xx/5xx status).
    pub fn delete(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.send(
            HttpRequest::from_url(Method::Delete, self.resolve_url(url)?),
            true,
        )
    }

    /// `POST url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn post(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.send(
            HttpRequest::from_url(Method::Post, self.resolve_url(url)?).with_body(body),
            true,
        )
    }

    /// `PUT url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn put(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.send(
            HttpRequest::from_url(Method::Put, self.resolve_url(url)?).with_body(body),
            true,
        )
    }

    /// `PATCH url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn patch(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.send(
            HttpRequest::from_url(Method::Patch, self.resolve_url(url)?).with_body(body),
            true,
        )
    }

    /// Merges the session's default headers into `request` (a per-request header
    /// overrides a session default) and returns the final request — the single
    /// place every request is assembled before sending.
    pub fn prepare(&self, request: HttpRequest) -> HttpRequest {
        let headers = self.headers.merge(&request.headers);
        HttpRequest {
            method: request.method,
            url: request.url,
            headers,
            body: request.body,
            allow_redirect: request.allow_redirect,
            keep_alive: request.keep_alive,
            http_version: request.http_version,
        }
    }

    /// **The one place every request is sent.** [`prepare`](HttpSession::prepare)s
    /// the request, runs it with the retry policy, and returns an [`HttpResponse`].
    ///
    /// `raise_error` (`true` for the verb helpers) turns a 4xx/5xx status into an
    /// [`HttpError::Status`]. Connection reuse follows the request's keep-alive idle
    /// TTL ([`keep_alive`](HttpRequest::keep_alive), default 5 minutes; `0` →
    /// `Connection: close`). As a pool safeguard, once more than
    /// [`pool_size`](HttpSession::pool_size) streams are already open, a new one
    /// drops keep-alive regardless, so streaming reads never starve the pool.
    ///
    /// The body is **always streamed**: the response holds the live
    /// [`HttpStream`], read lazily/seekably off the connection and drained on
    /// demand by [`bytes`](HttpResponse::bytes) / [`text`](HttpResponse::text) /
    /// [`into_io`](HttpResponse::into_io). `HttpStream` handles buffering and
    /// random access itself (a sliding cache, `Range` requests), so there is no
    /// separate buffered mode.
    pub fn send(&self, request: HttpRequest, raise_error: bool) -> Result<HttpResponse, HttpError> {
        let mut request = self.prepare(request);
        // `sent_at` reflects the *first* dispatch across a redirect chain.
        let mut sent_at: Option<f64> = None;
        // Loop detection: a `(method, url)` seen twice is a cycle (RFC says stop).
        let mut visited: HashSet<(Method, String)> = HashSet::new();

        for hop in 0.. {
            let allow_redirect = request.allow_redirect;
            let key = (request.method, request.url.to_string());
            if !visited.insert(key) {
                return Err(HttpError::TooManyRedirects(format!(
                    "redirect loop revisiting {} {}",
                    request.method.as_str(),
                    request.url
                )));
            }

            // Snapshot the request shape *before* the jar's Cookie is applied, so a
            // later hop re-derives the Cookie for its own host instead of resending
            // this hop's value. A user-set per-request Cookie is already in the
            // headers here and is preserved. Also capture the body's replayability
            // and a replayable copy now, before dispatch consumes the body, so a
            // 307/308 hop can preserve method + body (and refuse a consumed stream).
            let previous = HttpRequest {
                method: request.method,
                url: request.url.clone(),
                headers: request.headers.clone(),
                body: Body::Empty,
                allow_redirect,
                keep_alive: request.keep_alive,
                http_version: request.http_version,
            };
            let replayable = request.body.replayable();
            let replay_body = request.body.replay_copy();

            // Add the jar's Cookie header before dispatch (unless the request set
            // one itself), then dispatch this single hop.
            self.apply_cookies(&mut request);
            let response = self.dispatch(request)?;
            sent_at.get_or_insert(response.sent_at());
            let status = response.status();

            // Ingest any Set-Cookie before deciding the next hop.
            self.cookies
                .lock()
                .expect("cookie jar poisoned")
                .set_from_response(response.url(), response.headers());

            // Follow a redirect only when allowed, within the hop limit, and the
            // 3xx carries a Location.
            let location = response.headers().get("location").map(str::to_string);
            let should_follow = allow_redirect && redirect::is_redirect(status);
            let Some(location) = location.filter(|_| should_follow) else {
                // Final response: stamp the first dispatch and apply `raise_error`.
                return self.finalize(response, sent_at, raise_error);
            };
            if hop >= self.max_redirects {
                return Err(HttpError::TooManyRedirects(format!(
                    "exceeded max_redirects ({}) following {status}",
                    self.max_redirects
                )));
            }

            let target = redirect::resolve(&previous.url, &location)?;
            match redirect::next_request(&previous, target, status, replay_body, replayable) {
                Some(next) => {
                    // Drain/close the intermediate body to release its connection,
                    // then continue with the next hop.
                    drop(response);
                    log_event!(debug, "following {status} redirect to {}", next.url());
                    request = next;
                }
                // A 307/308 with a non-replayable (already consumed) body cannot be
                // re-sent: stop and return the 3xx itself rather than corrupt state.
                None => return self.finalize(response, sent_at, raise_error),
            }
        }
        unreachable!("the redirect loop returns or errors before exhausting usize")
    }

    /// Adds the cookie jar's `Cookie` header to `request` unless it already carries
    /// one (a per-request `Cookie` wins, like any explicit header).
    fn apply_cookies(&self, request: &mut HttpRequest) {
        if request.headers.contains("cookie") {
            return;
        }
        if let Some(value) = self
            .cookies
            .lock()
            .expect("cookie jar poisoned")
            .header_for(&request.url)
        {
            request.headers.insert("cookie", value);
        }
    }

    /// Dispatches a single hop (no redirect handling): runs the retry loop and
    /// builds the [`HttpResponse`] over a streamed or buffered body, exactly the
    /// pre-redirect [`send`](HttpSession::send) behaviour.
    fn dispatch(&self, mut request: HttpRequest) -> Result<HttpResponse, HttpError> {
        // Resolve and negotiate the protocol version up front: a request pins its
        // own, else inherits the session default. A pinned version with no wired
        // transport errors here, before any bytes leave, rather than downgrading.
        let requested = request.http_version.unwrap_or(self.http_version);
        // HTTP/2 / HTTP/3 (and Auto negotiating h2 over TLS) go through the optional
        // async transport; the buffered response then re-joins the redirect/cookie loop.
        #[cfg(any(feature = "http2", feature = "http3"))]
        if let Some(prefer) = crate::transport::route_for(requested, &request.url) {
            return self.dispatch_async(request, prefer);
        }
        let negotiated = negotiate_version(requested)?;
        let keep_alive =
            !request.keep_alive.is_zero() && self.held.load(Ordering::SeqCst) < self.max_pool;
        if keep_alive {
            // Advertise the idle TTL the caller asked for (a client hint; the local
            // pool also evicts on the agent's `max_idle_age`).
            request.headers.set(
                "keep-alive",
                format!("timeout={}", request.keep_alive.as_secs()),
            );
        } else {
            request.headers.set("connection", "close");
        }
        let url = request.url.clone();
        let method = request.method;
        log_event!(
            debug,
            "HttpSession::dispatch {} {url} keep_alive={keep_alive}",
            method.as_str()
        );
        let raw = self.execute(
            method,
            url.to_string().as_str(),
            &request.headers,
            request.body,
        )?;
        // The response headers are back: stamp the dispatch instant. Parse them
        // once here and hand the derived size / content-type to the stream.
        let sent_at = now_secs();
        let status = raw.status().as_u16();
        let response_headers = HttpHeaders::from(raw.headers());
        // A HEAD response, and 204 No Content / 304 Not Modified, carry no message
        // body even when they echo a `Content-Length` — so the body size is zero
        // regardless of the header. This keeps the short-body truncation guard in
        // `HttpStream` from firing on a legitimately empty body.
        let size = if method == Method::Head || matches!(status, 204 | 304) {
            Some(0)
        } else {
            response_headers.content_size()
        };
        let content_type = response_headers.get("content-type").map(str::to_string);
        let received_at = Instant::new();
        let http_stream = HttpStream::from_response(
            raw,
            self.agent.clone(),
            url.clone(),
            request.headers,
            self.retry.clone(),
            keep_alive,
            self.held.clone(),
            received_at.clone(),
            size,
            content_type,
        );
        // The live stream is the body: `received_at` is stamped later, when the
        // caller drains or closes it. `HttpStream` itself handles buffering and
        // random access (sliding cache, `Range` requests).
        let body: Box<dyn Io> = Box::new(http_stream);
        Ok(HttpResponse::new(
            status,
            url,
            response_headers,
            body,
            sent_at,
            received_at,
            negotiated,
        ))
    }

    /// Dispatches one hop over the optional async HTTP/2 transport, buffering the
    /// response (its body is a seekable [`BytesIO`](yggdryl_core::BytesIO)). The
    /// request body is read into memory first, so it stays replayable across the
    /// retry loop; transient statuses are retried under the session's
    /// [`RetryConfig`], the same policy the HTTP/1.1 path uses.
    #[cfg(any(feature = "http2", feature = "http3"))]
    fn dispatch_async(
        &self,
        request: HttpRequest,
        prefer: crate::version::HttpVersion,
    ) -> Result<HttpResponse, HttpError> {
        // Buffer the body once up front (the h2 path does not stream uploads yet).
        let body = match request.body {
            Body::Empty => Vec::new(),
            Body::Bytes(bytes) => bytes,
            Body::Reader(mut io) | Body::Io(mut io) => {
                let mut buffer = Vec::new();
                io.read_to_end(&mut buffer)?;
                buffer
            }
        };
        let url = request.url;
        let headers = request.headers;
        let mut attempt = 0u32;
        loop {
            let sent_at = now_secs();
            let raw = crate::transport::send(crate::transport::AsyncRequest {
                method: request.method,
                url: &url,
                headers: &headers,
                body: &body,
                prefer,
                verify: self.verify,
                ca_certs: &self.ca_certs,
            });
            match raw {
                Ok(raw) => {
                    if attempt < self.retry.max_retries
                        && self.retry.retryable_status(raw.status, attempt)
                    {
                        std::thread::sleep(self.retry.backoff(attempt, raw.headers.retry_after()));
                        attempt += 1;
                        continue;
                    }
                    let received_at = Instant::new();
                    received_at.stamp_once();
                    let body_io: Box<dyn Io> = Box::new(BytesIO::from_bytes(raw.body));
                    return Ok(HttpResponse::new(
                        raw.status,
                        url,
                        raw.headers,
                        body_io,
                        sent_at,
                        received_at,
                        raw.version,
                    ));
                }
                // The buffered body is replayable, so a *transient* transport error
                // retries up to the cap (a fresh-connection re-dispatch). Deterministic
                // failures (Unsupported / InvalidUrl / a decoded status) are returned
                // at once — retrying them only adds pointless backoff.
                Err(err) => {
                    if attempt < self.retry.max_retries && matches!(err, HttpError::Transport(_)) {
                        std::thread::sleep(self.retry.backoff(attempt, None));
                        attempt += 1;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    /// Stamps `sent_at` from the first dispatch on the final `response` and applies
    /// `raise_error` to it (the only response that error-raises).
    fn finalize(
        &self,
        mut response: HttpResponse,
        sent_at: Option<f64>,
        raise_error: bool,
    ) -> Result<HttpResponse, HttpError> {
        if let Some(sent_at) = sent_at {
            response.set_sent_at(sent_at);
        }
        let status = response.status();
        if raise_error && status >= 400 {
            // Drop closes the held connection; the error carries the status.
            return Err(HttpError::Status(status));
        }
        Ok(response)
    }

    /// Sends an iterator of requests concurrently, **streamed** in batches of
    /// [`batch_size`](HttpSession::batch_size) (each running up to
    /// [`max_concurrency`](HttpSession::max_concurrency) at a time) and yielding
    /// one [`HttpResponseBatch`] per batch. Lazy: only one batch is in flight, so
    /// an unbounded request stream uses bounded memory. Responses are returned
    /// whatever their status (transport/parse failures are `Err` entries).
    pub fn send_many<I>(&self, requests: I) -> SendMany<'_, I::IntoIter>
    where
        I: IntoIterator<Item = HttpRequest>,
    {
        SendMany {
            session: self,
            requests: requests.into_iter(),
        }
    }

    /// Runs one batch with bounded concurrency (waves of `max_concurrency`),
    /// preserving request order.
    fn run_batch(&self, batch: Vec<HttpRequest>) -> HttpResponseBatch {
        let concurrency = self.max_concurrency.max(1);
        let mut results = Vec::with_capacity(batch.len());
        let mut requests = batch.into_iter();
        loop {
            let wave: Vec<HttpRequest> = requests.by_ref().take(concurrency).collect();
            if wave.is_empty() {
                break;
            }
            let wave_results: Vec<Result<HttpResponse, HttpError>> = std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .into_iter()
                    .map(|request| scope.spawn(move || self.send(request, false)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(HttpError::Transport("worker panicked".into())))
                    })
                    .collect()
            });
            results.extend(wave_results);
        }
        HttpResponseBatch { results }
    }

    /// The retry loop shared by every send: replayable bodies (none / bytes) are
    /// retried on transient statuses and lost connections; a streamed body is
    /// single-shot.
    fn execute(
        &self,
        method: Method,
        url: &str,
        headers: &HttpHeaders,
        mut body: Body,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        let replayable = body.replayable();
        let mut attempt = 0u32;
        loop {
            let builder = self.builder(method, url, headers);
            let outcome = match &body {
                Body::Empty => self.agent.run(builder.body(ureq::SendBody::none())?),
                Body::Bytes(bytes) => self.agent.run(builder.body(bytes.clone())?),
                Body::Reader(_) | Body::Io(_) => {
                    return self.run_streamed(builder, std::mem::replace(&mut body, Body::Empty));
                }
            };
            match outcome {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if attempt < self.retry.max_retries
                        && self.retry.retryable_status(status, attempt)
                    {
                        let delay = self
                            .retry
                            .backoff(attempt, HttpHeaders::from(response.headers()).retry_after());
                        log_event!(warn, "retrying status {status} after {delay:?}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    if attempt < self.retry.max_retries && replayable {
                        let delay = self.retry.backoff(attempt, None);
                        log_event!(warn, "reconnecting after transport error: {error}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Err(error.into());
                }
            }
        }
    }

    /// Builds a request builder with `method`, `url` and all (already merged)
    /// `headers` applied.
    fn builder(
        &self,
        method: Method,
        url: &str,
        headers: &HttpHeaders,
    ) -> ureq::http::request::Builder {
        let mut builder = ureq::http::Request::builder()
            .method(method.as_str())
            .uri(url);
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
        builder
    }

    /// Sends a single-shot streamed body (reader or `Io`). An `Io` body sets
    /// `Content-Length` from its known length so the upload is framed, not chunked.
    fn run_streamed(
        &self,
        builder: ureq::http::request::Builder,
        body: Body,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        match body {
            // A plain reader is chunked (no known length); an `Io` body frames the
            // request with `Content-Length` from its `stream_len`, so a file
            // upload is never buffered. Both stream straight off the handle.
            Body::Reader(reader) => {
                let mut bridge = IoBridge(reader);
                Ok(self
                    .agent
                    .run(builder.body(ureq::SendBody::from_reader(&mut bridge))?)?)
            }
            Body::Io(io) => {
                let length = io.stream_len();
                let mut bridge = IoBridge(io);
                let builder = match length {
                    Some(length) => builder.header("content-length", length.to_string()),
                    None => builder,
                };
                Ok(self
                    .agent
                    .run(builder.body(ureq::SendBody::from_reader(&mut bridge))?)?)
            }
            Body::Empty | Body::Bytes(_) => {
                unreachable!("run_streamed called with a replayable body")
            }
        }
    }
}

impl Default for HttpSession {
    fn default() -> HttpSession {
        HttpSession::new()
    }
}

/// The lazy iterator returned by [`HttpSession::send_many`]: each `next` pulls up
/// to [`batch_size`](HttpSession::batch_size) requests and runs them concurrently.
pub struct SendMany<'a, I: Iterator<Item = HttpRequest>> {
    session: &'a HttpSession,
    requests: I,
}

impl<I: Iterator<Item = HttpRequest>> Iterator for SendMany<'_, I> {
    type Item = HttpResponseBatch;

    fn next(&mut self) -> Option<HttpResponseBatch> {
        let batch: Vec<HttpRequest> = self
            .requests
            .by_ref()
            .take(self.session.batch_size)
            .collect();
        if batch.is_empty() {
            return None;
        }
        Some(self.session.run_batch(batch))
    }
}

/// One batch of results from [`HttpSession::send_many`], in request order.
pub struct HttpResponseBatch {
    results: Vec<Result<HttpResponse, HttpError>>,
}

impl HttpResponseBatch {
    /// The number of responses in the batch.
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Consumes the batch, yielding each request's `Result` in order.
    pub fn into_results(self) -> Vec<Result<HttpResponse, HttpError>> {
        self.results
    }
}

impl IntoIterator for HttpResponseBatch {
    type Item = Result<HttpResponse, HttpError>;
    type IntoIter = std::vec::IntoIter<Result<HttpResponse, HttpError>>;

    fn into_iter(self) -> Self::IntoIter {
        self.results.into_iter()
    }
}

/// Module-level convenience verbs that dispatch through the process-wide
/// [`HttpSession::shared`] singleton, mirroring `requests.get(...)` and friends.
/// Each raises on a 4xx/5xx status, keeps the connection alive and buffers no
/// more than the streamed [`HttpResponse`] does — for per-client configuration
/// build an explicit [`HttpSession`] instead.
pub fn get(url: &str) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().get(url)
}

/// `HEAD url` via the shared session (raises on a 4xx/5xx status).
pub fn head(url: &str) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().head(url)
}

/// `DELETE url` via the shared session (raises on a 4xx/5xx status).
pub fn delete(url: &str) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().delete(url)
}

/// `POST url` with an in-memory byte body via the shared session.
pub fn post(url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().post(url, body)
}

/// `PUT url` with an in-memory byte body via the shared session.
pub fn put(url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().put(url, body)
}

/// `PATCH url` with an in-memory byte body via the shared session.
pub fn patch(url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().patch(url, body)
}

/// Sends an arbitrary [`HttpRequest`] via the shared session (streamed; connection
/// reuse follows the request's [`keep_alive`](HttpRequest::keep_alive) flag, default
/// `false`), raising on a 4xx/5xx when `raise_error`.
pub fn request(request: HttpRequest, raise_error: bool) -> Result<HttpResponse, HttpError> {
    HttpSession::shared().send(request, raise_error)
}
