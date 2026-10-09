//! A request: what goes out, and the resource its URL names.
//!
//! A [`Request`] is built once - method, URL, headers, body, the session it
//! rides on - and sent as many times as a caller likes: [`Request::send`]
//! reads the whole answer, [`Request::stream`] leaves it on the wire,
//! [`Request::pages`] walks a paginated resource. The same value is an
//! [`IOBase`] leaf over the resource, where every verb is a stated number of
//! requests (the table in [the module](super)): a positional read is one
//! ranged `GET`, a size is one `HEAD`, a positional write stages and one
//! `PUT` publishes it. A [`Body`] is held whole so a retry or a redirect can
//! send it again; a streaming upload takes [`Request::send_reader`].

use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use super::client::{Answer, Wire};
use super::wire::{RequestHead, parse_request, render_request};
use super::{
    Authorization, ContentRange, Headers, HttpVersion, Method, Pages, Pagination, Response,
    Session, StatsSnapshot, Status, Stream, range_header,
};
use crate::holder::Holder;
use crate::uri::Parameters;
use crate::{
    Error, FieldPath, IOBase, IOFile, IOKind, Listing, MediaType, MimeType, Result, Scalar, Uri,
    Url,
};

/// What a request carries.
///
/// Held whole and shared, so a retry and a redirect resend the same bytes
/// and a clone costs a reference.
///
/// ```
/// use yggdryl::http::Body;
///
/// let form = Body::form([("symbol", "MSFT"), ("as of", "2026-01-02 09:30")]);
/// assert_eq!(form.as_bytes(), b"symbol=MSFT&as+of=2026-01-02+09:30");
/// assert_eq!(Body::from("hello").len(), 5);
/// assert!(Body::Empty.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Body {
    /// No body at all.
    #[default]
    Empty,
    /// The bytes, held whole.
    Bytes(Arc<[u8]>),
}

impl Body {
    /// How many bytes the body holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Whether the body holds no byte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// The bytes, borrowed.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Empty => &[],
            Self::Bytes(bytes) => bytes,
        }
    }

    /// `value` rendered as compact JSON.
    ///
    /// # Errors
    ///
    /// A value JSON cannot represent.
    pub fn json(value: &Scalar) -> Result<Self> {
        Ok(Self::from(crate::into_json_scalar(value)?))
    }

    /// `pairs` as `application/x-www-form-urlencoded`: each key and value
    /// percent-encoded through the URI parser's encoding, a space spelled `+`.
    pub fn form<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut parameters = Parameters::new(true);
        for (key, value) in pairs {
            // A decoding view encodes what the syntax cannot carry and refuses
            // nothing, so this cannot fail.
            let _ = parameters.append(key.as_ref(), value.as_ref());
        }
        let query = parameters
            .into_query()
            .map_or_else(String::new, |query| query.replace("%20", "+"));
        Self::from(query)
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            Self::Empty
        } else {
            Self::Bytes(Arc::from(bytes))
        }
    }
}

impl From<&[u8]> for Body {
    fn from(bytes: &[u8]) -> Self {
        Self::from(bytes.to_vec())
    }
}

impl From<String> for Body {
    fn from(text: String) -> Self {
        Self::from(text.into_bytes())
    }
}

impl From<&str> for Body {
    fn from(text: &str) -> Self {
        Self::from(text.as_bytes())
    }
}

impl From<Arc<[u8]>> for Body {
    fn from(bytes: Arc<[u8]>) -> Self {
        if bytes.is_empty() {
            Self::Empty
        } else {
            Self::Bytes(bytes)
        }
    }
}

/// What the resource's metadata a `HEAD`, or a read along the way, taught.
#[derive(Clone, Copy, Debug, Default)]
struct Meta {
    /// The byte length, when the headers stated one.
    size: Option<u64>,
    /// `Last-Modified`, when stated.
    mtime: Option<i64>,
}

/// One attempt of a request: what a request's own hooks are shown of it.
///
/// [`Request::with_attempt_headers`] is shown the attempt as it is about to
/// go out, so what it makes - a proof, a signature - covers what is really
/// sent: the hop's method and URL, the headers already on it, and the body.
#[derive(Clone, Copy)]
pub struct Attempt<'a> {
    number: u32,
    method: Method,
    url: &'a Url,
    headers: &'a Headers,
    body: Option<&'a [u8]>,
    streamed: bool,
}

impl<'a> Attempt<'a> {
    /// The view of attempt `number` of `method` at `url`.
    pub(crate) const fn new(
        number: u32,
        method: Method,
        url: &'a Url,
        headers: &'a Headers,
        body: Option<&'a [u8]>,
        streamed: bool,
    ) -> Self {
        Self {
            number,
            method,
            url,
            headers,
            body,
            streamed,
        }
    }

    /// The attempt's number within one hop, from 1: a redirect hop starts
    /// again at 1.
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.number
    }

    /// The method of the hop: a redirect may have rewritten the request's.
    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }

    /// The URL of the hop, query included.
    #[must_use]
    pub const fn url(&self) -> &'a Url {
        self.url
    }

    /// The headers the attempt carries: the request's own over the
    /// session's defaults, the credential, the cookies, and whatever a hook
    /// already added.
    #[must_use]
    pub const fn headers(&self) -> &'a Headers {
        self.headers
    }

    /// The body, when its bytes are in hand: `None` for a request with no
    /// body, and for one [streamed](Self::is_streamed) from a reader.
    #[must_use]
    pub const fn body(&self) -> Option<&'a [u8]> {
        self.body
    }

    /// Whether the body is read from the caller's reader as it is sent
    /// ([`Request::send_reader`]), so nothing can read it beforehand.
    #[must_use]
    pub const fn is_streamed(&self) -> bool {
        self.streamed
    }
}

impl std::fmt::Debug for Attempt<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Header values may be credentials: the names are what is rendered.
        formatter
            .debug_struct("Attempt")
            .field("number", &self.number)
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers.len())
            .field("body_len", &self.body.map(<[u8]>::len))
            .field("streamed", &self.streamed)
            .finish()
    }
}

/// The headers one attempt of a request adds over its own, computed from the
/// attempt as it is about to go out.
pub(crate) type AttemptHeaders = dyn Fn(&Attempt<'_>) -> Result<Headers> + Send + Sync;

/// Whether a refused attempt goes out once more: the attempt as it was sent,
/// and the status, the headers and the first bytes of the body it was
/// answered with.
pub(crate) type ResendOn =
    dyn Fn(&Attempt<'_>, Status, &Headers, &[u8]) -> Result<bool> + Send + Sync;

/// Whether an answer the client would not retry by its status alone is
/// asked for again: its status, its headers and the first bytes of its body.
pub(crate) type RetryOn = dyn Fn(Status, &Headers, &[u8]) -> bool + Send + Sync;

/// The staged value and whether the server has seen it.
struct Stage {
    bytes: Vec<u8>,
    dirty: bool,
}

/// What the leaf knows and what it has not published yet.
#[derive(Default)]
struct State {
    /// The resource's metadata: `None` when unknown, `Some(None)` when known
    /// absent. Kept only while open, plus whatever a read learned along the
    /// way.
    meta: Option<Option<Meta>>,
    /// Positional writes waiting to be published.
    stage: Option<Stage>,
    /// Whether metadata learned along the way is kept.
    opened: bool,
}

/// One HTTP request, and the resource its URL names.
///
/// Build it with [`Request::get`] and the other method constructors, or
/// through a [`Session`], then shape it with the `with_` builders; nothing
/// goes out until [`Request::send`], [`Request::stream`] or an [`IOBase`]
/// verb asks.
///
/// ```no_run
/// use yggdryl::http::Request;
///
/// # fn main() -> yggdryl::Result<()> {
/// let response = Request::get("https://api.example.com/v1/orders")?
///     .with_header("Accept", "application/json")?
///     .with_query([("limit", "10")])?
///     .send()?;
/// response.raise_for_status()?;
/// let orders = response.scalar()?;
/// assert!(orders.get_key_str("data").is_some());
/// # Ok(())
/// # }
/// ```
pub struct Request {
    method: Method,
    url: Url,
    headers: Headers,
    body: Body,
    session: Session,
    authorization: Option<Authorization>,
    timeout: Option<Duration>,
    follow_redirects: Option<bool>,
    /// Whether the request may go out again after a server may have seen
    /// it, when the caller says so rather than the method.
    idempotent: Option<bool>,
    max_attempts: Option<u32>,
    connect_timeout: Option<Duration>,
    deadline: Option<Duration>,
    direct: bool,
    attempt_headers: Option<Arc<AttemptHeaders>>,
    retry_on: Option<Arc<RetryOn>>,
    resend_on: Option<Arc<ResendOn>>,
    pagination: Option<Pagination>,
    records: Option<FieldPath>,
    /// An explicit media type overrides what a response taught and what the
    /// URL infers.
    declared: Option<MediaType>,
    /// The `Content-Type` the first response taught.
    learned: OnceLock<MediaType>,
    /// Inference from the URL's compound filename, computed on demand.
    inferred: OnceLock<MediaType>,
    state: Mutex<State>,
}

impl Clone for Request {
    /// The same request, its staged writes and cached metadata left behind:
    /// what a clone shares is the resource, not this handle's view of it.
    fn clone(&self) -> Self {
        Self {
            method: self.method,
            url: self.url.clone(),
            headers: self.headers.clone(),
            body: self.body.clone(),
            session: self.session.clone(),
            authorization: self.authorization.clone(),
            timeout: self.timeout,
            follow_redirects: self.follow_redirects,
            idempotent: self.idempotent,
            max_attempts: self.max_attempts,
            connect_timeout: self.connect_timeout,
            deadline: self.deadline,
            direct: self.direct,
            attempt_headers: self.attempt_headers.clone(),
            retry_on: self.retry_on.clone(),
            resend_on: self.resend_on.clone(),
            pagination: self.pagination.clone(),
            records: self.records.clone(),
            declared: self.declared.clone(),
            learned: self.learned.clone(),
            inferred: self.inferred.clone(),
            state: Mutex::new(State::default()),
        }
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Request");
        debug
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len());
        // A hook is code: it is named, never rendered.
        if self.attempt_headers.is_some() {
            debug.field("attempt_headers", &format_args!("<attempt headers>"));
        }
        if self.retry_on.is_some() {
            debug.field("retry_on", &format_args!("<retry rule>"));
        }
        if self.resend_on.is_some() {
            debug.field("resend_on", &format_args!("<resend rule>"));
        }
        debug.finish_non_exhaustive()
    }
}

impl Request {
    /// A request of `method` at `url`, bound to the process-wide default
    /// session.
    #[must_use]
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: Headers::new(),
            body: Body::Empty,
            session: super::session(),
            authorization: None,
            timeout: None,
            follow_redirects: None,
            idempotent: None,
            max_attempts: None,
            connect_timeout: None,
            deadline: None,
            direct: false,
            attempt_headers: None,
            retry_on: None,
            resend_on: None,
            pagination: None,
            records: None,
            declared: None,
            learned: OnceLock::new(),
            inferred: OnceLock::new(),
            state: Mutex::new(State::default()),
        }
    }

    /// A `GET` of `url` on the default session.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] with target `http url` for a URL of another scheme
    /// or text that is not an absolute URL.
    pub fn get(url: &str) -> Result<Self> {
        super::session().get(url)
    }

    /// A `HEAD` of `url` on the default session.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub fn head(url: &str) -> Result<Self> {
        super::session().head(url)
    }

    /// A `POST` of `body` to `url` on the default session.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub fn post(url: &str, body: impl Into<Body>) -> Result<Self> {
        super::session().post(url, body)
    }

    /// A `PUT` of `body` to `url` on the default session.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub fn put(url: &str, body: impl Into<Body>) -> Result<Self> {
        super::session().put(url, body)
    }

    /// A `PATCH` of `body` to `url` on the default session.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub fn patch(url: &str, body: impl Into<Body>) -> Result<Self> {
        super::session().patch(url, body)
    }

    /// A `DELETE` of `url` on the default session.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub fn delete(url: &str) -> Result<Self> {
        super::session().delete(url)
    }

    /// Parse one request message: the request line, the field lines and the
    /// framed body, the target and the `Host` header forming the URL.
    ///
    /// An absolute-form target is the URL; an origin-form target joins onto
    /// `http://` and the `Host` header, which then leaves the headers, since
    /// the transport writes its own. The request is bound to the default
    /// session.
    ///
    /// ```
    /// use yggdryl::http::{Method, Request};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let wire = b"POST /rows?limit=2 HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\n\r\nhello";
    /// let request = Request::from_bytes(wire)?;
    /// assert_eq!(request.method(), Method::Post);
    /// assert_eq!(request.url().to_string(), "http://example.com/rows?limit=2");
    /// assert_eq!(request.body().as_bytes(), b"hello");
    /// // Rendered back, the field names are lower case in lexical order.
    /// assert_eq!(
    ///     request.into_bytes()?,
    ///     b"POST /rows?limit=2 HTTP/1.1\r\ncontent-length: 5\r\nhost: example.com\r\n\r\nhello".to_vec()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] with target `http message` for a malformed message,
    /// or one whose origin-form target has no `Host` to join onto.
    pub fn from_bytes(wire: &[u8]) -> Result<Self> {
        let (head, body) = parse_request(wire)?;
        let RequestHead {
            method,
            target,
            mut headers,
            ..
        } = head;
        let url = if super::session::is_absolute_reference(&target) {
            super::session::http_url(&target)?
        } else {
            let Some(host) = headers.remove("host") else {
                return Err(Error::Parse {
                    target: "http message",
                    position: 0,
                    reason: smol_str::format_smolstr!(
                        "expected a Host header to join the target {target:?} onto"
                    ),
                });
            };
            let path = if target == "*" { "/" } else { target.as_str() };
            super::session::http_url(&format!("http://{host}{path}"))?
        };
        let mut request = Self::new(method, url);
        request.headers = headers;
        request.body = Body::from(body);
        Ok(request)
    }

    /// Render the request message: the request line in origin form, the
    /// headers with a `Host` when they carry none, the framed body.
    ///
    /// # Errors
    ///
    /// A URL whose query will not read, or a `Host` that will not validate.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        let mut target = self.url.path_text(false)?.into_owned();
        if target.is_empty() {
            target.push('/');
        }
        if let Some(query) = self.url.query(false)? {
            target.push('?');
            target.push_str(&query);
        }
        let mut headers = self.headers.clone();
        if !headers.contains_key("host") {
            let host = host_header(&self.url);
            headers.insert("host", &host)?;
        }
        let head = RequestHead {
            method: self.method,
            target,
            version: HttpVersion::Http11,
            headers,
        };
        Ok(render_request(&head, self.body.as_bytes()))
    }

    /// Bind the request to `session`.
    #[must_use]
    pub fn with_session(mut self, session: Session) -> Self {
        self.session = session;
        self
    }

    /// Set one header, replacing the value the name had.
    ///
    /// # Errors
    ///
    /// As [`Headers::insert`].
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self> {
        self.headers.insert(name, value)?;
        Ok(self)
    }

    /// Merge `headers` under the request's own, which win.
    #[must_use]
    pub fn with_headers(mut self, headers: Headers) -> Self {
        // Both sides passed validation, so the merge cannot fail.
        if let Ok(merged) = self.headers.merge_with(&headers) {
            self.headers = merged;
        }
        self
    }

    /// Append `pairs` to the URL's query.
    ///
    /// # Errors
    ///
    /// A query the URL cannot carry.
    pub fn with_query<I, K, V>(mut self, pairs: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut parameters = self.url.parameters(true)?.into_owned();
        for (key, value) in pairs {
            parameters.append(key.as_ref(), value.as_ref())?;
        }
        self.url.set_parameters(&parameters)?;
        Ok(self)
    }

    /// The body, as it is.
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// `value` as a compact JSON body, `Content-Type: application/json`.
    ///
    /// # Errors
    ///
    /// As [`Body::json`].
    pub fn with_json(mut self, value: &Scalar) -> Result<Self> {
        self.body = Body::json(value)?;
        self.headers.insert("content-type", "application/json")?;
        Ok(self)
    }

    /// `pairs` as a form body, `Content-Type: application/x-www-form-urlencoded`.
    #[must_use]
    pub fn with_form<I, K, V>(mut self, pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.body = Body::form(pairs);
        // A constant that validates.
        let _ = self
            .headers
            .insert("content-type", "application/x-www-form-urlencoded");
        self
    }

    /// The credential this request sends, whatever the session's is.
    #[must_use]
    pub fn with_authorization(mut self, authorization: Authorization) -> Self {
        self.authorization = Some(authorization);
        self
    }

    /// The whole-request timeout, in place of the session's.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Whether a `3xx` is followed, in place of the session's answer.
    #[must_use]
    pub fn with_follow_redirects(mut self, follow: bool) -> Self {
        self.follow_redirects = Some(follow);
        self
    }

    /// Whether the request can do no harm twice, in place of what its
    /// method says.
    ///
    /// The caller attests it: a `POST` whose service documents it idempotent,
    /// such as an OAuth token refresh within its validity or a poll, is
    /// retried after the server may have seen it, as a `GET` is; `false`
    /// keeps a `GET` from going out twice. The retry budget, the attempts and the
    /// `Retry-After` rules are the ones every retry reads, and a request
    /// sent with [`Self::send_reader`] is never retried whatever this says.
    ///
    /// ```
    /// use yggdryl::http::Request;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let refresh = Request::post("https://oauth.example.com/token", "grant_type=refresh_token")?
    ///     .with_idempotent(true);
    /// assert_eq!(refresh.idempotent(), Some(true));
    /// assert_eq!(Request::get("https://example.com/")?.idempotent(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = Some(idempotent);
        self
    }

    /// Headers computed for each attempt, merged over everything else the
    /// attempt carries - the request's own headers, the session's, the
    /// credential - a name both state taking the hook's value.
    ///
    /// `headers` is called at the top of every attempt with the [`Attempt`]
    /// as it is about to go out - its number from 1, the method and URL of
    /// the hop it goes to, the headers already on it and the body - so a
    /// value that must be fresh per attempt, or must cover what is sent - a
    /// DPoP proof with its own `jti` and `iat`, a signature over the instant
    /// and the payload - is made for each one. A body streamed by
    /// [`Self::send_reader`] cannot be read beforehand: the attempt says so
    /// ([`Attempt::is_streamed`]) and shows none. An error the hook returns
    /// is the request's error, and is never retried.
    ///
    /// What it makes is a credential for the origin the request names: a
    /// redirect followed inside that origin calls it for the hop, and one
    /// followed to another origin does not, so nothing it would make is
    /// sent there - as the `Authorization` the caller stated is not.
    ///
    /// ```
    /// use yggdryl::http::{Headers, Request};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let request =
    ///     Request::get("https://api.example.com/v1/orders")?.with_attempt_headers(|attempt| {
    ///         let mut headers = Headers::new();
    ///         let sent = format!(
    ///             "{} {} {} {}",
    ///             attempt.number(),
    ///             attempt.method(),
    ///             attempt.url(),
    ///             attempt.body().map_or(0, <[u8]>::len),
    ///         );
    ///         headers.insert("x-attempt", &sent)?;
    ///         Ok(headers)
    ///     });
    /// assert!(format!("{request:?}").contains("<attempt headers>"));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_attempt_headers(
        mut self,
        headers: impl Fn(&Attempt<'_>) -> Result<Headers> + Send + Sync + 'static,
    ) -> Self {
        self.attempt_headers = Some(Arc::new(headers));
        self
    }

    /// Whether an attempt the server refused goes out once more, whatever
    /// its method.
    ///
    /// A `4xx` answer hands `rule` the attempt as it was sent - the headers
    /// its hook made included - and the status, the headers and at most
    /// 64 KiB of the body it was answered with. `true` says the refusal
    /// proves the server did nothing and another attempt would go out
    /// differently - signed by another key, say - so the request is sent
    /// again at once: no pause, nothing drawn from the retry budget, and at
    /// most once per hop. That is what separates it from
    /// [`Self::with_retry_on`], which asks the same request again later and
    /// only when it is idempotent. An error the rule returns is the
    /// request's; an answer not sent again is handed back whole. A body
    /// streamed by [`Self::send_reader`] is never sent again.
    // The rule's one writer is `Request::with_sigv4`, so without `aws` the
    // method would be dead code and the field stays `None`.
    #[cfg(feature = "aws")]
    #[must_use]
    pub(crate) fn with_resend_on(
        mut self,
        rule: impl Fn(&Attempt<'_>, Status, &Headers, &[u8]) -> Result<bool> + Send + Sync + 'static,
    ) -> Self {
        self.resend_on = Some(Arc::new(rule));
        self
    }

    /// How many times this request is attempted, in place of the client's
    /// `max_attempts`; zero is one.
    ///
    /// Every retry beyond the first attempt is still paid for out of the
    /// client's one retry budget, so a request asking for more attempts than
    /// the client grants others cannot retry past what the client's other
    /// requests have left it.
    #[must_use]
    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = Some(attempts.max(1));
        self
    }

    /// The bound on establishing this request's connection - the socket,
    /// and the TLS handshake over it - in place of the pool's.
    ///
    /// A connection already open in the pool is reused and waits for
    /// nothing; the bound is spent only where one is opened.
    #[must_use]
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// One bound on the whole of one attempt: resolving, connecting,
    /// sending, the answer's head and its body together, beside the
    /// per-phase bound [`Self::with_timeout`] sets.
    ///
    /// An attempt that runs out is a timeout the retry rules read as any
    /// other, so an idempotent request is attempted again under a fresh
    /// deadline; a body read past it fails as a cut transfer.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Ask `rule` whether an answer is worth another attempt when its status
    /// alone does not say so.
    ///
    /// For an idempotent request - declared with [`Self::with_idempotent`]
    /// or by its method - whose answer is not a success and not a status the
    /// client retries anyway ([`Status::is_retryable`]), the client reads at
    /// most 64 KiB of the body and hands `rule` the status, the headers and
    /// those bytes; `true` retries under the budget, the attempts, the
    /// backoff and the `Retry-After` rules every retry reads. An answer not
    /// retried is handed back with its body whole, the bytes the rule read
    /// in front of the rest. A service that answers throttling as `400` with
    /// a code in its body - AWS STS's `Throttling` - is read this way.
    ///
    /// ```
    /// use yggdryl::http::Request;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let request = Request::post("https://sts.amazonaws.com/", "Action=GetCallerIdentity")?
    ///     .with_idempotent(true)
    ///     .with_retry_on(|status, _headers, body| {
    ///         status.code() == 400 && body.windows(10).any(|code| code == b"Throttling")
    ///     });
    /// assert!(format!("{request:?}").contains("<retry rule>"));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_retry_on(
        mut self,
        rule: impl Fn(Status, &Headers, &[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.retry_on = Some(Arc::new(rule));
        self
    }

    /// Whether this request goes to its server directly, never through a
    /// proxy, whatever the options or the environment name: a link-local
    /// metadata endpoint is reached from the host itself, and a proxy
    /// between would answer for another.
    #[must_use]
    pub fn with_direct(mut self, direct: bool) -> Self {
        self.direct = direct;
        self
    }

    /// How the next page is found, in place of the session's.
    #[must_use]
    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }

    /// Where a page's rows are in its document.
    #[must_use]
    pub fn with_records(mut self, path: FieldPath) -> Self {
        self.records = Some(path);
        self
    }

    /// Declare what the resource is, whatever a response says.
    #[must_use]
    pub fn with_media_type(mut self, media_type: MediaType) -> Self {
        self.declared = Some(media_type);
        self
    }

    /// The request method.
    #[must_use]
    pub fn method(&self) -> Method {
        self.method
    }

    /// The URL, query included.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The request's own headers, before the session's defaults join them.
    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// The body.
    #[must_use]
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// The session the request goes out on.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The credential this request names, when it names one.
    pub(crate) fn authorization(&self) -> Option<&Authorization> {
        self.authorization.as_ref()
    }

    /// The request's own timeout, when it has one.
    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// The request's own redirect answer, when it has one.
    pub(crate) fn follow_redirects(&self) -> Option<bool> {
        self.follow_redirects
    }

    /// Whether the caller declared the request idempotent, or not; `None`
    /// when its method decides.
    #[must_use]
    pub fn idempotent(&self) -> Option<bool> {
        self.idempotent
    }

    /// How many times this request is attempted, when it says rather than
    /// the client.
    #[must_use]
    pub fn max_attempts(&self) -> Option<u32> {
        self.max_attempts
    }

    /// The bound on establishing this request's connection, when it states
    /// one rather than the pool.
    #[must_use]
    pub fn connect_timeout(&self) -> Option<Duration> {
        self.connect_timeout
    }

    /// The bound on the whole of one attempt, when the request states one.
    #[must_use]
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// Whether this request never goes through a proxy.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        self.direct
    }

    /// One hop of this request as it goes on the wire: `method` at `url`
    /// with the complete `headers` and `body`, each phase bounded by
    /// `timeout`, under the attempts, bounds, proxy choice and hooks the
    /// request states for itself.
    pub(crate) fn wire<'a>(
        &'a self,
        method: Method,
        url: &'a Url,
        headers: &'a Headers,
        body: Option<&'a [u8]>,
        timeout: Duration,
    ) -> Wire<'a> {
        Wire {
            method,
            url,
            headers,
            body,
            timeout,
            idempotent: self.idempotent.unwrap_or(method.is_idempotent()),
            max_attempts: self.max_attempts,
            connect_timeout: self.connect_timeout,
            deadline: self.deadline,
            direct: self.direct,
            attempt_headers: self.attempt_headers.as_deref(),
            retry_on: self.retry_on.as_deref(),
            resend_on: self.resend_on.as_deref(),
        }
    }

    /// Send, reading the whole body into memory, bounded by the session's
    /// `max_body_size`.
    ///
    /// # Errors
    ///
    /// As [`Session::send`].
    pub fn send(&self) -> Result<Response> {
        self.session.send(self)
    }

    /// Send, leaving the body on the wire as a [`Stream`].
    ///
    /// # Errors
    ///
    /// As [`Session::stream`].
    pub fn stream(&self) -> Result<Response> {
        self.session.stream(self)
    }

    /// Send with the body streamed from `reader`, `length` bytes long.
    ///
    /// A reader cannot be read twice, so nothing is retried - whatever
    /// [`Self::with_idempotent`] says - and no redirect is followed: the
    /// answer is the first server's.
    ///
    /// # Errors
    ///
    /// As [`Session::send`], every transport failure final.
    pub fn send_reader(&self, reader: &mut dyn Read, length: u64) -> Result<Response> {
        let started = std::time::Instant::now();
        let headers = self.session.headers_for(self, &self.url, false)?;
        let mut wire = self.wire(
            self.method,
            &self.url,
            &headers,
            None,
            self.timeout.unwrap_or(self.session.options().timeout()),
        );
        wire.idempotent = false;
        let answer = self
            .session
            .client_ref()
            .execute_reader(&wire, reader, length)?;
        Response::from_answer(
            self.clone(),
            answer,
            self.url.clone(),
            Vec::new(),
            started.elapsed(),
            false,
        )
    }

    /// Walk the pages of this resource per its pagination.
    #[must_use]
    pub fn pages(&self) -> Pages {
        Pages::new(self.session.clone(), self.clone())
    }

    /// The session's client counters.
    #[must_use]
    pub fn stats(&self) -> StatsSnapshot {
        self.session.stats()
    }

    /// Send with a `Range` and, when given, an `If-Range`: the door the
    /// stream and the leaf use. `last` `None` is to the end. Answers the
    /// final hop's answer and URL after redirects.
    pub(crate) fn exchange_range(
        &self,
        start: u64,
        last: Option<u64>,
        if_range: Option<&str>,
    ) -> Result<(Answer, Url)> {
        let mut extra = Headers::new();
        extra.insert("range", &range_header(start, last))?;
        if let Some(validator) = if_range {
            extra.insert("if-range", validator)?;
        }
        let (answer, url, _history, _elapsed) = self.session.exchange(self, &extra, true)?;
        Ok((answer, url))
    }

    /// The same request at another URL.
    pub(crate) fn with_url(&self, url: Url) -> Self {
        let mut request = self.clone();
        request.url = url;
        request
    }

    /// The same request with one query parameter set.
    pub(crate) fn with_parameter(&self, name: &str, value: &str) -> Result<Self> {
        let mut request = self.clone();
        let mut parameters = request.url.parameters(true)?.into_owned();
        parameters.insert(name, value)?;
        request.url.set_parameters(&parameters)?;
        Ok(request)
    }

    /// The request's pagination, else the session's.
    pub(crate) fn pagination(&self) -> &Pagination {
        self.pagination
            .as_ref()
            .unwrap_or_else(|| self.session.options().pagination())
    }

    /// Where a page's rows are: the request's path, else the session's.
    pub(crate) fn records(&self) -> Option<&FieldPath> {
        self.records
            .as_ref()
            .or_else(|| self.session.options().records())
    }

    /// The most pages a walk reads, when the session bounds it.
    pub(crate) fn page_limit(&self) -> Option<usize> {
        self.session.options().page_limit()
    }

    /// The same request under another method, for the leaf's own verbs.
    fn as_method(&self, method: Method) -> Self {
        let mut request = self.clone();
        request.method = method;
        request.body = Body::Empty;
        request
    }

    /// Lock the state, reporting a poisoned lock rather than panicking.
    fn state(&self) -> Result<MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| poisoned())
    }

    /// Learn what an answer's headers say about the resource: its media
    /// type from `Content-Type`, once, and its length when `size` says so.
    fn learn(&self, headers: &Headers, size: Option<u64>) {
        if headers.content_type().is_some()
            && let Ok(media_type) = headers.media_type()
        {
            let _ = self.learned.set(media_type);
        }
        if let Some(size) = size
            && let Ok(mut state) = self.state()
        {
            learn_size(&mut state, size);
        }
    }

    /// The resource's metadata from one `HEAD`, or from the cache while
    /// open. The lock is never held across the request.
    fn meta(&self) -> Result<Option<Meta>> {
        if let Some(known) = self.state()?.meta {
            return Ok(known);
        }
        let probe = self.as_method(Method::Head);
        let (mut answer, url) = probe.exchange_range_free()?;
        let meta = match answer.status.code() {
            200..=299 => {
                let meta = Meta {
                    size: answer.headers.content_length()?,
                    mtime: answer.headers.last_modified()?,
                };
                self.learn(&answer.headers, None);
                Some(meta)
            }
            404 | 410 => None,
            _ => return Err(refusal(Method::Head, &mut answer, &url)),
        };
        let mut state = self.state()?;
        if state.opened {
            state.meta = Some(meta);
        }
        Ok(meta)
    }

    /// One exchange of this request with no extra headers, the identity
    /// coding asked for.
    fn exchange_range_free(&self) -> Result<(Answer, Url)> {
        let (answer, url, _history, _elapsed) =
            self.session.exchange(self, &Headers::new(), true)?;
        Ok((answer, url))
    }

    /// One ranged `GET` from `start` to `last`, answering the body reader
    /// positioned at `start` - skipping into a `200` when the server ignored
    /// the range - or `None` when the resource is absent or the range is
    /// past its end. What the answer teaches is learned.
    fn ranged(&self, start: u64, last: Option<u64>) -> Result<Option<Box<dyn Read + Send>>> {
        let probe = self.as_method(Method::Get);
        let (mut answer, url) = probe.exchange_range(start, last, None)?;
        match answer.status.code() {
            206 => {
                let range = starting_at(&answer, &url, start)?;
                self.learn(&answer.headers, range.total());
                Ok(Some(answer.body))
            }
            200 => {
                let total = answer.headers.content_length()?;
                self.learn(&answer.headers, total);
                let mut body = answer.body;
                let skipped = std::io::copy(&mut body.by_ref().take(start), &mut std::io::sink())
                    .map_err(Error::Io)?;
                if skipped < start {
                    return Ok(None);
                }
                Ok(Some(body))
            }
            416 => {
                let total = answer
                    .headers
                    .content_range()?
                    .and_then(|range| range.total());
                self.learn(&answer.headers, total);
                Ok(None)
            }
            404 | 410 => Ok(None),
            _ => Err(refusal(Method::Get, &mut answer, &url)),
        }
    }

    /// The whole resource with one `GET`, `None` when absent.
    fn fetch_all(&self) -> Result<Option<Vec<u8>>> {
        let probe = self.as_method(Method::Get);
        let (mut answer, url) = probe.exchange_range_free()?;
        match answer.status.code() {
            200..=299 => {
                // Read through a stream, so a transfer cut short resumes from
                // the byte it reached rather than failing the whole read.
                let stream = Stream::new(probe, url, answer, 0, None);
                let bytes = stream.read_from(0, self.session.options().max_body_size())?;
                self.learn(stream.headers(), Some(bytes.len() as u64));
                Ok(Some(bytes))
            }
            404 | 410 => {
                self.learn(&answer.headers, None);
                Ok(None)
            }
            _ => Err(refusal(Method::Get, &mut answer, &url)),
        }
    }

    /// Load the stored value into the stage so positional writes can land:
    /// one `GET`, and only the first time.
    fn materialize(&self) -> Result<()> {
        if self.state()?.stage.is_some() {
            return Ok(());
        }
        let bytes = self.fetch_all()?.unwrap_or_default();
        let mut state = self.state()?;
        if state.stage.is_none() {
            state.stage = Some(Stage {
                bytes,
                dirty: false,
            });
        }
        Ok(())
    }

    /// Publish the staged value with one `PUT`, `Content-Type` the media
    /// type. A closed handle lets go of what it published.
    fn publish(&self) -> Result<()> {
        let bytes = {
            let state = self.state()?;
            match state.stage.as_ref() {
                Some(stage) if stage.dirty => stage.bytes.clone(),
                _ => return Ok(()),
            }
        };
        let size = bytes.len() as u64;
        self.upload(&bytes)?;
        let mut state = self.state()?;
        if state.opened {
            if let Some(stage) = state.stage.as_mut() {
                stage.dirty = false;
            }
            state.meta = Some(Some(Meta {
                size: Some(size),
                mtime: None,
            }));
        } else {
            state.stage = None;
            state.meta = None;
        }
        Ok(())
    }

    /// One `PUT` of `bytes` as the resource's whole value.
    fn upload(&self, bytes: &[u8]) -> Result<()> {
        let put = self.put_of(bytes)?;
        let (mut answer, url) = put.exchange_range_free()?;
        if answer.status.is_success() {
            self.learn(&answer.headers, None);
            return Ok(());
        }
        Err(refusal(Method::Put, &mut answer, &url))
    }

    /// One `PUT` of `bytes` carrying `If-None-Match: *`, which RFC 9110
    /// answers `412` where the resource has a current representation.
    ///
    /// The `PUT` is declared non-idempotent, so it goes again only when no
    /// server saw it: a second attempt after one that landed would read the
    /// create's own value as the conflict. A `412` - or a `409`, as a store
    /// answering the Azure way says it - is the [`Error::Conflict`] naming the
    /// URL.
    fn create(&self, bytes: &[u8]) -> Result<()> {
        let mut put = self.put_of(bytes)?.with_idempotent(false);
        put.headers.insert("if-none-match", "*")?;
        let (mut answer, url) = put.exchange_range_free()?;
        if answer.status.is_success() {
            self.learn(&answer.headers, None);
            return Ok(());
        }
        match answer.status.code() {
            409 | 412 => Err(Error::conflict("resource", "resource", &url)),
            _ => Err(refusal(Method::Put, &mut answer, &url)),
        }
    }

    /// The `PUT` of `bytes` as the resource's whole value, `Content-Type` the
    /// media type and `Content-Encoding` the codings it names.
    fn put_of(&self, bytes: &[u8]) -> Result<Self> {
        let mut put = self.as_method(Method::Put);
        put.body = Body::from(bytes);
        let media_type = self.media_type();
        put.headers
            .insert("content-type", &content_type_header(media_type))?;
        if media_type.is_encoded() {
            let codings: Vec<&str> = media_type
                .encodings()
                .iter()
                .map(|encoding| crate::Codec::from_mime_type(encoding).as_str())
                .collect();
            put.headers
                .insert("content-encoding", &codings.join(", "))?;
        }
        Ok(put)
    }

    /// Drop the stage and the cache without publishing.
    fn discard(&self) -> Result<()> {
        let mut state = self.state()?;
        state.stage = None;
        state.meta = None;
        Ok(())
    }
}

/// A request is the leaf role over the resource its URL names.
impl IOFile for Request {
    fn file_url(&self) -> &Url {
        &self.url
    }

    /// One `HEAD`, or none while open.
    fn file_exists(&self) -> bool {
        self.meta().is_ok_and(|meta| meta.is_some())
    }

    /// Empty the resource: one `PUT` of no bytes.
    ///
    /// This is where an HTTP leaf departs from [`IOBase::clear`]'s
    /// "clearing is not a write", as an S3 leaf does: HTTP offers no way to
    /// empty a resource that does not create one, and the only way to find
    /// out first is a probe the no-pre-call rule forbids - so clearing an
    /// absent resource creates it empty.
    fn clear_file(&mut self) -> Result<()> {
        self.discard()?;
        self.upload(&[])
    }

    /// One `DELETE`, a `404` being the success the contract asks for.
    fn delete_file(&mut self) -> Result<()> {
        self.discard()?;
        let probe = self.as_method(Method::Delete);
        let (mut answer, url) = probe.exchange_range_free()?;
        match answer.status.code() {
            200..=299 | 404 | 410 => Ok(()),
            _ => Err(refusal(Method::Delete, &mut answer, &url)),
        }
    }
}

impl IOBase for Request {
    /// Read into `buffer` from `offset` with one ranged `GET`.
    ///
    /// A staged write answers from memory instead, and a length - or an
    /// absence - this open scope already knows answers without asking.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        {
            let state = self.state()?;
            if let Some(stage) = state.stage.as_ref() {
                return Ok(copy_from(&stage.bytes, offset, buffer));
            }
            if state.opened {
                match state.meta {
                    Some(Some(Meta {
                        size: Some(size), ..
                    })) if offset >= size => return Ok(0),
                    Some(None) => return Ok(0),
                    _ => {}
                }
            }
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let last = offset.saturating_add(buffer.len() as u64 - 1);
        let Some(mut body) = self.ranged(offset, Some(last))? else {
            return Ok(0);
        };
        read_full(&mut body, buffer)
    }

    /// Stream from `position` with one `GET`, resuming a cut transfer.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        if batch_size == 0 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "byte stream batch_size must be greater than zero",
            )));
        }
        if self.state()?.stage.is_some() {
            return crate::ByteStream::from_handle(self, position, batch_size);
        }
        let probe = self.as_method(Method::Get);
        let (mut answer, url) = probe.exchange_range(position, None, None)?;
        match answer.status.code() {
            200 | 206 => {
                let total = match answer.status.code() {
                    206 => starting_at(&answer, &url, position)?.total(),
                    _ => answer.headers.content_length()?,
                };
                self.learn(&answer.headers, total);
                let stream = Stream::new(probe, url, answer, position, None);
                crate::ByteStream::from_reader(stream, batch_size)
            }
            404 | 410 | 416 => {
                self.learn(&answer.headers, None);
                crate::ByteStream::from_reader(std::io::empty(), batch_size)
            }
            _ => Err(refusal(Method::Get, &mut answer, &url)),
        }
    }

    /// Read the whole resource with one `GET`, resuming a cut transfer
    /// from the byte it reached.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        if let Some(stage) = self.state()?.stage.as_ref() {
            return Ok(stage.bytes.clone());
        }
        Ok(self.fetch_all()?.unwrap_or_default())
    }

    /// Read `length` bytes from `offset` with one ranged `GET`.
    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        {
            let state = self.state()?;
            if let Some(stage) = state.stage.as_ref() {
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(stage.bytes.len());
                let end = start.saturating_add(length).min(stage.bytes.len());
                return Ok(stage.bytes[start..end].to_vec());
            }
            if state.opened {
                match state.meta {
                    Some(Some(Meta {
                        size: Some(size), ..
                    })) if offset >= size => return Ok(Vec::new()),
                    Some(None) => return Ok(Vec::new()),
                    _ => {}
                }
            }
        }
        if length == 0 {
            return Ok(Vec::new());
        }
        let last = offset.saturating_add(length as u64 - 1);
        let Some(body) = self.ranged(offset, Some(last))? else {
            return Ok(Vec::new());
        };
        // Nothing allocates `length` on the caller's word: the body is read
        // as far as it goes, and it goes no further than the range.
        let mut bytes = Vec::new();
        body.take(length as u64)
            .read_to_end(&mut bytes)
            .map_err(Error::Io)?;
        Ok(bytes)
    }

    /// Hash `length` bytes from `offset` with one `GET` of exactly that range.
    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        if self.state()?.stage.is_some() {
            return crate::xxhash::stream::read_range_digest(self, offset, length, algorithm);
        }
        let mut digester = algorithm.digester();
        if length == 0 {
            return Ok(digester.as_digest());
        }
        let last = offset.saturating_add(length as u64 - 1);
        let Some(body) = self.ranged(offset, Some(last))? else {
            return Ok(digester.as_digest());
        };
        let mut body = body.take(length as u64);
        let mut window = vec![0_u8; crate::DEFAULT_STREAM_BATCH_SIZE.min(length)];
        loop {
            let read = body.read(&mut window).map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            digester.write_bytes(&window[..read]);
        }
        Ok(digester.as_digest())
    }

    /// Stage `bytes` at `offset`, loading the stored value once.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.materialize()?;
        let mut state = self.state()?;
        let stage = state.stage.as_mut().ok_or_else(poisoned)?;
        let offset = usize::try_from(offset).map_err(|_| crate::iobase::oversized(offset))?;
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| crate::iobase::oversized(u64::MAX))?;
        if end > stage.bytes.len() {
            resize(&mut stage.bytes, end)?;
        }
        stage.bytes[offset..end].copy_from_slice(bytes);
        stage.dirty = true;
        Ok(bytes.len())
    }

    /// Replace the whole resource with one `PUT`; nothing is loaded first.
    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.state()?.stage = Some(Stage {
            bytes: bytes.to_vec(),
            dirty: true,
        });
        self.publish()
    }

    /// Create the resource with one `PUT` carrying `If-None-Match: *`;
    /// nothing is loaded first.
    ///
    /// The origin decides: one that honours the precondition - the crate's
    /// own [`Server`](crate::http::Server) does - answers `412` where the
    /// resource is, which is the [`Error::Conflict`] naming the URL, the
    /// resource left as it was; an origin that ignores preconditions
    /// overwrites, and nothing on this side can tell. A create that lands
    /// supersedes what this handle had staged; one refused forgets only what
    /// it knew of the stored value, and keeps a write still waiting to
    /// publish.
    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        if let Err(error) = self.create(bytes) {
            let mut state = self.state()?;
            if state.stage.as_ref().is_some_and(|stage| !stage.dirty) {
                state.stage = None;
            }
            state.meta = None;
            return Err(error);
        }
        let mut state = self.state()?;
        if state.opened {
            state.stage = Some(Stage {
                bytes: bytes.to_vec(),
                dirty: false,
            });
            state.meta = Some(Some(Meta {
                size: Some(bytes.len() as u64),
                mtime: None,
            }));
        } else {
            state.stage = None;
            state.meta = None;
        }
        Ok(())
    }

    /// Append after the current end: one `GET` and one `PUT`.
    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.materialize()?;
        let offset = {
            let mut state = self.state()?;
            let stage = state.stage.as_mut().ok_or_else(poisoned)?;
            let offset = stage.bytes.len() as u64;
            stage
                .bytes
                .try_reserve(bytes.len())
                .map_err(|_| crate::iobase::oversized(offset + bytes.len() as u64))?;
            stage.bytes.extend_from_slice(bytes);
            stage.dirty = true;
            offset
        };
        self.publish()?;
        Ok(offset)
    }

    /// The byte length: the stage's, the cached one while open, else one
    /// `HEAD`; zero when absent or unstated.
    fn size(&self) -> u64 {
        if let Ok(state) = self.state()
            && let Some(stage) = state.stage.as_ref()
        {
            return stage.bytes.len() as u64;
        }
        self.meta()
            .ok()
            .flatten()
            .and_then(|meta| meta.size)
            .unwrap_or(0)
    }

    fn capacity(&self) -> u64 {
        if let Ok(state) = self.state()
            && let Some(stage) = state.stage.as_ref()
        {
            return stage.bytes.capacity() as u64;
        }
        self.size()
    }

    /// Grow the staged buffer; a handle that staged nothing has nowhere to
    /// land the hint, which is success.
    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let mut state = self.state()?;
        let Some(stage) = state.stage.as_mut() else {
            return Ok(());
        };
        let capacity = usize::try_from(capacity).map_err(|_| crate::iobase::oversized(capacity))?;
        if capacity > stage.bytes.capacity() {
            stage
                .bytes
                .try_reserve_exact(capacity - stage.bytes.len())
                .map_err(|_| crate::iobase::oversized(capacity as u64))?;
        }
        Ok(())
    }

    /// Set the staged length, publishing nothing until a flush; to zero
    /// loads nothing.
    fn truncate(&mut self, size: u64) -> Result<()> {
        let size = usize::try_from(size).map_err(|_| crate::iobase::oversized(size))?;
        if size == 0 {
            self.state()?.stage = Some(Stage {
                bytes: Vec::new(),
                dirty: true,
            });
            return Ok(());
        }
        self.materialize()?;
        let mut state = self.state()?;
        let stage = state.stage.as_mut().ok_or_else(poisoned)?;
        resize(&mut stage.bytes, size)?;
        stage.dirty = true;
        Ok(())
    }

    fn uri(&self) -> Option<&Uri> {
        Some(self.url.as_ref())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    /// `Last-Modified`, from the cached head or one `HEAD`.
    fn mtime(&self) -> Option<i64> {
        self.meta().ok().flatten().and_then(|meta| meta.mtime)
    }

    /// The declared media type, else the `Content-Type` a response taught,
    /// else the URL's own.
    fn media_type(&self) -> &MediaType {
        if let Some(declared) = &self.declared {
            return declared;
        }
        if let Some(learned) = self.learned.get() {
            return learned;
        }
        self.inferred.get_or_init(|| {
            if self.url.extension().is_none() {
                return MediaType::from(MimeType::FILE);
            }
            self.url.media_type()
        })
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    /// `File` once a head answered `2xx` or a write is staged, `Unknown` on
    /// a `404`.
    fn kind(&self) -> IOKind {
        if let Ok(state) = self.state()
            && state.stage.as_ref().is_some_and(|stage| stage.dirty)
        {
            return IOKind::File;
        }
        self.file_kind()
    }

    fn is_container(&self) -> bool {
        false
    }

    fn is_atomic(&self) -> bool {
        self.file_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.file_is_tabular()
    }

    fn flush(&mut self) -> Result<()> {
        self.publish()
    }

    /// Cache the resource's head for this scope: one `HEAD`, never the
    /// bytes.
    fn open(&mut self) -> Result<()> {
        {
            let mut state = self.state()?;
            if state.opened && state.meta.is_some() {
                return Ok(());
            }
            state.opened = true;
        }
        let meta = self.meta()?;
        self.state()?.meta = Some(meta);
        Ok(())
    }

    fn opened(&self) -> bool {
        self.state().is_ok_and(|state| state.opened)
    }

    /// Publish anything staged and drop what was cached.
    fn close(&mut self) -> Result<()> {
        self.publish()?;
        let mut state = self.state()?;
        state.stage = None;
        state.meta = None;
        state.opened = false;
        Ok(())
    }

    /// HTTP names no hierarchy: the URL above this one is another resource,
    /// not this one's container.
    fn parent(&self) -> Option<Holder> {
        None
    }

    fn clear(&mut self) -> Result<()> {
        self.clear_file()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.file_remove(recursive)
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        self.file_child_by_path(name)
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        self.file_ls()
    }
}

impl crate::IOMedia for Request {
    crate::impl_default_iomedia!();

    /// The rows of the resource under `options`: a structured document whose
    /// first page paginates - per the request's [`Pagination`], `Auto` by
    /// default - reads one batch per page through [`Request::pages`], the
    /// declared field and batch row size taken off `options` and its
    /// clauses applied; every other resource reads through its bytes as
    /// every leaf does, a structured document of one page included.
    ///
    /// The first page is read once to decide and handed to the walk when it
    /// paginates, so a paginated read costs one `GET` per page and no more.
    fn read_serie(&self, options: Option<&crate::media::RecordOptions>) -> Result<crate::Serie> {
        use crate::media::IORecordOptions;

        match self.first_page()? {
            Some(FirstPage {
                paginates: true,
                response,
            }) => {
                // A paginated document supplies its own row encoding; absent
                // options select the native page transport, not a JSON codec.
                let options = options
                    .cloned()
                    .unwrap_or_else(|| crate::media::RecordOptions::Ipc(Default::default()));
                let reader = self.pages_from(response)?.into_arrow_reader(
                    options.field().as_ref(),
                    options
                        .batch_row_size()
                        .unwrap_or(crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE),
                )?;
                crate::iomedia::landed(
                    options.limit_arrow_reader(options.apply_arrow_expressions(reader)?)?,
                )
                .map(crate::Serie::from)
            }
            Some(first) => crate::IOMedia::read_serie(&first.held()?, options),
            None if crate::text::Format::from_media_type(self.media_type()).is_ok() => {
                crate::iomedia::read_document(self, options)
            }
            None => {
                let options = crate::iomedia::own_options(self, options)?;
                let reader = crate::iobase::leaf_reader(self, &options)?;
                crate::iomedia::landed(
                    options.limit_arrow_reader(options.apply_arrow_expressions(reader)?)?,
                )
                .map(crate::Serie::from)
            }
        }
    }
}

/// The first page of a structured resource read under a pagination.
struct FirstPage {
    response: Response,
    paginates: bool,
}

impl FirstPage {
    /// The page's decoded bytes held in memory under its media type, so the
    /// document is read once off the wire.
    fn held(&self) -> Result<Holder> {
        let mut media_type = self.response.media_type().clone();
        media_type.clear_encodings();
        Ok(Holder::buffer(
            crate::holder::Buffer::from_bytes(self.response.bytes()?.to_vec())
                .with_media_type(media_type),
        ))
    }
}

impl Request {
    /// Read the first page of a structured document under this request's
    /// pagination and say whether a second follows; `None` when the resource
    /// is not a structured document or the pagination is `None`.
    ///
    /// A `404` reads as the empty document every leaf's absence is, so it
    /// is not a page; any other refusal is the error it is.
    /// The page walk from `first`, the `GET` of this resource already fetched
    /// to decide that it paginates.
    fn pages_from(&self, first: Response) -> Result<Pages> {
        Pages::from_first(self.session.clone(), self.as_method(Method::Get), first)
    }

    fn first_page(&self) -> Result<Option<FirstPage>> {
        let structured = crate::text::Format::from_media_type(self.media_type()).is_ok();
        if !structured || matches!(self.pagination(), Pagination::None) {
            return Ok(None);
        }
        let response = self.as_method(Method::Get).send()?;
        if response.status().code() == 404 {
            return Ok(None);
        }
        response.raise_for_status()?;
        self.learn(response.headers(), None);
        let body = response.scalar()?;
        let rows = Pagination::records_path(&body, self.records())
            .and_then(|path| rows_at(&body, &path))
            .unwrap_or(1);
        let next =
            self.pagination()
                .next(response.url(), response.headers(), Some(&body), 0, rows)?;
        Ok(Some(FirstPage {
            response,
            paginates: next.is_some(),
        }))
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        // Publish a staged write; a failure here cannot be reported, and a
        // caller who cares calls `flush` or `close` explicitly.
        let _ = self.publish();
    }
}

/// How many rows `body` holds at `path`, when it holds a sequence there.
fn rows_at(body: &Scalar, path: &FieldPath) -> Option<usize> {
    if path.is_root() {
        return body.sequence_rows().map(|rows| rows.len());
    }
    body.path(path)
        .and_then(|value| value.sequence_rows().map(|rows| rows.len()))
}

/// The `Host` header a URL asks for: the host, with the port when it is
/// not the scheme's default - an IPv6 literal in its brackets, as RFC 3986
/// writes it and the transport sends it, so a signature over this host is
/// a signature over the one sent.
pub(crate) fn host_header(url: &Url) -> String {
    let host = url.hostname().unwrap_or_default();
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    match url.authority().port() {
        Some(port) if Some(port) != url.default_port() => format!("{host}:{port}"),
        _ => host,
    }
}

/// The `Content-Type` a media type is written as: the base, its charset
/// when declared, never the codings, which `Content-Encoding` carries.
fn content_type_header(media_type: &MediaType) -> String {
    match media_type.charset() {
        Some(charset) => format!("{}; charset={}", media_type.base(), charset.as_str()),
        None => media_type.base().to_string(),
    }
}

/// Record what a read has just learned about the resource's length, while
/// open.
fn learn_size(state: &mut State, size: u64) {
    if !state.opened {
        return;
    }
    match state.meta.as_mut() {
        Some(Some(meta)) => meta.size = Some(size),
        _ => {
            state.meta = Some(Some(Meta {
                size: Some(size),
                mtime: None,
            }));
        }
    }
}

/// Copy what `bytes` holds from `offset` into `buffer`.
fn copy_from(bytes: &[u8], offset: u64, buffer: &mut [u8]) -> usize {
    let Ok(offset) = usize::try_from(offset) else {
        return 0;
    };
    if offset >= bytes.len() {
        return 0;
    }
    let available = &bytes[offset..];
    let count = available.len().min(buffer.len());
    buffer[..count].copy_from_slice(&available[..count]);
    count
}

/// Fill `buffer` from `body` until it is full or the body ends.
fn read_full(body: &mut dyn Read, buffer: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = body.read(&mut buffer[filled..]).map_err(Error::Io)?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// The `Content-Range` of a `206` asked for bytes from `start`, refused
/// unless it states that it starts there: a range starting anywhere else,
/// or stating nowhere, would hand over bytes at the wrong place.
fn starting_at(answer: &Answer, url: &Url, start: u64) -> Result<ContentRange> {
    let range = answer.headers.content_range()?;
    match range {
        Some(range @ ContentRange::Bytes { start: stated, .. }) if stated == start => Ok(range),
        _ => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "asked {url} for bytes from {start}, got a 206 stating {}",
                range.map_or_else(|| "no Content-Range".to_owned(), |range| range.to_string())
            ),
        ))),
    }
}

/// The refusal a status past the redirects is, named by the method, the
/// status, its reason phrase, the first line of the body and the URL.
pub(crate) fn refusal(method: Method, answer: &mut Answer, url: &Url) -> Error {
    let mut body = Vec::new();
    let _ = answer.body.by_ref().take(4096).read_to_end(&mut body);
    let message = crate::Charset::Utf8.transcribe(&body);
    Error::remote(
        "http",
        method.as_str(),
        answer.status.code(),
        answer.status.reason(),
        message.as_ref(),
        url,
    )
}

/// Resize a staged value, refusing rather than aborting when it will not fit.
fn resize(bytes: &mut Vec<u8>, size: usize) -> Result<()> {
    if size <= bytes.len() {
        bytes.truncate(size);
        return Ok(());
    }
    bytes
        .try_reserve(size - bytes.len())
        .map_err(|_| crate::iobase::oversized(size as u64))?;
    bytes.resize(size, 0);
    Ok(())
}

/// Report a poisoned state lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the HTTP request's state lock was poisoned by a panicking writer",
    ))
}

crate::media_serie::media_serie!(HttpSerie, Http, as_http, get_http_mut, accepts = None);
