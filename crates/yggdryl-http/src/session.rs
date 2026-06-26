//! The connection-pooling [`HttpSession`] and the concurrent [`send_many`] support.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use yggdryl_core::{BytesIO, Io};

#[cfg(not(feature = "http2"))]
use crate::bridge::IoBridge;
use crate::cookies::{Cookie, HttpCookies};
use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
#[cfg(not(feature = "http2"))]
use crate::protocol::HttpVersion;
use crate::redirect::{self, DEFAULT_MAX_REDIRECTS};
use crate::request::{Body, HttpRequest};
use crate::response::HttpResponse;
use crate::retry::{RetryConfig, DEFAULT_POOL};
#[cfg(not(feature = "http2"))]
use crate::stream::HttpStream;
use crate::time::{now_secs, Instant};

/// Builds the pooled `ureq` agent: statuses are surfaced (not errors), the idle
/// pool is sized to `max_pool`, and **ureq's own redirect following is disabled**
/// (`max_redirects(0)`) so the 3xx surfaces to our [`redirect`](crate::redirect)
/// layer, which owns method/body/cookie/security semantics.
fn build_agent(max_pool: usize) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_idle_connections(max_pool)
        .max_idle_connections_per_host(max_pool)
        .build()
        .into()
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
    /// Pooling HTTP/2+HTTP/1.1 client via hyper (present only with `http2` feature).
    #[cfg(feature = "http2")]
    h2: Arc<crate::h2::H2Client>,
}

impl HttpSession {
    /// Creates a session with a pooled connection (default 16 idle connections,
    /// reused without re-doing the TLS handshake), default retry policy, a
    /// concurrency of 8 and a batch size of 80 (`max_concurrency * 10`).
    pub fn new() -> HttpSession {
        HttpSession::with_config(RetryConfig::default(), DEFAULT_POOL)
    }

    fn with_config(retry: RetryConfig, max_pool: usize) -> HttpSession {
        // Plug http/https into the yggdryl-io factory the first time a session is
        // built, so `yggdryl_core::from_str("https://…")` works once this crate links.
        crate::factory::register();
        let max_pool = max_pool.max(1);
        let agent = build_agent(max_pool);
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
            #[cfg(feature = "http2")]
            h2: Arc::new(crate::h2::H2Client::new(max_pool)),
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

    /// Sets the [`RetryConfig`] for transient failures.
    pub fn with_retry(mut self, retry: RetryConfig) -> HttpSession {
        self.retry = retry;
        self
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
        self.agent = build_agent(max_pool);
        self.max_pool = max_pool;
        #[cfg(feature = "http2")]
        {
            self.h2 = Arc::new(crate::h2::H2Client::new(max_pool));
        }
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

    /// `GET url` (raises on a 4xx/5xx status).
    pub fn get(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::get(url)?, true)
    }

    /// `HEAD url` (raises on a 4xx/5xx status).
    pub fn head(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::head(url)?, true)
    }

    /// `DELETE url` (raises on a 4xx/5xx status).
    pub fn delete(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::delete(url)?, true)
    }

    /// `POST url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn post(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::post(url)?.with_body(body), true)
    }

    /// `PUT url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn put(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::put(url)?.with_body(body), true)
    }

    /// `PATCH url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn patch(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::patch(url)?.with_body(body), true)
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
        }
    }

    /// **The one place every request is sent.** [`prepare`](HttpSession::prepare)s
    /// the request, runs it with the retry policy, and returns an [`HttpResponse`].
    ///
    /// `raise_error` (`true` for the verb helpers) turns a 4xx/5xx status into an
    /// [`HttpError::Status`]. `keep_alive` keeps the connection pooled for reuse;
    /// `false` sends `Connection: close`. As a pool safeguard, once more than
    /// [`pool_size`](HttpSession::pool_size) streams are already open, a new one
    /// drops keep-alive regardless, so streaming reads never starve the pool.
    ///
    /// `stream` (`true` for the verb helpers and [`request`](HttpSession::request))
    /// keeps the live [`HttpStream`] as the body, read lazily/seekably off the
    /// connection; `false` drains the body into an in-memory
    /// [`BytesIO`](yggdryl_core::BytesIO) before returning, releasing the connection
    /// immediately — the same accessors expose either body.
    pub fn send(
        &self,
        request: HttpRequest,
        raise_error: bool,
        keep_alive: bool,
        stream: bool,
    ) -> Result<HttpResponse, HttpError> {
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
            };
            let replayable = request.body.replayable();
            let replay_body = request.body.replay_copy();

            // Add the jar's Cookie header before dispatch (unless the request set
            // one itself), then dispatch this single hop.
            self.apply_cookies(&mut request);
            let response = self.dispatch(request, keep_alive, stream)?;
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

    /// Dispatches a single hop (no redirect handling) — routes to the H2 or
    /// H1.1 transport based on the enabled feature set.
    fn dispatch(
        &self,
        request: HttpRequest,
        keep_alive: bool,
        stream: bool,
    ) -> Result<HttpResponse, HttpError> {
        #[cfg(feature = "http2")]
        return self.dispatch_h2(request, keep_alive, stream);
        #[cfg(not(feature = "http2"))]
        self.dispatch_h1(request, keep_alive, stream)
    }

    /// HTTP/1.1 dispatch via ureq: runs the retry loop and builds the
    /// [`HttpResponse`] over a streamed or buffered body.
    #[cfg(not(feature = "http2"))]
    fn dispatch_h1(
        &self,
        mut request: HttpRequest,
        keep_alive: bool,
        stream: bool,
    ) -> Result<HttpResponse, HttpError> {
        let keep_alive = keep_alive && self.held.load(Ordering::SeqCst) < self.max_pool;
        if !keep_alive {
            request.headers.set("connection", "close");
        }
        let url = request.url.clone();
        log_event!(
            debug,
            "HttpSession::dispatch_h1 {} {url} keep_alive={keep_alive} stream={stream}",
            request.method.as_str()
        );
        let raw = self.execute(
            request.method,
            url.to_string().as_str(),
            &request.headers,
            request.body,
        )?;
        // The response headers are back: stamp the dispatch instant. Parse them
        // once here and hand the derived size / content-type to the stream.
        let sent_at = now_secs();
        let status = raw.status().as_u16();
        let response_headers = HttpHeaders::from(raw.headers());
        let size = response_headers.content_size();
        let content_type = response_headers.get("content-type").map(str::to_string);
        let received_at = Instant::new();
        let mut http_stream = HttpStream::from_response(
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
        // Buffered mode drains the body now (releasing the connection and stamping
        // `received_at` via the drain); streamed mode keeps the live stream as the
        // body, stamping `received_at` later when the caller drains or closes it.
        let body: Box<dyn Io> = if stream {
            Box::new(http_stream)
        } else {
            let mut buffer = Vec::new();
            http_stream.read_to_end(&mut buffer)?;
            Box::new(BytesIO::from_bytes(buffer))
        };
        Ok(HttpResponse::new(
            status,
            url,
            response_headers,
            body,
            sent_at,
            received_at,
            HttpVersion::H1_1,
        ))
    }

    /// HTTP/2 dispatch via hyper: drains non-replayable bodies to bytes (H2
    /// has no chunked upload; the single-shot reader limitation is documented),
    /// runs the retry loop, and wraps the response in an [`H2Stream`] body.
    #[cfg(feature = "http2")]
    fn dispatch_h2(
        &self,
        request: HttpRequest,
        _keep_alive: bool,
        stream: bool,
    ) -> Result<HttpResponse, HttpError> {
        use crate::h2::H2Stream;
        use http_body_util::Full;
        use hyper::body::Bytes;

        let url = request.url.clone();
        let method_str = request.method.as_str();
        let request_headers = request.headers.clone();

        // Drain Reader/Io bodies to bytes — H2 does not chunk-encode uploads and
        // the body must be replayable for the retry loop.
        let body_bytes: Bytes = match request.body {
            Body::Empty => Bytes::new(),
            Body::Bytes(v) => Bytes::from(v),
            Body::Reader(mut r) => {
                let mut buf = Vec::new();
                r.read_to_end(&mut buf)?;
                Bytes::from(buf)
            }
            Body::Io(mut io) => {
                let mut buf = Vec::new();
                io.read_to_end(&mut buf)?;
                Bytes::from(buf)
            }
        };

        log_event!(
            debug,
            "HttpSession::dispatch_h2 {method_str} {url} stream={stream}"
        );

        let uri = url.to_string();
        let mut attempt = 0u32;
        let (response, version) = loop {
            let mut req_builder = hyper::Request::builder()
                .method(method_str)
                .uri(uri.as_str());
            for (name, value) in request_headers.iter() {
                req_builder = req_builder.header(name, value);
            }
            let req = req_builder
                .body(Full::from(body_bytes.clone()))
                .map_err(|e| HttpError::InvalidHeader(e.to_string()))?;

            match self.h2.execute(req) {
                Ok((resp, ver)) => {
                    let status = resp.status().as_u16();
                    if attempt < self.retry.max_retries
                        && self.retry.retryable_status(status, attempt)
                    {
                        // Drain the body to release the H2 stream slot, then retry.
                        let resp_headers = HttpHeaders::from(resp.headers());
                        let delay = self.retry.backoff(attempt, resp_headers.retry_after());
                        log_event!(warn, "retrying H2 status {status} after {delay:?}");
                        // Discard the body — we can't use it and must release the stream.
                        drop(resp);
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    break (resp, ver);
                }
                Err(e) => {
                    if attempt < self.retry.max_retries {
                        let delay = self.retry.backoff(attempt, None);
                        log_event!(warn, "retrying H2 transport error: {e}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Err(e);
                }
            }
        };

        let sent_at = now_secs();
        let status = response.status().as_u16();
        let response_headers = HttpHeaders::from(response.headers());
        let size = response_headers.content_size();
        let content_type = response_headers.get("content-type").map(str::to_string);
        let received_at = Instant::new();

        let mut h2_stream = H2Stream::from_response(
            response.into_body(),
            self.h2.clone(),
            url.clone(),
            request_headers,
            received_at.clone(),
            size,
            content_type,
            version,
        );

        let body: Box<dyn Io> = if stream {
            Box::new(h2_stream)
        } else {
            let mut buffer = Vec::new();
            h2_stream.read_to_end(&mut buffer)?;
            Box::new(BytesIO::from_bytes(buffer))
        };

        Ok(HttpResponse::new(
            status,
            url,
            response_headers,
            body,
            sent_at,
            received_at,
            version,
        ))
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

    /// Sends a request, raising on a 4xx/5xx when `raise_error` (a keep-alive,
    /// streamed [`send`](HttpSession::send)).
    pub fn request(
        &self,
        request: HttpRequest,
        raise_error: bool,
    ) -> Result<HttpResponse, HttpError> {
        self.send(request, raise_error, true, true)
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
                    .map(|request| scope.spawn(move || self.request(request, false)))
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
    #[cfg(not(feature = "http2"))]
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
    #[cfg(not(feature = "http2"))]
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
    #[cfg(not(feature = "http2"))]
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
