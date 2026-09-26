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
    Authorization, Headers, HttpVersion, Method, Pages, Pagination, Response, Session,
    StatsSnapshot, Stream, range_header,
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
        formatter
            .debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
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
    /// A reader cannot be read twice, so nothing is retried and no redirect
    /// is followed: the answer is the first server's.
    ///
    /// # Errors
    ///
    /// As [`Session::send`], every transport failure final.
    pub fn send_reader(&self, reader: &mut dyn Read, length: u64) -> Result<Response> {
        let started = std::time::Instant::now();
        let headers = self.session.headers_for(self, &self.url, false)?;
        let wire = Wire {
            method: self.method,
            url: &self.url,
            headers: &headers,
            body: None,
            timeout: self.timeout.unwrap_or(self.session.options().timeout()),
            retry_body: false,
        };
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
        if headers.content_type().is_some() {
            if let Ok(media_type) = headers.media_type() {
                let _ = self.learned.set(media_type);
            }
        }
        if let Some(size) = size {
            if let Ok(mut state) = self.state() {
                learn_size(&mut state, size);
            }
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
                let total = answer
                    .headers
                    .content_range()?
                    .and_then(|range| range.total());
                self.learn(&answer.headers, total);
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
                let bytes = read_bounded(&mut answer.body, self.session.options().max_body_size())?;
                self.learn(&answer.headers, Some(bytes.len() as u64));
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
        let (mut answer, url) = put.exchange_range_free()?;
        if answer.status.is_success() {
            self.learn(&answer.headers, None);
            return Ok(());
        }
        Err(refusal(Method::Put, &mut answer, &url))
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

    /// One `PUT` of no bytes: HTTP offers no way to empty a resource that
    /// does not create one, and probing first is what the contract forbids.
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
    /// A staged write answers from memory instead, and a length this open
    /// scope already knows bounds the read without asking.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        {
            let state = self.state()?;
            if let Some(stage) = state.stage.as_ref() {
                return Ok(copy_from(&stage.bytes, offset, buffer));
            }
            if state.opened {
                if let Some(Some(Meta {
                    size: Some(size), ..
                })) = state.meta
                {
                    if offset >= size {
                        return Ok(0);
                    }
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
                    206 => answer
                        .headers
                        .content_range()?
                        .and_then(|range| range.total()),
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

    /// Read the whole resource with one `GET`.
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
        if let Ok(state) = self.state() {
            if let Some(stage) = state.stage.as_ref() {
                return stage.bytes.len() as u64;
            }
        }
        self.meta()
            .ok()
            .flatten()
            .and_then(|meta| meta.size)
            .unwrap_or(0)
    }

    fn capacity(&self) -> u64 {
        if let Ok(state) = self.state() {
            if let Some(stage) = state.stage.as_ref() {
                return stage.bytes.capacity() as u64;
            }
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
        if let Ok(state) = self.state() {
            if state.stage.as_ref().is_some_and(|stage| stage.dirty) {
                return IOKind::File;
            }
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
    fn read_arrow_reader(
        &self,
        options: &crate::media::RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        use crate::media::IORecordOptions;

        match self.first_page()? {
            Some(FirstPage {
                paginates: true,
                response,
            }) => {
                let reader = self.pages_from(response)?.into_arrow_reader(
                    options.field().as_ref(),
                    options
                        .batch_row_size()
                        .unwrap_or(crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE),
                )?;
                options.limit_arrow_reader(options.apply_arrow_expressions(reader)?)
            }
            Some(first) => crate::IOMedia::read_arrow_reader(&first.held()?, options),
            None => {
                let reader = crate::iobase::leaf_reader(self, options)?;
                options.limit_arrow_reader(options.apply_arrow_expressions(reader)?)
            }
        }
    }

    /// The rows of the resource as a [`SerieReader`](crate::SerieReader): a
    /// structured document whose first page paginates streams one record
    /// column per page; one of a single page is the record column its rows
    /// parse into, read off the page already fetched; every other resource
    /// answers what [`crate::IOMedia::read_arrow_reader`] produces.
    fn read_arrow(
        &self,
        options: Option<&crate::media::RecordOptions>,
    ) -> Result<crate::SerieReader> {
        use crate::media::IORecordOptions;

        let field = options.and_then(IORecordOptions::field);
        match self.first_page()? {
            Some(FirstPage {
                paginates: true,
                response,
            }) => self.pages_from(response)?.into_serie_reader(field.as_ref()),
            Some(first) => {
                let records = crate::media::structured::read_arrow(&first.held()?, field.as_ref())?;
                Ok(crate::SerieReader::from_serie(records)?)
            }
            None if crate::text::Format::from_media_type(self.media_type()).is_ok() => {
                let records = crate::media::structured::read_arrow(self, field.as_ref())?;
                Ok(crate::SerieReader::from_serie(records)?)
            }
            None => {
                let reader = match options {
                    Some(options) => self.read_arrow_reader(options)?,
                    None => self.read_arrow_reader(
                        &crate::media::RecordOptions::for_media_type(self.media_type())?,
                    )?,
                };
                Ok(crate::SerieReader::from_arrow_reader(
                    None,
                    reader,
                    crate::ArrowCastOptions::default(),
                )?)
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
    body.path(&path.to_string())
        .and_then(|value| value.sequence_rows().map(|rows| rows.len()))
}

/// The `Host` header a URL asks for: the host, with the port when it is
/// not the scheme's default.
fn host_header(url: &Url) -> String {
    let host = url.hostname().unwrap_or_default();
    match url.authority().port() {
        Some(port) if Some(port) != url.default_port() => format!("{host}:{port}"),
        _ => host.to_owned(),
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

/// Read `body` whole, refusing one longer than `limit`.
pub(crate) fn read_bounded(body: &mut dyn Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    body.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(Error::Io)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Io(std::io::Error::other(format!(
            "the response body exceeds the {limit} bytes a whole-body read holds; stream it instead"
        ))));
    }
    Ok(bytes)
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
