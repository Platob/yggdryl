//! The connection-pooling [`HttpSession`] and the concurrent [`send_many`] support.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use yggdryl_io::{BytesIO, Io};

use crate::bridge::IoBridge;
use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
use crate::request::{Body, HttpRequest};
use crate::response::HttpResponse;
use crate::retry::{RetryConfig, DEFAULT_POOL};
use crate::stream::HttpStream;
use crate::time::{now_secs, Instant};

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
}

impl HttpSession {
    /// Creates a session with a pooled connection (default 16 idle connections,
    /// reused without re-doing the TLS handshake), default retry policy, a
    /// concurrency of 8 and a batch size of 80 (`max_concurrency * 10`).
    pub fn new() -> HttpSession {
        HttpSession::with_config(RetryConfig::default(), DEFAULT_POOL)
    }

    fn with_config(retry: RetryConfig, max_pool: usize) -> HttpSession {
        let max_pool = max_pool.max(1);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(max_pool)
            .max_idle_connections_per_host(max_pool)
            .build()
            .into();
        let max_concurrency = 8;
        HttpSession {
            agent,
            headers: HttpHeaders::new(),
            retry,
            max_concurrency,
            batch_size: max_concurrency * 10,
            max_pool,
            held: Arc::new(AtomicUsize::new(0)),
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
        self.agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(max_pool)
            .max_idle_connections_per_host(max_pool)
            .build()
            .into();
        self.max_pool = max_pool;
        self
    }

    /// The idle-connection pool size.
    pub fn pool_size(&self) -> usize {
        self.max_pool
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
    /// [`BytesIO`](yggdryl_io::BytesIO) before returning, releasing the connection
    /// immediately — the same accessors expose either body.
    pub fn send(
        &self,
        request: HttpRequest,
        raise_error: bool,
        keep_alive: bool,
        stream: bool,
    ) -> Result<HttpResponse, HttpError> {
        let mut request = self.prepare(request);
        let keep_alive = keep_alive && self.held.load(Ordering::SeqCst) < self.max_pool;
        if !keep_alive {
            request.headers.insert("connection", "close");
        }
        let url = request.url.clone();
        log_event!(
            debug,
            "HttpSession::send {} {url} keep_alive={keep_alive} stream={stream}",
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
        let response = HttpResponse::new(status, url, response_headers, body, sent_at, received_at);
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
                    if attempt < self.retry.max_retries && self.retry.retryable_status(status) {
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
