//! A session: the client, the options every request starts from, and the
//! cookie jar; the one place a request goes out.
//!
//! [`Session::send`] is where the session's defaults meet the request's own
//! headers, where the credential and the cookies are added, where the
//! `Accept-Encoding` and `User-Agent` a caller did not spell are filled in,
//! where a `3xx` is followed - method rewritten, body dropped, credential
//! withheld from another host - and where every `Set-Cookie` lands in the
//! jar. The client beneath retries; the session redirects.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::client::{Answer, Wire};
use super::{
    Authorization, Client, Cookie, CookieJar, Headers, HttpOptions, Method, Pages, Request,
    Response, StatsSnapshot, Status,
};
use crate::holder::Holder;
use crate::{Error, IOBase, IOKind, Listing, MediaType, Result, Uri, Url};

/// How many requests one worker of [`Session::send_all`] takes at a time.
const SEND_ALL_CHUNK: usize = 1;

/// What every request of a session shares.
struct Inner {
    client: Client,
    options: HttpOptions,
    cookies: Mutex<CookieJar>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("client", &self.client)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// The defaults, the credential and the cookies a set of requests share.
///
/// Cloning a session shares its client and its jar; building one touches no
/// network. A relative URL handed to [`Self::request`] joins onto the base
/// URL the options name.
///
/// ```
/// use yggdryl::http::{HttpOptions, Session};
/// use yggdryl::Url;
///
/// # fn main() -> yggdryl::Result<()> {
/// let session = Session::with_options(
///     HttpOptions::default()
///         .with_base_url(Url::from_str("https://api.example.com/v1/")?)
///         .with_header("Accept", "application/json")?,
/// )?;
/// let request = session.get("orders?limit=10")?;
/// assert_eq!(request.url().to_string(), "https://api.example.com/v1/orders?limit=10");
/// // Nothing has gone out: building a session and a request costs no request.
/// assert_eq!(session.stats().requests, 0);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session over the shared client with the default options.
    #[must_use]
    pub fn new() -> Self {
        Self::from_parts(Client::new(), HttpOptions::default())
    }

    /// A session with `options`, over a client built for their transport.
    ///
    /// # Errors
    ///
    /// As [`Client::with_options`].
    pub fn with_options(options: HttpOptions) -> Result<Self> {
        let client = Client::with_options(&options)?;
        Ok(Self::from_parts(client, options))
    }

    /// A session with `options` over `client`, whose pool the transport
    /// knobs of `options` do not change.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a transport the client
    /// cannot serve.
    pub fn with_client(client: Client, options: HttpOptions) -> Result<Self> {
        Ok(Self::from_parts(client, options))
    }

    /// Assemble a session, touching nothing.
    pub(crate) fn from_parts(client: Client, options: HttpOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                options,
                cookies: Mutex::new(CookieJar::new()),
            }),
        }
    }

    /// The options every request starts from.
    #[must_use]
    pub fn options(&self) -> &HttpOptions {
        &self.inner.options
    }

    /// The client requests go out on.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.inner.client
    }

    /// The client's counters.
    #[must_use]
    pub fn stats(&self) -> StatsSnapshot {
        self.inner.client.stats()
    }

    /// The URL a relative request joins onto, when the options name one.
    #[must_use]
    pub fn base_url(&self) -> Option<&Url> {
        self.inner.options.base_url()
    }

    /// A request of `method` at `url`, bound to this session.
    ///
    /// An absolute `http` or `https` URL is taken as it is; a relative
    /// reference joins onto the base URL per RFC 3986.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] with target `http url` for a URL of another scheme,
    /// a reference that does not parse, or a relative reference on a session
    /// with no base URL.
    pub fn request(&self, method: Method, url: &str) -> Result<Request> {
        let url = self.resolve(url)?;
        Ok(Request::new(method, url).with_session(self.clone()))
    }

    /// A `GET` request at `url`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn get(&self, url: &str) -> Result<Request> {
        self.request(Method::Get, url)
    }

    /// A `HEAD` request at `url`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn head(&self, url: &str) -> Result<Request> {
        self.request(Method::Head, url)
    }

    /// A `DELETE` request at `url`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn delete(&self, url: &str) -> Result<Request> {
        self.request(Method::Delete, url)
    }

    /// An `OPTIONS` request at `url`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn options_request(&self, url: &str) -> Result<Request> {
        self.request(Method::Options, url)
    }

    /// A `POST` request at `url` carrying `body`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn post(&self, url: &str, body: impl Into<super::Body>) -> Result<Request> {
        Ok(self.request(Method::Post, url)?.with_body(body))
    }

    /// A `PUT` request at `url` carrying `body`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn put(&self, url: &str, body: impl Into<super::Body>) -> Result<Request> {
        Ok(self.request(Method::Put, url)?.with_body(body))
    }

    /// A `PATCH` request at `url` carrying `body`; see [`Self::request`].
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn patch(&self, url: &str, body: impl Into<super::Body>) -> Result<Request> {
        Ok(self.request(Method::Patch, url)?.with_body(body))
    }

    /// Send `request` on this session, reading the whole body.
    ///
    /// The session's defaults go under the request's headers, its
    /// credential is sent when the request names none, the jar's cookies for
    /// the URL ride along, redirects are followed and the answer's cookies
    /// are stored. The body is held whole, bounded by the options'
    /// `max_body_size`, and decoded per its `Content-Encoding`.
    ///
    /// # Errors
    ///
    /// A transport failure ([`Error::Io`]), a header that will not validate
    /// ([`Error::Parse`]), or more redirects than the options allow. A
    /// refusing status is not an error: read it off the response, or ask
    /// [`Response::raise_for_status`].
    pub fn send(&self, request: &Request) -> Result<Response> {
        let (answer, url, history, elapsed) = self.exchange(request, &Headers::new(), false)?;
        Response::from_answer(
            request.clone().with_session(self.clone()),
            answer,
            url,
            history,
            elapsed,
            false,
        )
    }

    /// Send `request`, leaving the body on the wire as a
    /// [`Stream`](super::Stream).
    ///
    /// The request asks for the identity coding, so the bytes are the
    /// resource's own and a cut transfer resumes at a byte offset.
    ///
    /// # Errors
    ///
    /// As [`Self::send`].
    pub fn stream(&self, request: &Request) -> Result<Response> {
        let (answer, url, history, elapsed) = self.exchange(request, &Headers::new(), true)?;
        Response::from_answer(
            request.clone().with_session(self.clone()),
            answer,
            url,
            history,
            elapsed,
            true,
        )
    }

    /// Send every request of `requests` on up to `concurrency` threads,
    /// answering in the order they were given.
    ///
    /// A failure is one item; the walk goes on with the next request. One
    /// thread is the sequential map and spawns nothing.
    pub fn send_all<I>(&self, requests: I) -> impl Iterator<Item = Result<Response>>
    where
        I: IntoIterator<Item = Request>,
        I::IntoIter: Send + 'static,
    {
        let session = self.clone();
        crate::parallel::ordered(
            requests,
            self.inner.options.concurrency(),
            SEND_ALL_CHUNK,
            move |request: Request| session.send(&request),
        )
        .with_lane_depth(1)
    }

    /// Walk the pages of `request` per its pagination.
    #[must_use]
    pub fn pages(&self, request: Request) -> Pages {
        Pages::new(self.clone(), request)
    }

    /// Every cookie the jar holds.
    #[must_use]
    pub fn cookies(&self) -> Vec<Cookie> {
        self.jar()
            .map(|jar| jar.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Store `cookie` in the jar, replacing the one it names.
    pub fn set_cookie(&self, cookie: Cookie) {
        if let Ok(mut jar) = self.jar() {
            jar.set(cookie);
        }
    }

    /// Release what this session holds between requests: the cookies that
    /// have lapsed are evicted. The pool's idle connections are the
    /// client's and lapse on their own.
    pub fn close(&self) {
        let now = self.now_ns();
        if let Ok(mut jar) = self.jar() {
            jar.remove_expired(now);
        }
    }

    /// The client, borrowed for the seam.
    pub(crate) fn client_ref(&self) -> &Client {
        &self.inner.client
    }

    /// One clock reading the cookie jar and the pauses share: UTC
    /// nanoseconds since the epoch.
    pub(crate) fn now_ns(&self) -> i64 {
        crate::holder::system_time_ns(std::time::SystemTime::now()).unwrap_or(0)
    }

    /// The longest a `Retry-After` or a rate limit is waited for.
    pub(crate) fn max_pause(&self) -> Duration {
        self.inner.options.max_pause()
    }

    /// The jar, reporting a poisoned lock rather than panicking.
    fn jar(&self) -> Result<MutexGuard<'_, CookieJar>> {
        self.inner.cookies.lock().map_err(|_| poisoned())
    }

    /// Resolve the text of a URL against this session.
    fn resolve(&self, url: &str) -> Result<Url> {
        if is_absolute_reference(url) {
            return http_url(url);
        }
        let Some(base) = self.base_url() else {
            return Err(Error::Parse {
                target: "http url",
                position: 0,
                reason: smol_str::format_smolstr!(
                    "expected an absolute http or https URL, got the relative reference {url:?} \
                     on a session with no base URL"
                ),
            });
        };
        base.join_reference(url)
    }

    /// The complete header set a request to `url` goes out with: the
    /// session's defaults under the request's own, the credential, the
    /// cookies for `url`, `Accept-Encoding` and `User-Agent` when unset.
    ///
    /// `ranged` asks for the identity coding, so byte offsets stay
    /// meaningful.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a header that will not validate.
    pub(crate) fn headers_for(
        &self,
        request: &Request,
        url: &Url,
        ranged: bool,
    ) -> Result<Headers> {
        let mut headers = request.headers().merge_with(self.inner.options.headers())?;
        let authorization = request
            .authorization()
            .or(self.inner.options.authorization())
            .cloned()
            .or_else(|| Authorization::from_url(url));
        if let Some(authorization) = authorization {
            if request.authorization().is_some()
                || !headers.contains_key(authorization.header_name())
            {
                headers.insert(authorization.header_name(), &authorization.header_value())?;
            }
        }
        if self.inner.options.cookies() && !headers.contains_key("cookie") {
            let cookie = self.jar()?.header_for(url, self.now_ns());
            if let Some(cookie) = cookie {
                headers.insert("cookie", &cookie)?;
            }
        }
        if !headers.contains_key("accept-encoding") {
            let codings = if ranged {
                "identity".to_owned()
            } else {
                accept_encoding(&self.inner.options)
            };
            headers.insert("accept-encoding", &codings)?;
        }
        if !headers.contains_key("user-agent") {
            headers.insert("user-agent", self.inner.options.user_agent())?;
        }
        Ok(headers)
    }

    /// One exchange with redirects and cookies.
    ///
    /// The client executes each hop; every `Set-Cookie` is stored; a
    /// `Location` is resolved through [`Url::join_reference`]; a `303`, and
    /// a `301` or `302` on `POST`, become a `GET` without the body, a `307`
    /// or `308` keep both; the credential is withheld from another host.
    /// Answers the final hop, its URL, the drained earlier hops oldest first,
    /// and the time the whole exchange took.
    ///
    /// # Errors
    ///
    /// As [`Self::send`], plus a redirect chain longer than the options'
    /// `max_redirects`, refused naming the URL it stopped at.
    pub(crate) fn exchange(
        &self,
        request: &Request,
        extra: &Headers,
        ranged: bool,
    ) -> Result<(Answer, Url, Vec<Response>, Duration)> {
        let started = Instant::now();
        let options = &self.inner.options;
        let follow = request
            .follow_redirects()
            .unwrap_or(options.follow_redirects());
        let timeout = request.timeout().unwrap_or(options.timeout());
        let mut method = request.method();
        let mut url = request.url().clone();
        let mut body = request.body().clone();
        let mut history = Vec::new();
        let mut hops = 0_u32;
        loop {
            let mut headers = self.headers_for(request, &url, ranged)?;
            if hops > 0 && !same_origin(request.url(), &url) {
                headers.remove("authorization");
            }
            if body.is_empty() && method != request.method() {
                // The body went with the method it was sent under.
                headers.remove("content-type");
                headers.remove("content-length");
            }
            for (name, value) in extra {
                headers.insert(name, value)?;
            }
            let wire = Wire {
                method,
                url: &url,
                headers: &headers,
                body: (!body.is_empty()).then(|| body.as_bytes()),
                timeout,
                retry_body: true,
            };
            let answer = self.inner.client.execute(&wire)?;
            if options.cookies() {
                self.jar()?
                    .set_from_headers(&url, &answer.headers, self.now_ns());
            }
            let location = answer.headers.location().map(str::to_owned);
            let Some(location) = location.filter(|_| follow && follows(answer.status)) else {
                return Ok((answer, url, history, started.elapsed()));
            };
            hops += 1;
            if hops > options.max_redirects() {
                return Err(Error::remote(
                    "http",
                    method.as_str(),
                    answer.status.code(),
                    "TooManyRedirects",
                    format!(
                        "more than {} redirects, the last to {location}",
                        options.max_redirects()
                    ),
                    &url,
                ));
            }
            let status = answer.status;
            let next = url.join_reference(&location)?;
            history.push(Response::drained(
                request.clone().with_session(self.clone()),
                answer,
                url,
            )?);
            self.inner.client.record_redirect();
            url = next;
            if rewrites_to_get(status, method) {
                method = Method::Get;
                body = super::Body::Empty;
            }
        }
    }
}

/// Whether `status` is a redirect this session follows when a `Location`
/// is given.
fn follows(status: Status) -> bool {
    matches!(status.code(), 301 | 302 | 303 | 307 | 308)
}

/// Whether the redirect `status` turns `method` into a `GET` without its
/// body: a `303` always (a `HEAD` stays), a `301` or `302` on `POST`.
fn rewrites_to_get(status: Status, method: Method) -> bool {
    match status.code() {
        303 => method != Method::Head,
        301 | 302 => method == Method::Post,
        _ => false,
    }
}

/// Whether two URLs name one origin: scheme, host and port.
fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.hostname().map(str::to_ascii_lowercase)
            == right.hostname().map(str::to_ascii_lowercase)
        && left.authority().port().or(left.default_port())
            == right.authority().port().or(right.default_port())
}

/// The `Accept-Encoding` a session sends: the codings it decodes, in
/// preference order, `identity` when it decodes none.
fn accept_encoding(options: &HttpOptions) -> String {
    let codings: Vec<&str> = options
        .accept_encodings()
        .iter()
        .filter(|codec| !codec.is_identity())
        .map(|codec| codec.as_str())
        .collect();
    if codings.is_empty() {
        "identity".to_owned()
    } else {
        codings.join(", ")
    }
}

/// Whether `text` opens with a URI scheme: RFC 3986's `ALPHA *(ALPHA / DIGIT
/// / "+" / "-" / ".") ":"` before any `/`, `?` or `#`.
pub(crate) fn is_absolute_reference(text: &str) -> bool {
    let Some(colon) = text.find(':') else {
        return false;
    };
    let scheme = &text[..colon];
    let mut bytes = scheme.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

/// Parse `text` as an absolute `http` or `https` URL.
///
/// # Errors
///
/// [`Error::Parse`] with target `http url` for another scheme or text that
/// is not a URL.
pub(crate) fn http_url(text: &str) -> Result<Url> {
    let url = Url::from_str(text)?;
    if !url.scheme().is_http() {
        return Err(Error::Parse {
            target: "http url",
            position: 0,
            reason: smol_str::format_smolstr!(
                "expected an http or https URL, got the scheme {:?} in {text:?}",
                url.scheme().as_str()
            ),
        });
    }
    Ok(url)
}

/// Report a poisoned session lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the HTTP session's cookie jar lock was poisoned by a panicking writer",
    ))
}

/// A session is a container over its base URL: it lists nothing, holds no
/// bytes, and resolves a path to the `GET` request of that resource.
impl IOBase for Session {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        Err(super::client::is_a_directory(
            bytes.len(),
            "an HTTP session",
        ))
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
        super::client::truncate_container(size, "an HTTP session")
    }

    fn uri(&self) -> Option<&Uri> {
        self.base_url().map(AsRef::as_ref)
    }

    fn url(&self) -> Option<&Url> {
        self.base_url()
    }

    fn media_type(&self) -> &MediaType {
        super::client::directory_media_type()
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

    /// The resource `path` names, as a `GET` of it on this session: a
    /// relative path joins onto the base URL, an absolute URL is taken as
    /// it is.
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    fn child_by_path(&self, path: &str) -> Result<Holder> {
        Ok(Holder::HttpRequest(self.get(path)?))
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

impl crate::IOMedia for Session {
    crate::impl_default_iomedia!();
}
