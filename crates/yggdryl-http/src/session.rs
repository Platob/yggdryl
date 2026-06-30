//! [`HttpSession`] — connection-pooling, cookie-aware HTTP client.
//!
//! `HttpSession` is the main entry point. It holds shared configuration
//! (default headers, timeout, redirect limit, optional retry policy) and a
//! thread-safe cookie jar. The underlying transport is chosen once at
//! construction time and reused so HTTP/2 connection pools are shared.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use yggdryl_core::Io;

use crate::body::RequestBody;
use crate::cookie::CookieJar;
use crate::error::HttpError;
use crate::method::Method;
use crate::request::HttpRequest;
use crate::response::HttpResponse;
use crate::retry::RetryConfig;
use crate::stream::HttpStream;
use crate::transport::{self, RawResponse, SendConfig, Transport};
use crate::version::HttpVersion;

/// Configuration applied to every request sent through an `HttpSession`.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Headers merged with every request's own headers (request wins on conflict).
    pub default_headers: Vec<(String, String)>,
    /// Session-level timeout applied when the request doesn't set its own.
    pub timeout: Option<Duration>,
    /// Maximum number of redirects to follow per request.
    pub redirect_limit: usize,
    /// Optional retry policy (default: no retries).
    pub retry: Option<RetryConfig>,
    /// Default HTTP version.
    pub version: HttpVersion,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            default_headers: Vec::new(),
            timeout: None,
            redirect_limit: 10,
            retry: None,
            version: HttpVersion::Auto,
        }
    }
}

struct SessionInner {
    config: SessionConfig,
    cookies: Mutex<CookieJar>,
    transport: Arc<dyn Transport>,
}

/// A blocking, connection-pooling HTTP client session.
///
/// ```no_run
/// # fn main() -> Result<(), yggdryl_http::HttpError> {
/// use yggdryl_http::HttpSession;
///
/// let session = HttpSession::new();
/// let resp = session.get("https://example.com")?;
/// assert!(resp.ok());
/// println!("{}", resp.text()?);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct HttpSession {
    inner: Arc<SessionInner>,
}

impl HttpSession {
    /// Creates a session with defaults: HTTP/1.1, no timeout, 10 redirects, no retry.
    pub fn new() -> Self {
        HttpSession::with_config(SessionConfig::default())
    }

    /// Creates a session with a custom configuration.
    pub fn with_config(config: SessionConfig) -> Self {
        let transport = transport::for_version(config.version)
            .unwrap_or_else(|_| Box::new(crate::transport::h1::H1Transport::new()));
        HttpSession {
            inner: Arc::new(SessionInner {
                config,
                cookies: Mutex::new(CookieJar::new()),
                transport: Arc::from(transport),
            }),
        }
    }

    // ── Convenience verbs ─────────────────────────────────────────────────────

    /// Sends a `GET` request.
    pub fn get(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::get(url)?)
    }

    /// Sends a `POST` request with a raw byte body.
    pub fn post(&self, url: &str, body: &[u8]) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::post(url)?.with_body(body))
    }

    /// Sends a `PUT` request with a raw byte body.
    pub fn put(&self, url: &str, body: &[u8]) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::put(url)?.with_body(body))
    }

    /// Sends a `DELETE` request.
    pub fn delete(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::delete(url)?)
    }

    /// Sends a `PATCH` request with a raw byte body.
    pub fn patch(&self, url: &str, body: &[u8]) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::patch(url)?.with_body(body))
    }

    /// Sends a `HEAD` request.
    pub fn head(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::head(url)?)
    }

    // ── Core API ──────────────────────────────────────────────────────────────

    /// Sends `req`, following redirects and applying the session's retry policy.
    pub fn request(&self, req: HttpRequest) -> Result<HttpResponse, HttpError> {
        let cfg = &self.inner.config;
        let timeout = req.timeout.or(cfg.timeout);
        let redirect_limit = req.redirect_limit.unwrap_or(cfg.redirect_limit);
        let version = req.version.unwrap_or(cfg.version);

        let send_config = SendConfig {
            timeout,
            redirect_limit,
            version,
        };

        let url = req.url_with_params();
        let method = req.method;
        let headers = merge_headers(&cfg.default_headers, &req.headers);
        let retry = cfg.retry.clone();

        match req.body {
            Some(RequestBody::Io(io)) => {
                // Streaming bodies: send exactly once (no retry, one redirect pass).
                self.send_streaming_once(method, url, headers, io, send_config)
            }
            body => {
                let bytes = match body {
                    Some(RequestBody::Bytes(b)) => Some(b),
                    _ => None,
                };
                self.send_with_retry(method, url, headers, bytes, send_config, retry)
            }
        }
    }

    /// Opens a seekable `HttpStream` over `url` using HTTP Range requests.
    pub fn stream(&self, url: &str) -> Result<HttpStream, HttpError> {
        let cfg = &self.inner.config;
        let config = SendConfig {
            timeout: cfg.timeout,
            redirect_limit: cfg.redirect_limit,
            version: cfg.version,
        };
        Ok(HttpStream::new(
            url.to_string(),
            Arc::clone(&self.inner.transport),
            config,
        ))
    }

    /// Sends multiple requests and returns a result per request, in order.
    pub fn send_many(&self, requests: Vec<HttpRequest>) -> Vec<Result<HttpResponse, HttpError>> {
        requests.into_iter().map(|req| self.request(req)).collect()
    }

    // ── Internal dispatch ─────────────────────────────────────────────────────

    fn send_with_retry(
        &self,
        mut method: Method,
        mut url: String,
        mut headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        config: SendConfig,
        retry: Option<RetryConfig>,
    ) -> Result<HttpResponse, HttpError> {
        let max_retries = retry.as_ref().map_or(0, |r| r.max_retries);
        let mut attempt = 0u32;
        let mut redirect_count = 0usize;

        loop {
            let req_headers = self.with_cookies(&headers);

            let raw = self.inner.transport.send(
                method.to_str(),
                &url,
                &req_headers,
                body.as_deref(),
                &config,
            );

            match raw {
                // Retry transient transport failures.
                Err(_e @ HttpError::Transport(_)) | Err(_e @ HttpError::Timeout)
                    if attempt < max_retries =>
                {
                    let delay = retry.as_ref().map_or(Duration::ZERO, |r| r.delay(attempt));
                    crate::log_event!(
                        debug,
                        "retry {}/{max_retries} after {delay:?}: {_e}",
                        attempt + 1
                    );
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(e),
                Ok(raw) => {
                    self.absorb_cookies(&raw);

                    // Redirect handling.
                    if raw.status >= 300
                        && raw.status < 400
                        && redirect_count < config.redirect_limit
                    {
                        let location = raw
                            .headers
                            .iter()
                            .find(|(k, _)| k == "location")
                            .map(|(_, v)| v.clone());

                        if let Some(loc) = location {
                            redirect_count += 1;
                            let new_url = resolve_url(&url, &loc);
                            match raw.status {
                                // 301/302/303: switch to GET, drop body.
                                301..=303 => {
                                    method = Method::Get;
                                    url = new_url;
                                    headers = self.inner.config.default_headers.clone();
                                }
                                // 307/308: keep method + body.
                                307 | 308 => {
                                    url = new_url;
                                }
                                _ => {}
                            }
                            attempt = 0;
                            continue;
                        }
                    }
                    if redirect_count >= config.redirect_limit
                        && raw.status >= 300
                        && raw.status < 400
                    {
                        return Err(HttpError::Redirect {
                            limit: config.redirect_limit,
                        });
                    }

                    // Retry on 429/502/503/504.
                    if attempt < max_retries && matches!(raw.status, 429 | 502 | 503 | 504) {
                        let delay = retry_after_delay(&raw, &retry, attempt);
                        crate::log_event!(
                            debug,
                            "retry {}/{max_retries} after {delay:?}: status {}",
                            attempt + 1,
                            raw.status
                        );
                        if !delay.is_zero() {
                            std::thread::sleep(delay);
                        }
                        attempt += 1;
                        continue;
                    }

                    return Ok(raw_to_response(raw));
                }
            }
        }
    }

    fn send_streaming_once(
        &self,
        method: Method,
        url: String,
        headers: Vec<(String, String)>,
        io: Box<dyn Io + Send + 'static>,
        config: SendConfig,
    ) -> Result<HttpResponse, HttpError> {
        let len = Some(io.size());
        let reader: Box<dyn io::Read + Send + 'static> = Box::new(IoReader(io));
        let req_headers = self.with_cookies(&headers);

        let raw = self.inner.transport.send_streaming(
            method.to_str(),
            &url,
            &req_headers,
            reader,
            len,
            &config,
        )?;
        self.absorb_cookies(&raw);
        Ok(raw_to_response(raw))
    }

    fn with_cookies(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        let mut out = headers.to_vec();
        if let Ok(jar) = self.inner.cookies.lock() {
            if let Some(cookie_header) = jar.as_header_value() {
                out.push(("cookie".to_string(), cookie_header));
            }
        }
        out
    }

    fn absorb_cookies(&self, raw: &RawResponse) {
        if let Ok(mut jar) = self.inner.cookies.lock() {
            for (k, v) in &raw.headers {
                if k == "set-cookie" {
                    jar.absorb_set_cookie(v);
                }
            }
        }
    }
}

impl Default for HttpSession {
    fn default() -> Self {
        HttpSession::new()
    }
}

impl std::fmt::Debug for HttpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSession")
            .field("version", &self.inner.config.version)
            .field("timeout", &self.inner.config.timeout)
            .field("redirect_limit", &self.inner.config.redirect_limit)
            .finish_non_exhaustive()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn merge_headers(
    defaults: &[(String, String)],
    overrides: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out = defaults.to_vec();
    out.extend_from_slice(overrides);
    out
}

fn raw_to_response(raw: RawResponse) -> HttpResponse {
    let version = raw.version;
    let status = raw.status;
    let content_length = raw.content_length;
    let headers = raw.headers.clone();
    let body = raw.into_body();
    HttpResponse::new(status, headers, version, content_length, body)
}

fn retry_after_delay(raw: &RawResponse, retry: &Option<RetryConfig>, attempt: u32) -> Duration {
    if raw.status == 429 {
        if let Some((_, v)) = raw.headers.iter().find(|(k, _)| k == "retry-after") {
            if let Ok(secs) = v.parse::<u64>() {
                return Duration::from_secs(secs);
            }
        }
    }
    retry.as_ref().map_or(Duration::ZERO, |r| r.delay(attempt))
}

/// Resolves a redirect `location` (possibly relative) against the current `base` URL.
fn resolve_url(base: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    // Find the origin (scheme + host) in the base URL.
    let origin_end = base
        .find("://")
        .and_then(|i| {
            let after_scheme = &base[i + 3..];
            after_scheme.find('/').map(|j| i + 3 + j)
        })
        .unwrap_or(base.len());
    let origin = &base[..origin_end];

    if location.starts_with('/') {
        format!("{origin}{location}")
    } else {
        // Relative path: append to base directory.
        let dir_end = base[..base.len()].rfind('/').unwrap_or(origin_end);
        let dir = &base[..dir_end + 1];
        format!("{dir}{location}")
    }
}

/// Adapts a `Box<dyn Io + Send + 'static>` to `std::io::Read`.
struct IoReader(Box<dyn Io + Send + 'static>);

impl io::Read for IoReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .read_into(buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}
