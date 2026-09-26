//! The transport: one pooled HTTP/1.1 agent, the retry loop every request
//! crosses, and the counters that state what a surface cost.
//!
//! A [`Client`] sends one attempt at a time and decides whether a failure is
//! worth another, by the rules in `retry`: a connection that never
//! established, timed out or was cut, and an answer that says "not now"
//! ([`Status::is_retryable`]) when the body can be sent again, are paid for
//! out of the retry budget and retried after a jittered pause; anything
//! else is the caller's. A redirect is never followed here - that is the
//! session's, which owns the cookies and the method rewrite - and a
//! successful body is never read here, because it belongs to whoever asked.
//!
//! The agent is shared process-wide when every transport knob is the default,
//! so a hundred handles against one origin share connections; a client whose
//! timeouts, proxy or trust roots differ gets a pool of its own.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::retry::{self, RETRY_COST, RETRY_REFUND, RetryBudget, fresh_jitter};
use super::{Headers, HttpOptions, HttpVersion, Method, Session, Status};
use crate::holder::Holder;
use crate::{Error, IOBase, IOKind, Listing, MediaType, Result, Uri, Url};

/// The most of a failing answer's body the client reads into memory, so a
/// refusal can be reported: a page of HTML is a refusal too, a gigabyte of
/// one is not worth holding.
const FAILURE_BODY_LIMIT: u64 = 4 * 1024 * 1024;

/// Idle connections one pool keeps, over every host.
const MAX_IDLE_CONNECTIONS: usize = 256;
/// Idle connections one pool keeps to one host.
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 64;

/// The environment names a CA bundle is read from, in order, when the
/// options name none and read the environment.
const CA_BUNDLE_VARIABLES: [&str; 3] = ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"];

/// The media type every container reports.
static DIRECTORY_MEDIA_TYPE: std::sync::LazyLock<MediaType> =
    std::sync::LazyLock::new(|| MediaType::from(crate::MimeType::DIRECTORY));

/// How many requests of each shape have gone out.
///
/// Counted rather than timed, because the number of round trips is what
/// this backend is designed around and what a test asserts.
#[derive(Debug, Default)]
pub(crate) struct Stats {
    requests: AtomicU64,
    gets: AtomicU64,
    heads: AtomicU64,
    posts: AtomicU64,
    puts: AtomicU64,
    patches: AtomicU64,
    deletes: AtomicU64,
    others: AtomicU64,
    retries: AtomicU64,
    resumes: AtomicU64,
    redirects: AtomicU64,
}

impl Stats {
    /// Count one request of `method`.
    fn record(&self, method: Method) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let counter = match method {
            Method::Get => &self.gets,
            Method::Head => &self.heads,
            Method::Post => &self.posts,
            Method::Put => &self.puts,
            Method::Patch => &self.patches,
            Method::Delete => &self.deletes,
            Method::Options | Method::Trace | Method::Connect => &self.others,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// A reading of a client's request counters at one instant.
///
/// ```
/// use yggdryl::http::StatsSnapshot;
///
/// // Nothing has gone out, so every count is zero.
/// assert_eq!(StatsSnapshot::default().requests, 0);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatsSnapshot {
    /// Every request, retries, resumes and redirect hops included.
    pub requests: u64,
    /// `GET` requests, whole or ranged.
    pub gets: u64,
    /// `HEAD` requests, which read metadata without bytes.
    pub heads: u64,
    /// `POST` requests.
    pub posts: u64,
    /// `PUT` requests.
    pub puts: u64,
    /// `PATCH` requests.
    pub patches: u64,
    /// `DELETE` requests.
    pub deletes: u64,
    /// `OPTIONS`, `TRACE` and `CONNECT` requests.
    pub others: u64,
    /// Attempts beyond the first of one request.
    pub retries: u64,
    /// Transfers re-opened after a body was cut.
    pub resumes: u64,
    /// Redirect hops followed.
    pub redirects: u64,
    /// What is left of the retry budget.
    ///
    /// A client that is only failing spends this down and then stops
    /// retrying, so a falling number is the sign of an origin in trouble
    /// rather than of a slow one. A [`Default`] snapshot reports zero because
    /// it describes no client at all.
    pub retry_tokens: i64,
}

/// One answer as the transport hands it over: the head read, the body
/// still on the wire.
///
/// A failing answer - any status past the redirects - has its body read
/// whole into a cursor by the client, bounded, so a refusal can name what
/// the server said; a `2xx` body is never touched here.
pub(crate) struct Answer {
    pub(crate) status: Status,
    pub(crate) version: HttpVersion,
    pub(crate) headers: Headers,
    pub(crate) body: Box<dyn Read + Send>,
    /// How many attempts this answer took.
    pub(crate) attempts: u32,
}

/// One request as it goes on the wire: the complete header set, session
/// defaults and cookies already merged in.
pub(crate) struct Wire<'a> {
    pub(crate) method: Method,
    pub(crate) url: &'a Url,
    pub(crate) headers: &'a Headers,
    pub(crate) body: Option<&'a [u8]>,
    pub(crate) timeout: Duration,
    /// Whether the request may go out again after the server may have seen
    /// it: an idempotent method. A request that is not goes out again only
    /// when its connection was never made.
    pub(crate) idempotent: bool,
}

/// What the pool this client sends on was built for.
struct Inner {
    agent: ureq::Agent,
    /// The whole-request timeout the agent was built with, so a request
    /// asking for the same one is not configured twice.
    timeout: Duration,
    max_attempts: u32,
    /// The longest a `Retry-After` is waited for; a longer one ends the
    /// retries and hands the answer back.
    max_pause: Duration,
    stats: Stats,
    /// What is left to spend on retries.
    retries: RetryBudget,
    /// The counter every jitter draw is taken from.
    jitter: AtomicU64,
    /// Whether the proxy is the environment's, chosen for each request
    /// ([`super::proxy`]): no proxy is named and the environment is read.
    environment_proxy: bool,
    /// The last proxy the environment named, as written and as parsed, so
    /// an unchanged variable is not parsed again for every request.
    parsed_proxy: Mutex<Option<(String, ureq::Proxy)>>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("timeout", &self.timeout)
            .field("max_attempts", &self.max_attempts)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

/// The pooled connections, the retry policy and the counters every session
/// shares.
///
/// Cloning a client shares its pool and its counters; building one touches
/// no network. A client whose transport knobs are all the defaults shares
/// one process-wide pool with every other such client.
///
/// The retry budget is the client's, not a destination's: retries to a host
/// that keeps failing spend the tokens retries to every other host would
/// draw on, until successes refund them. That is the bound - a client's
/// total retry load stays proportional to its successes whatever fails - and
/// a caller who wants one host's failures kept from another's gives that
/// host a client of its own.
///
/// ```
/// use yggdryl::http::Client;
///
/// let client = Client::new();
/// assert_eq!(client.stats().requests, 0);
/// assert_eq!(client.session().stats(), client.stats());
/// ```
#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// A client over the default transport: the shared process-wide pool.
    #[must_use]
    pub fn new() -> Self {
        Self::over(shared_agent().clone(), &HttpOptions::default())
    }

    /// A client over the transport `options` describe.
    ///
    /// The timeouts, the proxy, the CA bundle and whether the environment's
    /// proxy and bundle variables are read shape the pool; every other knob
    /// is a session's. A client whose pool would match the defaults shares
    /// the process-wide one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the proxy URL does not parse, or
    /// [`Error::Io`] when the CA bundle cannot be read or holds no
    /// certificate.
    pub fn with_options(options: &HttpOptions) -> Result<Self> {
        let tls = tls_config(options)?;
        Ok(Self::over(agent_for(options, tls)?, options))
    }

    /// A client sending on `agent` under the retry policy `options` state,
    /// with fresh counters and a full retry budget.
    fn over(agent: ureq::Agent, options: &HttpOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                agent,
                timeout: options.timeout(),
                max_attempts: options.max_attempts(),
                max_pause: options.max_pause(),
                stats: Stats::default(),
                retries: RetryBudget::default(),
                jitter: AtomicU64::new(fresh_jitter()),
                environment_proxy: options.proxy().is_none() && options.read_environment(),
                parsed_proxy: Mutex::new(None),
            }),
        }
    }

    /// The proxy `url` goes through when the environment decides, read now:
    /// `Some(None)` to go direct, `None` when the pool's own setting stands.
    fn proxy_for(&self, url: &Url) -> std::result::Result<Option<Option<ureq::Proxy>>, Error> {
        if !self.inner.environment_proxy {
            return Ok(None);
        }
        let Some(named) = super::proxy::environment_proxy(url, crate::auth::variable) else {
            // Nothing to override when the pool goes direct too.
            return Ok(self.inner.agent.config().proxy().map(|_| None));
        };
        let mut parsed = self
            .inner
            .parsed_proxy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((text, proxy)) = parsed.as_ref() {
            if *text == named {
                return Ok(Some(Some(proxy.clone())));
            }
        }
        let proxy = ureq::Proxy::new(&named).map_err(|error| Error::Parse {
            target: "http proxy",
            position: 0,
            reason: smol_str::format_smolstr!(
                "the environment's proxy for {url}: expected a proxy URL, got {named:?}: {error}"
            ),
        })?;
        *parsed = Some((named, proxy.clone()));
        Ok(Some(Some(proxy)))
    }

    /// Every counter, plus what is left of the retry budget.
    #[must_use]
    pub fn stats(&self) -> StatsSnapshot {
        let stats = &self.inner.stats;
        StatsSnapshot {
            requests: stats.requests.load(Ordering::Relaxed),
            gets: stats.gets.load(Ordering::Relaxed),
            heads: stats.heads.load(Ordering::Relaxed),
            posts: stats.posts.load(Ordering::Relaxed),
            puts: stats.puts.load(Ordering::Relaxed),
            patches: stats.patches.load(Ordering::Relaxed),
            deletes: stats.deletes.load(Ordering::Relaxed),
            others: stats.others.load(Ordering::Relaxed),
            retries: stats.retries.load(Ordering::Relaxed),
            resumes: stats.resumes.load(Ordering::Relaxed),
            redirects: stats.redirects.load(Ordering::Relaxed),
            retry_tokens: self.inner.retries.remaining(),
        }
    }

    /// A session over this client with the default options.
    #[must_use]
    pub fn session(&self) -> Session {
        Session::from_parts(self.clone(), HttpOptions::default())
    }

    /// How many times one request is attempted.
    pub(crate) fn max_attempts(&self) -> u32 {
        self.inner.max_attempts
    }

    /// Count one re-opened transfer.
    pub(crate) fn record_resume(&self) {
        self.inner.stats.resumes.fetch_add(1, Ordering::Relaxed);
    }

    /// Count one redirect hop.
    pub(crate) fn record_redirect(&self) {
        self.inner.stats.redirects.fetch_add(1, Ordering::Relaxed);
    }

    /// Wait before attempt `attempt + 1`: what the server asked for, else a
    /// draw from the backoff window.
    pub(crate) fn pause(&self, attempt: u32, asked: Option<Duration>) {
        std::thread::sleep(retry::delay(attempt, asked, &self.inner.jitter));
    }

    /// Whether another attempt is allowed, and pay for it if so.
    fn may_retry(&self, attempt: u32) -> bool {
        attempt < self.inner.max_attempts && self.inner.retries.withdraw()
    }

    /// Give back what an exchange that reached a verdict is owed.
    fn settle(&self, status: Status, attempt: u32) {
        if status.is_retryable() {
            return;
        }
        self.inner.retries.refund(if attempt == 1 {
            RETRY_REFUND
        } else {
            RETRY_COST
        });
    }

    /// One request with retries.
    ///
    /// An idempotent request is retried per `retry` on a transport failure
    /// worth retrying and on a [`Status::is_retryable`] answer; any other is
    /// retried only when its connection was never made, since the server
    /// may have acted on it otherwise. A `Retry-After` - seconds or a date -
    /// is waited for up to the options' `max_pause`, and one asking longer
    /// ends the retries with that answer; a `3xx` is not followed here.
    /// Every attempt is counted, so a test reads the true number of round
    /// trips rather than the intended one. A `2xx` body is never read.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] naming the method and the URL for a transport failure
    /// that was not, or could no longer be, retried; [`Error::Parse`] for a
    /// response header that will not validate.
    pub(crate) fn execute(&self, wire: &Wire<'_>) -> Result<Answer> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if attempt > 1 {
                self.inner.stats.retries.fetch_add(1, Ordering::Relaxed);
            }
            let outcome = self.attempt(wire, wire.body.map(Payload::Bytes));
            let mut answer = match outcome {
                Ok(answer) => answer,
                Err(Failure::Transport(error)) => {
                    let unsent = matches!(
                        error,
                        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound
                    );
                    if retry::is_retryable_transport(&error)
                        && (wire.idempotent || unsent)
                        && self.may_retry(attempt)
                    {
                        self.pause(attempt, None);
                        continue;
                    }
                    return Err(transport_failure(wire.method, wire.url, &error));
                }
                Err(Failure::Refused(error)) => return Err(error),
            };
            if answer.status.is_retryable() && wire.idempotent {
                let asked = retry::retry_after(answer.headers.get("retry-after"));
                let patient = asked.is_none_or(|pause| pause <= self.inner.max_pause);
                if patient && self.may_retry(attempt) {
                    self.pause(attempt, asked);
                    continue;
                }
            }
            self.settle(answer.status, attempt);
            answer.attempts = attempt;
            return Ok(answer);
        }
    }

    /// One request whose body is streamed from `body`, `length` bytes long,
    /// with no retry: a consumed reader cannot be sent again.
    ///
    /// # Errors
    ///
    /// As [`Self::execute`].
    pub(crate) fn execute_reader(
        &self,
        wire: &Wire<'_>,
        body: &mut dyn Read,
        length: u64,
    ) -> Result<Answer> {
        let outcome = self.attempt(wire, Some(Payload::Reader { body, length }));
        match outcome {
            Ok(mut answer) => {
                self.settle(answer.status, 1);
                answer.attempts = 1;
                Ok(answer)
            }
            Err(Failure::Transport(error)) => Err(transport_failure(wire.method, wire.url, &error)),
            Err(Failure::Refused(error)) => Err(error),
        }
    }

    /// Send one attempt and read its head.
    fn attempt(&self, wire: &Wire<'_>, payload: Option<Payload<'_>>) -> Outcome {
        let proxy = self.proxy_for(wire.url).map_err(Failure::Refused)?;
        self.inner.stats.record(wire.method);
        let mut builder = ureq::http::Request::builder()
            .method(wire.method.as_str())
            .uri(wire.url.to_string());
        // The transport frames the body it sends and states its length
        // itself; a length the caller stated too would be a second one.
        for (name, value) in wire.headers {
            if name != "content-length" {
                builder = builder.header(name, value);
            }
        }
        let response = match payload {
            None => {
                let request = builder.body(()).map_err(ureq::Error::Http)?;
                self.run(request, wire.timeout, proxy)?
            }
            Some(Payload::Bytes(bytes)) => {
                let request = builder.body(bytes).map_err(ureq::Error::Http)?;
                self.run(request, wire.timeout, proxy)?
            }
            Some(Payload::Reader { body, length }) => {
                let request = builder
                    .header("content-length", length.to_string())
                    .body(ureq::SendBody::from_reader(body))
                    .map_err(ureq::Error::Http)?;
                self.run(request, wire.timeout, proxy)?
            }
        };
        let status = Status::new(response.status().as_u16()).map_err(Failure::Refused)?;
        let version = if response.version() == ureq::http::Version::HTTP_10 {
            HttpVersion::Http10
        } else {
            HttpVersion::Http11
        };
        let mut headers = Headers::new();
        for (name, value) in response.headers() {
            let value = crate::Charset::Utf8.transcribe(value.as_bytes());
            headers
                .append(name.as_str(), &value)
                .map_err(Failure::Refused)?;
        }
        let reader = response
            .into_body()
            .into_with_config()
            .limit(u64::MAX)
            .reader();
        // Only a failing answer is read here, and only so its refusal can be
        // reported: a successful body belongs to the caller, untouched.
        let body: Box<dyn Read + Send> = if status.is_success() || status.is_informational() {
            Box::new(reader)
        } else {
            let mut body = Vec::new();
            let _ = reader
                .take(FAILURE_BODY_LIMIT)
                .read_to_end(&mut body)
                .map_err(ureq::Error::Io)?;
            Box::new(std::io::Cursor::new(body))
        };
        Ok(Answer {
            status,
            version,
            headers,
            body,
            attempts: 1,
        })
    }

    /// Run one built request on the pool, under `timeout` when it is not the
    /// pool's own.
    fn run<B: ureq::AsSendBody>(
        &self,
        request: ureq::http::Request<B>,
        timeout: Duration,
        proxy: Option<Option<ureq::Proxy>>,
    ) -> std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        if timeout == self.inner.timeout && proxy.is_none() {
            return self.inner.agent.run(request);
        }
        let mut request = self.inner.agent.configure_request(request);
        if timeout != self.inner.timeout {
            // Every phase the pool bounds is restated: a global bound only
            // ever shortens the pool's own phase timeouts, never lengthens
            // them.
            request = request
                .timeout_send_request(Some(timeout))
                .timeout_recv_response(Some(timeout))
                .timeout_recv_body(Some(timeout));
        }
        if let Some(proxy) = proxy {
            request = request.proxy(proxy);
        }
        self.inner.agent.run(request.build())
    }
}

/// A body on its way out.
enum Payload<'a> {
    /// Held whole, so it can go out again.
    Bytes(&'a [u8]),
    /// Read once from a caller's reader.
    Reader { body: &'a mut dyn Read, length: u64 },
}

/// Why one attempt answered nothing.
enum Failure {
    /// The transport's: worth another attempt, or not.
    Transport(ureq::Error),
    /// The answer's: a status or a header this client cannot read.
    Refused(Error),
}

impl From<ureq::Error> for Failure {
    fn from(error: ureq::Error) -> Self {
        Self::Transport(error)
    }
}

type Outcome = std::result::Result<Answer, Failure>;

/// A request that never reached the server, or whose connection failed.
fn transport_failure(method: Method, url: &Url, error: &ureq::Error) -> Error {
    Error::Io(std::io::Error::other(format!(
        "http {method} {url} failed: {error}"
    )))
}

/// The pool `options` and `tls` call for: the process-wide one when every
/// transport knob is the default and no bundle is named, else one of its
/// own. The one door every client in the crate - the S3 backend's included -
/// takes its connections through.
///
/// # Errors
///
/// [`Error::Parse`] when the proxy URL does not parse.
pub(crate) fn agent_for(
    options: &HttpOptions,
    tls: Option<ureq::tls::TlsConfig>,
) -> Result<ureq::Agent> {
    if tls.is_none() && !has_custom_transport(options) {
        return Ok(shared_agent().clone());
    }
    build_agent(options, tls)
}

/// Whether `options` ask for a pool other than the shared one.
fn has_custom_transport(options: &HttpOptions) -> bool {
    options.timeout() != HttpOptions::DEFAULT_TIMEOUT
        || options.connect_timeout() != HttpOptions::DEFAULT_CONNECT_TIMEOUT
        || options.proxy().is_some()
        || !options.read_environment()
}

/// The process-wide pool, shared by every default-configured client.
fn shared_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let options = HttpOptions::default();
        build_agent(&options, None).unwrap_or_else(|_| {
            // The defaults name no proxy, so nothing here can fail to parse;
            // the fallback is ureq's own defaults with statuses left to us.
            ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .max_redirects(0)
                    .max_redirects_will_error(false)
                    .build(),
            )
        })
    })
}

/// Build a pool for `options`.
///
/// Statuses are read, never raised, and a `3xx` is never followed: both
/// are the session's to read. The whole-request budget is applied per phase
/// rather than as one global deadline, which is free where a deadline is
/// re-checked around every read.
fn build_agent(options: &HttpOptions, tls: Option<ureq::tls::TlsConfig>) -> Result<ureq::Agent> {
    let mut builder = ureq::Agent::config_builder()
        // Idle connections kept for reuse: enough for every thread of a
        // parallel walk to one host to find its connection again, where
        // the default three would reconnect all the others per request.
        .max_idle_connections(MAX_IDLE_CONNECTIONS)
        .max_idle_connections_per_host(MAX_IDLE_CONNECTIONS_PER_HOST)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_connect(Some(options.connect_timeout()))
        .timeout_send_request(Some(options.timeout()))
        .timeout_recv_response(Some(options.timeout()))
        .timeout_recv_body(Some(options.timeout()))
        .user_agent(options.user_agent().to_owned());
    // A named proxy replaces what the environment says; a client that does
    // not read the environment names none; otherwise `.proxy` is left
    // untouched so `HTTPS_PROXY` and `NO_PROXY` keep deciding.
    if let Some(proxy) = options.proxy() {
        let proxy = ureq::Proxy::new(proxy).map_err(|error| Error::Parse {
            target: "http option",
            position: 0,
            reason: smol_str::format_smolstr!(
                "proxy: expected a proxy URL, got {proxy:?}: {error}"
            ),
        })?;
        builder = builder.proxy(Some(proxy));
    } else if !options.read_environment() {
        builder = builder.proxy(None);
    }
    if let Some(tls) = tls {
        builder = builder.tls_config(tls);
    }
    Ok(ureq::Agent::new_with_config(builder.build()))
}

/// The TLS configuration the options' bundle - or the environment's, when
/// the options read it - asks for, when one is named.
fn tls_config(options: &HttpOptions) -> Result<Option<ureq::tls::TlsConfig>> {
    let path = match options.ca_bundle() {
        Some(path) => path.to_path_buf(),
        None if options.read_environment() => {
            let Some(path) = CA_BUNDLE_VARIABLES
                .iter()
                .find_map(|name| crate::auth::variable(name))
            else {
                return Ok(None);
            };
            std::path::PathBuf::from(path)
        }
        None => return Ok(None),
    };
    let pem = std::fs::read(&path).map_err(|error| {
        Error::Io(std::io::Error::new(
            error.kind(),
            format!(
                "could not read the certificate bundle {}: {error}",
                path.display()
            ),
        ))
    })?;
    let certificates: Vec<ureq::tls::Certificate<'static>> = ureq::tls::parse_pem(&pem)
        .filter_map(|item| match item {
            Ok(ureq::tls::PemItem::Certificate(certificate)) => Some(certificate),
            _ => None,
        })
        .collect();
    if certificates.is_empty() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "the certificate bundle {} holds no certificate",
                path.display()
            ),
        )));
    }
    Ok(Some(
        ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::Specific(Arc::new(certificates)))
            .build(),
    ))
}

/// A client is a container with no location: it holds every resource an
/// absolute URL names, lists none of them, and has no bytes of its own.
impl IOBase for Client {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        Err(is_a_directory(bytes.len(), "the HTTP client"))
    }

    fn size(&self) -> u64 {
        0
    }

    fn capacity(&self) -> u64 {
        0
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        truncate_container(size, "the HTTP client")
    }

    fn uri(&self) -> Option<&Uri> {
        None
    }

    fn media_type(&self) -> &MediaType {
        &DIRECTORY_MEDIA_TYPE
    }

    /// A container's representation is what it is; nothing is recorded.
    fn set_media_type(&mut self, _media_type: MediaType) {}

    fn kind(&self) -> IOKind {
        IOKind::Directory
    }

    fn is_container(&self) -> bool {
        true
    }

    fn is_atomic(&self) -> bool {
        false
    }

    fn is_tabular(&self) -> bool {
        false
    }

    /// The resource an absolute URL names, as a `GET` of it on this
    /// client's default session.
    ///
    /// # Errors
    ///
    /// A relative path, which no client without a base URL can resolve.
    fn child_by_path(&self, path: &str) -> Result<Holder> {
        Ok(Holder::HttpRequest(self.session().get(path)?))
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        Listing::empty()
    }

    fn clear(&mut self) -> Result<()> {
        Ok(())
    }

    fn remove(&mut self, _recursive: bool) -> Result<()> {
        Ok(())
    }
}

impl crate::IOMedia for Client {
    crate::impl_default_iomedia!();
}

/// Refuse a byte write into a container, naming it.
pub(crate) fn is_a_directory(bytes: usize, what: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::IsADirectory,
        format!("expected a file to write {bytes} bytes into, got {what}"),
    ))
}

/// Truncating a container means creating it, which for one over HTTP is
/// nothing; any other size is refused.
pub(crate) fn truncate_container(size: u64, what: &str) -> Result<()> {
    if size == 0 {
        return Ok(());
    }
    Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::IsADirectory,
        format!("expected a truncation to 0 for {what}, got {size}"),
    )))
}

/// The media type every HTTP container reports.
pub(crate) fn directory_media_type() -> &'static MediaType {
    &DIRECTORY_MEDIA_TYPE
}
