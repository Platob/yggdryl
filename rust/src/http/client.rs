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

use super::request::{AttemptHeaders, RetryOn};
use super::retry::{self, RETRY_COST, RETRY_REFUND, RetryBudget, fresh_jitter};
use super::{Headers, HttpOptions, HttpVersion, Method, Session, Status};
use crate::holder::Holder;
use crate::{Error, IOBase, IOKind, Listing, MediaType, Result, Uri, Url};

/// The most of a retried answer's body drained so its connection goes back
/// to the pool: a page of HTML is drained, and past this the connection is
/// dropped rather than read.
const FAILURE_BODY_LIMIT: u64 = 4 * 1024 * 1024;

/// Idle connections one pool keeps, over every host.
const MAX_IDLE_CONNECTIONS: usize = 256;
/// Idle connections one pool keeps to one host.
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 64;

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
/// defaults and cookies already merged in, and what the request states for
/// its own attempts ([`super::Request::wire`]).
#[derive(Clone, Copy)]
pub(crate) struct Wire<'a> {
    pub(crate) method: Method,
    pub(crate) url: &'a Url,
    pub(crate) headers: &'a Headers,
    pub(crate) body: Option<&'a [u8]>,
    /// The bound on each phase of an attempt.
    pub(crate) timeout: Duration,
    /// Whether the request may go out again after the server may have seen
    /// it: an idempotent method, or a request its caller declared so. A
    /// request that is not goes out again only when its connection was
    /// never made.
    pub(crate) idempotent: bool,
    /// The attempts the request asks for, in place of the client's.
    pub(crate) max_attempts: Option<u32>,
    /// The bound on opening a connection, in place of the pool's.
    pub(crate) connect_timeout: Option<Duration>,
    /// The bound on the whole of one attempt.
    pub(crate) deadline: Option<Duration>,
    /// Whether no proxy is gone through, whatever the client would choose.
    pub(crate) direct: bool,
    /// The headers each attempt computes over [`Self::headers`].
    pub(crate) attempt_headers: Option<&'a AttemptHeaders>,
    /// Whether an answer the status alone does not retry is retried.
    pub(crate) retry_on: Option<&'a RetryOn>,
}

/// The most of an answer's body a [`RetryOn`] rule is handed.
const RETRY_ON_PEEK: u64 = 64 * 1024;

/// What the pool this client sends on was built for.
struct Inner {
    agent: ureq::Agent,
    /// The whole-request timeout the agent was built with, so a request
    /// asking for the same one is not configured twice.
    timeout: Duration,
    stats: Stats,
    /// What is left to spend on retries.
    retries: RetryBudget,
    /// The counter every jitter draw is taken from.
    jitter: AtomicU64,
    /// Whether the proxy is the environment's, chosen for each request
    /// ([`super::proxy`]): no proxy is named and the environment is read.
    environment_proxy: bool,
    /// The proxies the environment named lately, as written and as parsed,
    /// so an unchanged variable is not parsed again for every request - one
    /// per variable a walk alternates between (`http_proxy`, `https_proxy`),
    /// at most [`PARSED_PROXIES`].
    parsed_proxies: Mutex<Vec<(String, ureq::Proxy)>>,
    /// The HTTP/2 and HTTP/3 transports, when the options leave room for
    /// them ([`super::framed`]).
    #[cfg(feature = "http2")]
    framed: Option<Arc<super::framed::Framed>>,
    /// The pool's knobs and the retry policy as the options the client was
    /// built from stated them, which a session over it may not state
    /// otherwise.
    transport: Transport,
}

/// The knobs of an [`HttpOptions`] that shape a client's pool rather than a
/// session's requests.
#[derive(Debug, PartialEq)]
struct Transport {
    connect_timeout: Duration,
    proxy: Option<String>,
    ca_bundle: Option<std::path::PathBuf>,
    read_environment: bool,
    http_version: Option<HttpVersion>,
    max_attempts: u32,
    /// The longest a `Retry-After` is waited for; a longer one ends the
    /// retries and hands the answer back.
    max_pause: Duration,
}

impl Transport {
    fn of(options: &HttpOptions) -> Self {
        Self {
            connect_timeout: options.connect_timeout(),
            proxy: options.proxy().map(str::to_owned),
            ca_bundle: options.ca_bundle().map(std::path::Path::to_path_buf),
            read_environment: options.read_environment(),
            http_version: options.http_version(),
            max_attempts: options.max_attempts(),
            max_pause: options.max_pause(),
        }
    }
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("timeout", &self.timeout)
            .field("max_attempts", &self.transport.max_attempts)
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
        Self::over(
            shared_agent().clone(),
            &HttpOptions::default(),
            #[cfg(feature = "http2")]
            super::framed::Framed::shared(),
        )
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
    /// Returns [`Error::Parse`] when the proxy URL does not parse,
    /// [`Error::Io`] when the CA bundle cannot be read or holds no
    /// certificate, or [`Error::Unsupported`] when the options ask for an
    /// HTTP version the build was made without.
    pub fn with_options(options: &HttpOptions) -> Result<Self> {
        let tls = tls_config(options)?;
        #[cfg(not(feature = "http2"))]
        if options
            .http_version()
            .is_some_and(super::HttpVersion::is_multiplexed)
        {
            return Err(Error::unsupported(
                "HTTP/2 and HTTP/3, which this build was made without (the `http2` and `http3` features)",
                "http_version",
            ));
        }
        #[cfg(feature = "http2")]
        let framed = if has_custom_transport(options)
            || options.ca_bundle().is_some()
            || options.http_version().is_some()
        {
            super::framed::Framed::for_options(options)?
        } else {
            super::framed::Framed::shared()
        };
        Ok(Self::over(
            agent_for(options, tls)?,
            options,
            #[cfg(feature = "http2")]
            framed,
        ))
    }

    /// A client sending on `agent` - and `framed`, when built with HTTP/2 -
    /// under the retry policy `options` state, with fresh counters and a
    /// full retry budget.
    fn over(
        agent: ureq::Agent,
        options: &HttpOptions,
        #[cfg(feature = "http2")] framed: Option<Arc<super::framed::Framed>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                agent,
                timeout: options.timeout(),
                stats: Stats::default(),
                retries: RetryBudget::default(),
                jitter: AtomicU64::new(fresh_jitter()),
                environment_proxy: options.proxy().is_none() && options.read_environment(),
                parsed_proxies: Mutex::new(Vec::new()),
                #[cfg(feature = "http2")]
                framed,
                transport: Transport::of(options),
            }),
        }
    }

    /// Refuse `options` for a session over this client when they state a
    /// knob of its pool or its retries otherwise than the client was built
    /// with: the session sends on the client's pool, so such a knob would be
    /// dropped. A knob left at its default states nothing and is the
    /// client's.
    pub(crate) fn check_session_options(&self, options: &HttpOptions) -> Result<()> {
        fn agree<T: PartialEq + std::fmt::Debug>(
            name: &str,
            client: &T,
            stated: &T,
            default: &T,
        ) -> Result<()> {
            if stated == default || stated == client {
                return Ok(());
            }
            Err(Error::Parse {
                target: "http option",
                position: 0,
                reason: smol_str::format_smolstr!(
                    "{name}: a session over a client sends on the client's pool, built with \
                     {client:?}; got {stated:?} - build a client with it"
                ),
            })
        }
        let (client, stated) = (&self.inner.transport, Transport::of(options));
        let default = Transport::of(&HttpOptions::default());
        agree(
            "connect_timeout",
            &client.connect_timeout,
            &stated.connect_timeout,
            &default.connect_timeout,
        )?;
        agree("proxy", &client.proxy, &stated.proxy, &default.proxy)?;
        agree(
            "ca_bundle",
            &client.ca_bundle,
            &stated.ca_bundle,
            &default.ca_bundle,
        )?;
        agree(
            "read_environment",
            &client.read_environment,
            &stated.read_environment,
            &default.read_environment,
        )?;
        agree(
            "http_version",
            &client.http_version,
            &stated.http_version,
            &default.http_version,
        )?;
        agree(
            "max_attempts",
            &client.max_attempts,
            &stated.max_attempts,
            &default.max_attempts,
        )?;
        agree(
            "max_pause",
            &client.max_pause,
            &stated.max_pause,
            &default.max_pause,
        )
    }

    /// The proxy `wire` goes through when the request or the environment
    /// decides, read now: `Some(None)` to go direct, `None` when the pool's
    /// own setting stands. A request that goes direct overrides whatever
    /// proxy the pool names.
    fn proxy_for(
        &self,
        wire: &Wire<'_>,
    ) -> std::result::Result<Option<Option<ureq::Proxy>>, Error> {
        if wire.direct {
            return Ok(self.inner.agent.config().proxy().map(|_| None));
        }
        if !self.inner.environment_proxy {
            return Ok(None);
        }
        let url = wire.url;
        let Some(named) = super::proxy::environment_proxy(url, crate::auth::variable) else {
            // Nothing to override when the pool goes direct too.
            return Ok(self.inner.agent.config().proxy().map(|_| None));
        };
        let mut parsed = self
            .inner
            .parsed_proxies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, proxy)) = parsed.iter().find(|(text, _)| *text == named) {
            return Ok(Some(Some(proxy.clone())));
        }
        let proxy = parse_proxy(
            &named,
            "http proxy",
            format_args!("the environment's proxy for {url}"),
        )?;
        if parsed.len() == PARSED_PROXIES {
            parsed.remove(0);
        }
        parsed.push((named, proxy.clone()));
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
        self.inner.transport.max_attempts
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

    /// How many times `wire` is attempted: what its request asks for, else
    /// the client's count.
    fn attempts_of(&self, wire: &Wire<'_>) -> u32 {
        wire.max_attempts
            .unwrap_or(self.inner.transport.max_attempts)
    }

    /// Whether another attempt of `wire` is allowed, and pay for it if so.
    fn may_retry(&self, wire: &Wire<'_>, attempt: u32) -> bool {
        attempt < self.attempts_of(wire) && self.inner.retries.withdraw()
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
    /// An idempotent request - by its method, or declared so - is retried
    /// per `retry` on a transport failure worth retrying, on a
    /// [`Status::is_retryable`] answer, and on any other failing answer its
    /// own rule reads as worth another attempt; any other is retried only
    /// when its connection was never made, since the server may have acted
    /// on it otherwise. A `Retry-After` - seconds or a date - is waited for
    /// up to the options' `max_pause`, and one asking longer ends the
    /// retries with that answer; a `3xx` is not followed here. The request's
    /// own attempt count stands in for the client's, and every retry is paid
    /// for out of the client's one budget. Each attempt carries the headers
    /// the request's hook computes for it. Every attempt is counted, so a
    /// test reads the true number of round trips rather than the intended
    /// one. A body is read only to drain an answer that is retried, or as
    /// far as a rule reads it; the one handed back is the caller's, whole.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] naming the method and the URL for a transport failure
    /// that was not, or could no longer be, retried; [`Error::Parse`] for a
    /// response header that will not validate; what the request's attempt
    /// headers hook returned, at once.
    pub(crate) fn execute(&self, wire: &Wire<'_>) -> Result<Answer> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let headers = attempt_headers(wire, attempt)?;
            if attempt > 1 {
                self.inner.stats.retries.fetch_add(1, Ordering::Relaxed);
            }
            let sent = Wire {
                headers: headers.as_ref().unwrap_or(wire.headers),
                ..*wire
            };
            let outcome = self.attempt(&sent, sent.body.map(Payload::Bytes));
            let mut answer = match outcome {
                Ok(answer) => answer,
                Err(Failure::Transport(error)) => {
                    if retry::is_retryable_transport(&error)
                        && (wire.idempotent || retry::is_unsent(&error))
                        && self.may_retry(wire, attempt)
                    {
                        self.pause(attempt, None);
                        continue;
                    }
                    return Err(transport_failure(wire.method, wire.url, &error));
                }
                Err(Failure::Refused(error)) => return Err(error),
            };
            if wire.idempotent && self.asks_again(wire, attempt, &mut answer) {
                continue;
            }
            self.settle(answer.status, attempt);
            answer.attempts = attempt;
            return Ok(answer);
        }
    }

    /// Whether an idempotent request's `answer` is asked for again: a status
    /// that says "not now", or a failing one the request's rule reads as
    /// that from at most [`RETRY_ON_PEEK`] bytes of the body - while the
    /// attempts, the budget and the `Retry-After` allow it, the retry paid
    /// for, the body drained and the pause waited out. An answer that is not
    /// asked for again keeps its body whole, the bytes a rule read in front.
    fn asks_again(&self, wire: &Wire<'_>, attempt: u32, answer: &mut Answer) -> bool {
        if !answer.status.is_retryable() {
            let Some(rule) = wire.retry_on else {
                return false;
            };
            if answer.status.is_success() || attempt >= self.attempts_of(wire) {
                return false;
            }
            let mut peeked = Vec::new();
            let read = (&mut answer.body)
                .take(RETRY_ON_PEEK)
                .read_to_end(&mut peeked);
            let rest = std::mem::replace(&mut answer.body, Box::new(std::io::empty()));
            if let Err(error) = read {
                // A body that failed while the rule was to read it is no
                // verdict: it is handed back failing where it failed.
                answer.body = Box::new(std::io::Cursor::new(peeked).chain(Severed {
                    error: Some(error),
                    rest,
                }));
                return false;
            }
            let retried = rule(answer.status, &answer.headers, &peeked);
            answer.body = Box::new(std::io::Cursor::new(peeked).chain(rest));
            if !retried {
                return false;
            }
        }
        let asked = retry::retry_after(answer.headers.get("retry-after"));
        let patient = asked.is_none_or(|pause| pause <= self.inner.transport.max_pause);
        if !patient || !self.may_retry(wire, attempt) {
            return false;
        }
        let _ = std::io::copy(
            &mut (&mut answer.body).take(FAILURE_BODY_LIMIT),
            &mut std::io::sink(),
        );
        self.pause(attempt, asked);
        true
    }

    /// One request whose body is streamed from `body`, `length` bytes long,
    /// with no retry: a consumed reader cannot be sent again. The one
    /// attempt carries the headers the request's hook computes for it.
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
        let headers = attempt_headers(wire, 1)?;
        let sent = Wire {
            headers: headers.as_ref().unwrap_or(wire.headers),
            ..*wire
        };
        let outcome = self.attempt(&sent, Some(Payload::Reader { body, length }));
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
        let proxy = self.proxy_for(wire).map_err(Failure::Refused)?;
        self.inner.stats.record(wire.method);
        #[cfg(feature = "http2")]
        let mut payload = payload;
        #[cfg(feature = "http2")]
        if let Some(framed) = &self.inner.framed {
            // A proxy is spoken to in HTTP/1.1: the framed versions go direct.
            let proxied = match &proxy {
                Some(chosen) => chosen.is_some(),
                None => self.inner.agent.config().proxy().is_some(),
            };
            if !proxied && let Some(exchanged) = framed.exchange(wire, &mut payload) {
                let answer = exchanged?;
                framed.learn(wire.url, &answer.headers);
                return Ok(answer);
            }
        }
        let answer = self.attempt_http1(wire, payload, proxy)?;
        #[cfg(feature = "http2")]
        if let Some(framed) = &self.inner.framed {
            framed.learn(wire.url, &answer.headers);
        }
        Ok(answer)
    }

    /// Send one attempt over HTTP/1.1 and read its head.
    fn attempt_http1(
        &self,
        wire: &Wire<'_>,
        payload: Option<Payload<'_>>,
        proxy: Option<Option<ureq::Proxy>>,
    ) -> Outcome {
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
            // A method that carries a body states its length when the body is
            // empty - `Content-Length: 0`, as curl and requests send it - so
            // no server has to read an empty chunked body to find its end.
            None if matches!(wire.method, Method::Post | Method::Put | Method::Patch) => {
                let request = builder.body(&[][..]).map_err(ureq::Error::Http)?;
                self.run(request, wire, proxy)?
            }
            None => {
                let request = builder.body(()).map_err(ureq::Error::Http)?;
                self.run(request, wire, proxy)?
            }
            Some(Payload::Bytes(bytes)) => {
                let request = builder.body(bytes).map_err(ureq::Error::Http)?;
                self.run(request, wire, proxy)?
            }
            Some(Payload::Reader { body, length }) => {
                let request = builder
                    .header("content-length", length.to_string())
                    .body(ureq::SendBody::from_reader(body))
                    .map_err(ureq::Error::Http)?;
                self.run(request, wire, proxy)?
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
        let body: Box<dyn Read + Send> = Box::new(reader);
        Ok(Answer {
            status,
            version,
            headers,
            body,
            attempts: 1,
        })
    }

    /// Run one built request on the pool, under the bounds `wire` states
    /// where they are not the pool's own: its phase timeout, its connect
    /// timeout and its deadline over the whole attempt.
    fn run<B: ureq::AsSendBody>(
        &self,
        request: ureq::http::Request<B>,
        wire: &Wire<'_>,
        proxy: Option<Option<ureq::Proxy>>,
    ) -> std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let timeout = wire.timeout;
        if timeout == self.inner.timeout
            && proxy.is_none()
            && wire.connect_timeout.is_none()
            && wire.deadline.is_none()
        {
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
        if let Some(connect) = wire.connect_timeout {
            request = request.timeout_connect(Some(connect));
        }
        if let Some(deadline) = wire.deadline {
            // From the call to the last byte of the body, every phase within.
            request = request.timeout_global(Some(deadline));
        }
        if let Some(proxy) = proxy {
            request = request.proxy(proxy);
        }
        self.inner.agent.run(request.build())
    }
}

/// A body on its way out.
pub(crate) enum Payload<'a> {
    /// Held whole, so it can go out again.
    Bytes(&'a [u8]),
    /// Read once from a caller's reader.
    Reader { body: &'a mut dyn Read, length: u64 },
}

impl Payload<'_> {
    /// The same body, lent for one attempt at a transport that may decline
    /// it before reading a byte.
    #[cfg(feature = "http2")]
    pub(crate) fn reborrow(&mut self) -> Payload<'_> {
        match self {
            Self::Bytes(bytes) => Payload::Bytes(bytes),
            Self::Reader { body, length } => Payload::Reader {
                body: &mut **body,
                length: *length,
            },
        }
    }
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

/// The headers one attempt of `wire` goes out with, when its request
/// computes some: the hook's, over the wire's own; `None` when it has no
/// hook and the wire's stand.
fn attempt_headers(wire: &Wire<'_>, attempt: u32) -> Result<Option<Headers>> {
    let Some(hook) = wire.attempt_headers else {
        return Ok(None);
    };
    hook(attempt, wire.method, wire.url)?
        .merge_with(wire.headers)
        .map(Some)
}

/// A body whose read failed while a [`RetryOn`] rule was to read it: the
/// failure once, where it happened, then whatever the rest still answers.
struct Severed {
    error: Option<std::io::Error>,
    rest: Box<dyn Read + Send>,
}

impl Read for Severed {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.error.take() {
            Some(error) => Err(error),
            None => self.rest.read(buffer),
        }
    }
}

/// A request that never reached the server, or whose connection failed.
///
/// The text names the method, the URL and the transport's own words; the
/// kind is the transport's - a refused, reset or timed-out connection as
/// such - and a failure no answer's head arrived for carries the marker
/// `is_unanswered` reads.
fn transport_failure(method: Method, url: &Url, error: &ureq::Error) -> Error {
    let message = format!("http {method} {url} failed: {error}");
    let (kind, unanswered) = match error {
        ureq::Error::Io(error) => (error.kind(), true),
        ureq::Error::Timeout(_) => (std::io::ErrorKind::TimedOut, true),
        ureq::Error::ConnectionFailed => (std::io::ErrorKind::ConnectionRefused, true),
        ureq::Error::HostNotFound => (std::io::ErrorKind::Other, true),
        // A request the client could not form, a TLS handshake or a proxy
        // that refused, a head that would not parse: something answered, or
        // nothing was sent to be answered.
        _ => (std::io::ErrorKind::Other, false),
    };
    Error::Io(if unanswered {
        std::io::Error::new(kind, Unanswered(message))
    } else {
        std::io::Error::new(kind, message)
    })
}

/// The text of a transport failure no answer's head arrived for: the marker
/// `is_unanswered` reads, displayed as the text alone.
#[derive(Debug)]
struct Unanswered(String);

impl std::fmt::Display for Unanswered {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Unanswered {}

/// Whether `error` is a transport failure in which no answer's head was
/// read: the name did not resolve, the connection was refused, unreachable
/// or timed out, the request timed out or was cut - reset, hung up on -
/// before a head arrived. That is "nothing answered", told apart from any
/// answer: a status, a head that would not read, a TLS handshake that
/// refused, a body cut after its head.
#[cfg(any(feature = "aws", feature = "internals"))]
pub(crate) fn is_unanswered(error: &Error) -> bool {
    matches!(
        error,
        Error::Io(error) if error.get_ref().is_some_and(|inner| inner.is::<Unanswered>())
    )
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
        builder = builder.proxy(Some(parse_proxy(proxy, "http option", "proxy")?));
    } else if !options.read_environment() {
        builder = builder.proxy(None);
    }
    if let Some(tls) = tls {
        builder = builder.tls_config(tls);
    }
    Ok(ureq::Agent::new_with_config(builder.build()))
}

/// How many environment proxies a client keeps parsed.
const PARSED_PROXIES: usize = 4;

/// The proxy `text` names, `context` saying whose it is.
///
/// A SOCKS proxy is refused: this build's transport speaks HTTP proxies
/// alone, and a request that went direct past the proxy it was told to use
/// would leave the network the caller meant it to go through.
fn parse_proxy(
    text: &str,
    target: &'static str,
    context: impl std::fmt::Display,
) -> Result<ureq::Proxy> {
    let proxy = ureq::Proxy::new(text).map_err(|error| Error::Parse {
        target,
        position: 0,
        reason: smol_str::format_smolstr!("{context}: expected a proxy URL, got {text:?}: {error}"),
    })?;
    match proxy.protocol() {
        ureq::ProxyProtocol::Http | ureq::ProxyProtocol::Https => Ok(proxy),
        other => Err(Error::unsupported(
            "a SOCKS proxy, which this transport does not speak",
            format_args!("{context}: {other}"),
        )),
    }
}

/// The TLS configuration the options' bundle - or the environment's, when
/// the options read it - asks for, when one is named.
fn tls_config(options: &HttpOptions) -> Result<Option<ureq::tls::TlsConfig>> {
    Ok(
        super::tls::bundle_certificates(options)?.map(|certificates| {
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::Specific(Arc::new(certificates)))
                .build()
        }),
    )
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
