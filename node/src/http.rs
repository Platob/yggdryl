//! `yggdryl::http`, exposed to JavaScript: the session, the request and its
//! answer, the header map, the page walk and the server.
//!
//! Every class here holds one core value and redirects into it. The loader
//! (`binding.js`) spells the JavaScript conveniences - a header map as a
//! `Headers`, a `Map`, entries or a plain object; a JSON body as any value
//! `Scalar.from` reads - down to the one shape each native entry takes, so
//! nothing below parses a header, a URL, a pagination or a body on its own.
//!
//! Every network call is synchronous, as every other handle read is: it
//! blocks the calling thread until the exchange ends. A server the same
//! thread answers from its own event loop (`node:http` on the main thread)
//! therefore cannot answer a request this thread is waiting on; the core's
//! `Server`, which answers from its own threads, can.

use std::time::Duration;

use napi::bindgen_prelude::{BigInt, Buffer, ClassInstance, Either, Null, Result, Uint8Array};
use napi_derive::napi;
use yggdryl::holder::Holder;
use yggdryl::http::{
    Authorization, ContentRange, Cookie, Fault, Headers, HttpOptions, Method, Pages, Pagination,
    Recorded, Request, Response, Server, ServerOptions, Session, StatsSnapshot, Status,
};
use yggdryl::media::DEFAULT_ROOT_NAME;
use yggdryl::{FieldPath, Url};

use crate::enums::{JsMediaType, MediaTypeInput, media_type_from_input};
use crate::iceberg::{FieldInput, field_from_input};
use crate::iobase::JsIOBase;
use crate::iomedia::JsBatchReader;
use crate::text::codec::JsScalar;
use crate::uri::JsUrl;
use crate::{exact_u64, napi_error};

/// A millisecond count as a duration, refused rather than rounded into one.
fn millis(value: f64, name: &str) -> Result<Duration> {
    if !value.is_finite() || value < 0.0 {
        return Err(napi_error(format!(
            "{name} must be a finite, non-negative number of milliseconds, got {value}"
        )));
    }
    Ok(Duration::from_secs_f64(value / 1000.0))
}

/// A duration as JavaScript's millisecond count.
fn as_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// An optional core count as a JavaScript number, exact below 2^53.
fn count(value: Option<u64>, name: &str) -> Result<Option<f64>> {
    value.map(|value| crate::exact_f64(value, name)).transpose()
}

fn method_of(text: &str) -> Result<Method> {
    Method::from_str(text).map_err(napi_error)
}

fn status_of(code: u32) -> Result<Status> {
    let code = u16::try_from(code)
        .map_err(|_| napi_error(format!("status must be between 100 and 599, got {code}")))?;
    Status::new(code).map_err(napi_error)
}

fn headers_of(entries: Vec<(String, String)>) -> Result<Headers> {
    Headers::from_entries(entries).map_err(napi_error)
}

/// A credential, spelled one way at a time.
///
/// The loader reads `[username, password]` as the `basic` pair, so the object
/// is the one shape the native side takes.
#[napi(object, object_to_js = false)]
pub struct HttpAuth {
    /// `Basic` credentials: the user, beside `password`.
    pub username: Option<String>,
    /// `Basic` credentials: the password, beside `username`.
    pub password: Option<String>,
    /// A `Bearer` token.
    pub bearer: Option<String>,
    /// A header of the caller's naming carrying `value`, such as `X-Api-Key`.
    pub header: Option<String>,
    /// The value `header` carries.
    pub value: Option<String>,
}

fn authorization_of(auth: HttpAuth) -> Result<Authorization> {
    match auth {
        HttpAuth {
            username: Some(username),
            password,
            bearer: None,
            header: None,
            value: None,
        } => Ok(Authorization::basic(username, password.unwrap_or_default())),
        HttpAuth {
            username: None,
            password: None,
            bearer: Some(token),
            header: None,
            value: None,
        } => Ok(Authorization::bearer(token)),
        HttpAuth {
            username: None,
            password: None,
            bearer: None,
            header: Some(name),
            value: Some(value),
        } => Ok(Authorization::header(name, value)),
        _ => Err(napi_error(
            "auth must be exactly one of [username, password], { username, password }, \
             { bearer }, or { header, value }",
        )),
    }
}

/// What one request carries beyond its method and URL, as the loader
/// normalizes it: pairs as `[name, value]` entries, a JSON body as a `Scalar`,
/// durations in milliseconds.
#[napi(object, object_to_js = false)]
pub struct HttpRequestInit<'env> {
    /// Query pairs appended to the URL's own.
    pub params: Option<Vec<(String, String)>>,
    /// Headers set on the request, winning over a body's `Content-Type`.
    pub headers: Option<Vec<(String, String)>>,
    /// The whole-request timeout, in milliseconds.
    pub timeout: Option<f64>,
    /// Whether the body is left on the wire rather than read whole.
    pub stream: Option<bool>,
    /// The body, as bytes.
    pub data: Option<Uint8Array>,
    /// The body, as a compact JSON document under `application/json`.
    pub json: Option<ClassInstance<'env, JsScalar>>,
    /// The body, as a form under `application/x-www-form-urlencoded`.
    pub form: Option<Vec<(String, String)>>,
    /// The credential this request sends, whatever the session's is.
    pub auth: Option<HttpAuth>,
    /// Whether a `3xx` is followed, in place of the session's answer.
    pub follow_redirects: Option<bool>,
    /// How the next page is found: a `Pagination` spelling.
    pub pagination: Option<String>,
    /// Where a page's rows are in its document: a field path.
    pub records: Option<String>,
    /// What the resource is, whatever a response says.
    pub media_type: Option<MediaTypeInput<'env>>,
}

/// Shape `request` with everything `init` carries.
fn prepared(mut request: Request, init: Option<HttpRequestInit<'_>>) -> Result<Request> {
    let Some(init) = init else {
        return Ok(request);
    };
    let bodies = [
        init.data.is_some(),
        init.json.is_some(),
        init.form.is_some(),
    ];
    if bodies.into_iter().filter(|given| *given).count() > 1 {
        return Err(napi_error(
            "a request carries one body: pass data, json or form, not several",
        ));
    }
    if let Some(params) = init.params {
        request = request.with_query(params).map_err(napi_error)?;
    }
    if let Some(data) = init.data {
        request = request.with_body(data.to_vec());
    }
    if let Some(json) = init.json {
        request = request.with_json(&json.inner).map_err(napi_error)?;
    }
    if let Some(form) = init.form {
        request = request.with_form(form);
    }
    // Set after the body, one name at a time, so a caller's own
    // `Content-Type` replaces the one the body chose.
    if let Some(entries) = init.headers {
        for (name, value) in &headers_of(entries)? {
            request = request.with_header(name, value).map_err(napi_error)?;
        }
    }
    if let Some(auth) = init.auth {
        request = request.with_authorization(authorization_of(auth)?);
    }
    if let Some(timeout) = init.timeout {
        request = request.with_timeout(millis(timeout, "timeout")?);
    }
    if let Some(follow) = init.follow_redirects {
        request = request.with_follow_redirects(follow);
    }
    if let Some(pagination) = init.pagination {
        request = request.with_pagination(Pagination::from_str(&pagination).map_err(napi_error)?);
    }
    if let Some(records) = init.records {
        request = request.with_records(FieldPath::from_str(&records).map_err(napi_error)?);
    }
    if let Some(media_type) = init.media_type {
        request = request.with_media_type(media_type_from_input(media_type)?);
    }
    Ok(request)
}

/// A client's request counters.
#[napi(object)]
pub struct HttpStats {
    /// Every request, retries, resumes and redirect hops included.
    pub requests: f64,
    /// `GET` requests, whole or ranged.
    pub gets: f64,
    /// `HEAD` requests.
    pub heads: f64,
    /// `POST` requests.
    pub posts: f64,
    /// `PUT` requests.
    pub puts: f64,
    /// `PATCH` requests.
    pub patches: f64,
    /// `DELETE` requests.
    pub deletes: f64,
    /// `OPTIONS`, `TRACE` and `CONNECT` requests.
    pub others: f64,
    /// Attempts beyond the first of one request.
    pub retries: f64,
    /// Transfers re-opened after a body was cut.
    pub resumes: f64,
    /// Redirect hops followed.
    pub redirects: f64,
    /// What is left of the retry budget.
    pub retry_tokens: f64,
}

impl HttpStats {
    #[allow(clippy::cast_precision_loss)]
    fn from_core(stats: StatsSnapshot) -> Self {
        Self {
            requests: stats.requests as f64,
            gets: stats.gets as f64,
            heads: stats.heads as f64,
            posts: stats.posts as f64,
            puts: stats.puts as f64,
            patches: stats.patches as f64,
            deletes: stats.deletes as f64,
            others: stats.others as f64,
            retries: stats.retries as f64,
            resumes: stats.resumes as f64,
            redirects: stats.redirects as f64,
            retry_tokens: stats.retry_tokens as f64,
        }
    }
}

/// One cookie a jar holds or an answer set.
#[napi(object)]
pub struct HttpCookie {
    /// The cookie name.
    pub name: String,
    /// The cookie value, as the server spelled it.
    pub value: String,
    /// The host or cover domain, lower case.
    pub domain: String,
    /// Whether only `domain` itself receives the cookie.
    pub host_only: bool,
    /// The path prefix the cookie covers.
    pub path: String,
    /// When the cookie lapses, UTC nanoseconds; `null` for a session cookie.
    #[napi(ts_type = "bigint | null")]
    pub expires: Either<BigInt, Null>,
    /// Whether the cookie travels over `https` alone.
    pub secure: bool,
    /// Whether the server marked the cookie `HttpOnly`.
    pub http_only: bool,
}

impl HttpCookie {
    fn from_core(cookie: Cookie) -> Self {
        Self {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain,
            host_only: cookie.host_only,
            path: cookie.path,
            expires: cookie
                .expires
                .map_or(Either::B(Null), |expires| Either::A(BigInt::from(expires))),
            secure: cookie.secure,
            http_only: cookie.http_only,
        }
    }
}

/// One RFC 8288 link value of a `Link` header.
#[napi(object)]
pub struct HttpLink {
    /// The target, as the header spelled it.
    pub target: String,
    /// The relation types, lower case.
    pub rel: Vec<String>,
    /// Every other parameter, in header order.
    pub parameters: Vec<(String, String)>,
}

fn links_of(links: Vec<yggdryl::http::Link>) -> Vec<HttpLink> {
    links
        .into_iter()
        .map(|link| HttpLink {
            target: link.target,
            rel: link.rel.into_iter().map(|rel| rel.to_string()).collect(),
            parameters: link
                .parameters
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        })
        .collect()
}

/// A `Content-Range` value: the bytes an answer carries of the whole, or the
/// whole's length alone when the range asked for could not be satisfied.
#[napi(object)]
pub struct HttpContentRange {
    /// The first byte the answer carries; `null` when unsatisfied.
    pub start: Option<f64>,
    /// The last byte the answer carries, inclusive; `null` when unsatisfied.
    pub end: Option<f64>,
    /// The length of the whole, when the sender knows it.
    pub total: Option<f64>,
    /// Whether the range was satisfiable.
    pub satisfied: bool,
}

/// An entity tag.
#[napi(object)]
pub struct HttpETag {
    /// The opaque tag, without quotes.
    pub opaque: String,
    /// Whether the tag is weak (`W/`).
    pub weak: bool,
}

/// An immutable, case-insensitive HTTP header map.
///
/// Names read case-insensitively and list lower case, in lexical order; a
/// name given twice joins its values with `, ` (`Set-Cookie` apart, which
/// keeps each). The typed readers parse one header each: `null` when it is
/// absent, a thrown refusal when it is present and malformed.
#[napi(js_name = "Headers")]
pub struct JsHeaders {
    pub(crate) inner: Headers,
}

impl JsHeaders {
    pub(crate) fn from_core(inner: Headers) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsHeaders {
    /// A map of `entries`, each a `[name, value]` pair.
    #[napi(constructor)]
    pub fn new(entries: Option<Vec<(String, String)>>) -> Result<Self> {
        headers_of(entries.unwrap_or_default()).map(Self::from_core)
    }

    /// The value of `name`, or `null`.
    #[napi]
    pub fn get(&self, name: String) -> Option<String> {
        self.inner.get(&name).map(str::to_owned)
    }

    /// Every member of `name`: the comma-separated members of a list
    /// header, or each `Set-Cookie`.
    #[napi]
    pub fn get_all(&self, name: String) -> Vec<String> {
        self.inner.get_all(&name).map(str::to_owned).collect()
    }

    /// Whether `name` is present.
    #[napi]
    pub fn has(&self, name: String) -> bool {
        self.inner.contains_key(&name)
    }

    /// Every name, lower case, in lexical order.
    #[napi]
    pub fn keys(&self) -> Vec<String> {
        self.inner.iter().map(|(name, _)| name.to_owned()).collect()
    }

    /// Every `[name, value]` pair, in lexical order of the names.
    #[napi]
    pub fn entries(&self) -> Vec<(String, String)> {
        self.inner
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    /// How many names are present.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// Whether both maps hold the same names and values.
    #[napi]
    pub fn equals(&self, other: &JsHeaders) -> bool {
        self.inner == other.inner
    }

    /// The map as one plain object of lower-case names.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Map<String, serde_json::Value> {
        self.inner
            .iter()
            .map(|(name, value)| (name.to_owned(), serde_json::Value::from(value)))
            .collect()
    }

    /// The map as the JSON object text it renders to.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// `Content-Length`, in bytes.
    #[napi(getter)]
    pub fn content_length(&self) -> Result<Option<f64>> {
        count(
            self.inner.content_length().map_err(napi_error)?,
            "contentLength",
        )
    }

    /// `Content-Type`, as sent.
    #[napi(getter)]
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_owned)
    }

    /// The media type `Content-Type`, its charset and `Content-Encoding` state.
    #[napi(getter)]
    pub fn media_type(&self) -> Result<JsMediaType> {
        self.inner
            .media_type()
            .map(JsMediaType::from_core)
            .map_err(napi_error)
    }

    /// The charset `Content-Type` declares.
    #[napi(getter)]
    pub fn charset(&self) -> Result<Option<String>> {
        Ok(self
            .inner
            .charset()
            .map_err(napi_error)?
            .map(|charset| charset.as_str().to_owned()))
    }

    /// The content codings `Content-Encoding` lists, identity left out.
    #[napi(getter)]
    pub fn content_encoding(&self) -> Result<Vec<String>> {
        Ok(self
            .inner
            .content_encoding()
            .map_err(napi_error)?
            .into_iter()
            .map(|codec| codec.as_str().to_owned())
            .collect())
    }

    /// `Content-Range`.
    #[napi(getter)]
    pub fn content_range(&self) -> Result<Option<HttpContentRange>> {
        let Some(range) = self.inner.content_range().map_err(napi_error)? else {
            return Ok(None);
        };
        Ok(Some(match range {
            ContentRange::Bytes { start, end, total } => HttpContentRange {
                start: Some(crate::exact_f64(start, "contentRange.start")?),
                end: Some(crate::exact_f64(end, "contentRange.end")?),
                total: count(total, "contentRange.total")?,
                satisfied: true,
            },
            ContentRange::Unsatisfied { total } => HttpContentRange {
                start: None,
                end: None,
                total: Some(crate::exact_f64(total, "contentRange.total")?),
                satisfied: false,
            },
        }))
    }

    /// Whether `Accept-Ranges` offers `bytes`.
    #[napi(getter)]
    pub fn accept_ranges(&self) -> bool {
        self.inner.accept_ranges()
    }

    /// `ETag`.
    #[napi(getter)]
    pub fn etag(&self) -> Result<Option<HttpETag>> {
        Ok(self.inner.etag().map_err(napi_error)?.map(|tag| HttpETag {
            weak: tag.is_weak(),
            opaque: tag.opaque,
        }))
    }

    /// `Last-Modified`, UTC nanoseconds since the epoch.
    #[napi(getter)]
    pub fn last_modified(&self) -> Result<Option<BigInt>> {
        Ok(self
            .inner
            .last_modified()
            .map_err(napi_error)?
            .map(BigInt::from))
    }

    /// `Date`, UTC nanoseconds since the epoch.
    #[napi(getter)]
    pub fn date(&self) -> Result<Option<BigInt>> {
        Ok(self.inner.date().map_err(napi_error)?.map(BigInt::from))
    }

    /// `Location`, as sent.
    #[napi(getter)]
    pub fn location(&self) -> Option<String> {
        self.inner.location().map(str::to_owned)
    }

    /// The `Link` header's link values.
    #[napi(getter)]
    pub fn links(&self) -> Result<Vec<HttpLink>> {
        self.inner.links().map(links_of).map_err(napi_error)
    }

    /// The target of the `Link` value whose `rel` holds `next`.
    #[napi(getter)]
    pub fn next_link(&self) -> Result<Option<String>> {
        Ok(self
            .inner
            .next_link()
            .map_err(napi_error)?
            .map(str::to_owned))
    }

    /// Every `Set-Cookie` value, as sent.
    #[napi(getter)]
    pub fn set_cookies(&self) -> Vec<String> {
        self.inner
            .set_cookies()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

/// An HTTP session: the options every request starts from, one cookie jar
/// and the client requests go out on.
#[napi(js_name = "Session")]
pub struct JsSession {
    pub(crate) inner: Session,
}

/// What a session is built with, as the loader normalizes it.
#[napi(object, object_to_js = false)]
pub struct HttpSessionInit {
    /// Headers every request carries unless it sets its own.
    pub headers: Option<Vec<(String, String)>>,
    /// The credential every request sends unless it names its own.
    pub auth: Option<HttpAuth>,
    /// The whole-request timeout, in milliseconds.
    pub timeout: Option<f64>,
    /// `HttpOptions` properties by name (`max_attempts`, `pagination`,
    /// `header.<name>`...), each value as text.
    pub options: Option<Vec<(String, String)>>,
}

#[napi]
impl JsSession {
    /// A session joining relative URLs onto `baseUrl`; the named arguments
    /// win over the same knob in `options`.
    #[napi(constructor)]
    pub fn new(base_url: Option<String>, init: Option<HttpSessionInit>) -> Result<Self> {
        let init = init.unwrap_or(HttpSessionInit {
            headers: None,
            auth: None,
            timeout: None,
            options: None,
        });
        let mut options =
            HttpOptions::from_properties(init.options.unwrap_or_default()).map_err(napi_error)?;
        if let Some(base_url) = base_url {
            options = options.with_base_url(Url::from_str(&base_url).map_err(napi_error)?);
        }
        if let Some(entries) = init.headers {
            let given = headers_of(entries)?;
            let merged = given.merge_with(options.headers()).map_err(napi_error)?;
            options = options.with_headers(merged);
        }
        if let Some(auth) = init.auth {
            options = options.with_authorization(authorization_of(auth)?);
        }
        if let Some(timeout) = init.timeout {
            options = options.with_timeout(millis(timeout, "timeout")?);
        }
        Session::with_options(options)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The process-wide default session the `http` doors send on.
    #[napi(factory, js_name = "_defaultNative", skip_typescript)]
    pub fn default_session() -> Self {
        Self {
            inner: yggdryl::http::session(),
        }
    }

    /// A request of `method` at `url` on this session, shaped by `init`.
    #[napi(js_name = "_prepareNative", skip_typescript)]
    pub fn prepare(
        &self,
        method: String,
        url: String,
        init: Option<HttpRequestInit<'_>>,
    ) -> Result<JsRequest> {
        let request = self
            .inner
            .request(method_of(&method)?, &url)
            .map_err(napi_error)?;
        prepared(request, init).map(|inner| JsRequest { inner })
    }

    /// Send every request on up to `concurrency` threads - the session's
    /// own unless given - as a walk answering in the order given, each
    /// answer pulled when asked for.
    #[napi(js_name = "_sendAllNative", skip_typescript)]
    pub fn send_all(
        &self,
        requests: Vec<ClassInstance<'_, JsRequest>>,
        concurrency: Option<u32>,
    ) -> Result<JsSendAllAnswers> {
        let requests: Vec<Request> = requests
            .iter()
            .map(|request| request.inner.clone())
            .collect();
        let concurrency = concurrency
            .map(usize::try_from)
            .transpose()
            .map_err(napi_error)?;
        Ok(JsSendAllAnswers {
            answers: Box::new(self.inner.send_all(requests, concurrency)),
        })
    }

    /// The URL a relative request joins onto.
    #[napi(getter)]
    pub fn base_url(&self) -> Option<JsUrl> {
        self.inner.base_url().cloned().map(JsUrl::from_core)
    }

    /// The headers every request carries unless it sets its own.
    #[napi(getter)]
    pub fn headers(&self) -> JsHeaders {
        JsHeaders::from_core(self.inner.options().headers().clone())
    }

    /// The whole-request timeout, in milliseconds.
    #[napi(getter)]
    pub fn timeout(&self) -> f64 {
        as_millis(self.inner.options().timeout())
    }

    /// The client's request counters.
    #[napi(getter)]
    pub fn stats(&self) -> HttpStats {
        HttpStats::from_core(self.inner.stats())
    }

    /// Every cookie the jar holds.
    #[napi(getter)]
    pub fn cookies(&self) -> Vec<HttpCookie> {
        self.inner
            .cookies()
            .into_iter()
            .map(HttpCookie::from_core)
            .collect()
    }

    /// Store a cookie of `name` and `value` covering `domain` and every path.
    #[napi]
    pub fn set_cookie(&self, name: String, value: String, domain: String) {
        self.inner.set_cookie(Cookie::new(name, value, &domain));
    }

    /// Evict the cookies that have lapsed.
    #[napi]
    pub fn close(&self) {
        self.inner.close();
    }

    /// This session as the `IOBase` container over its base URL: a path
    /// below it is the `GET` of that resource.
    #[napi(js_name = "intoIOBase")]
    pub fn into_iobase(&self) -> JsIOBase {
        JsIOBase::from_core(Holder::HttpSession(self.inner.clone()))
    }

    /// The session as `Session(<base URL>)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        match self.inner.base_url() {
            Some(url) => format!("Session({url})"),
            None => "Session()".to_owned(),
        }
    }
}

/// One HTTP request, and the resource its URL names.
///
/// A request is a value: every `with` builder answers a new request and
/// leaves this one as it was. Nothing goes out until `send`, `stream`,
/// `pages` or a read of `intoIOBase()` asks.
#[napi(js_name = "Request")]
pub struct JsRequest {
    pub(crate) inner: Request,
}

#[napi]
impl JsRequest {
    /// A `method` request at `url` on the process-wide default session.
    #[napi(constructor)]
    pub fn new(method: String, url: String) -> Result<Self> {
        yggdryl::http::session()
            .request(method_of(&method)?, &url)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// Parse one request message: request line, headers, framed body.
    #[napi(factory)]
    pub fn from_bytes(wire: Uint8Array) -> Result<Self> {
        Request::from_bytes(&wire)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// Render the request message.
    #[napi]
    pub fn into_bytes(&self) -> Result<Buffer> {
        self.inner
            .into_bytes()
            .map(Buffer::from)
            .map_err(napi_error)
    }

    /// The method, upper case.
    #[napi(getter)]
    pub fn method(&self) -> String {
        self.inner.method().as_str().to_owned()
    }

    /// The URL, query included.
    #[napi(getter)]
    pub fn url(&self) -> JsUrl {
        JsUrl::from_core(self.inner.url().clone())
    }

    /// The request's own headers, before the session's defaults join them.
    #[napi(getter)]
    pub fn headers(&self) -> JsHeaders {
        JsHeaders::from_core(self.inner.headers().clone())
    }

    /// The body.
    #[napi(getter)]
    pub fn body(&self) -> Buffer {
        Buffer::from(self.inner.body().as_bytes().to_vec())
    }

    /// The session the request goes out on.
    #[napi(getter)]
    pub fn session(&self) -> JsSession {
        JsSession {
            inner: self.inner.session().clone(),
        }
    }

    /// The session's request counters.
    #[napi(getter)]
    pub fn stats(&self) -> HttpStats {
        HttpStats::from_core(self.inner.stats())
    }

    /// The same request bound to `session`.
    #[napi]
    pub fn with_session(&self, session: &JsSession) -> Self {
        Self {
            inner: self.inner.clone().with_session(session.inner.clone()),
        }
    }

    /// The same request with one header set, replacing the value it had.
    #[napi]
    pub fn with_header(&self, name: String, value: String) -> Result<Self> {
        self.inner
            .clone()
            .with_header(&name, &value)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The same request with `entries` merged under its own headers.
    #[napi(js_name = "_withHeadersNative", skip_typescript)]
    pub fn with_headers(&self, entries: Vec<(String, String)>) -> Result<Self> {
        Ok(Self {
            inner: self.inner.clone().with_headers(headers_of(entries)?),
        })
    }

    /// The same request with `entries` appended to its query.
    #[napi(js_name = "_withQueryNative", skip_typescript)]
    pub fn with_query(&self, entries: Vec<(String, String)>) -> Result<Self> {
        self.inner
            .clone()
            .with_query(entries)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The same request carrying `body`.
    #[napi(js_name = "_withBodyNative", skip_typescript)]
    pub fn with_body(&self, body: Uint8Array) -> Self {
        Self {
            inner: self.inner.clone().with_body(body.to_vec()),
        }
    }

    /// The same request carrying `value` as a JSON body.
    #[napi(js_name = "_withJsonNative", skip_typescript)]
    pub fn with_json(&self, value: &JsScalar) -> Result<Self> {
        self.inner
            .clone()
            .with_json(&value.inner)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The same request carrying `entries` as a form body.
    #[napi(js_name = "_withFormNative", skip_typescript)]
    pub fn with_form(&self, entries: Vec<(String, String)>) -> Self {
        Self {
            inner: self.inner.clone().with_form(entries),
        }
    }

    /// The same request sending `auth`, whatever the session's is.
    #[napi(js_name = "_withAuthorizationNative", skip_typescript)]
    pub fn with_authorization(&self, auth: HttpAuth) -> Result<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .with_authorization(authorization_of(auth)?),
        })
    }

    /// The same request with its own whole-request timeout, in milliseconds.
    #[napi]
    pub fn with_timeout(&self, timeout: f64) -> Result<Self> {
        Ok(Self {
            inner: self.inner.clone().with_timeout(millis(timeout, "timeout")?),
        })
    }

    /// The same request following a `3xx`, or not, whatever the session says.
    #[napi]
    pub fn with_follow_redirects(&self, follow: bool) -> Self {
        Self {
            inner: self.inner.clone().with_follow_redirects(follow),
        }
    }

    /// The same request finding its next page per `pagination`: `auto`,
    /// `none`, `link`, `header:<name>`, `url:<path>`,
    /// `cursor:<path>:<parameter>`, `offset:<parameter>:<size>[:<total>]`
    /// or `page:<parameter>[:<start>]`.
    #[napi]
    pub fn with_pagination(&self, pagination: String) -> Result<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .with_pagination(Pagination::from_str(&pagination).map_err(napi_error)?),
        })
    }

    /// The same request reading a page's rows at the field path `records`.
    #[napi]
    pub fn with_records(&self, records: String) -> Result<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .with_records(FieldPath::from_str(&records).map_err(napi_error)?),
        })
    }

    /// The same request declaring what its resource is.
    #[napi]
    pub fn with_media_type(&self, media_type: MediaTypeInput<'_>) -> Result<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .with_media_type(media_type_from_input(media_type)?),
        })
    }

    /// Send, reading the whole body.
    #[napi]
    pub fn send(&self) -> Result<JsResponse> {
        self.inner
            .send()
            .map(JsResponse::from_core)
            .map_err(napi_error)
    }

    /// Send, leaving the body on the wire as a resumable stream.
    #[napi]
    pub fn stream(&self) -> Result<JsResponse> {
        self.inner
            .stream()
            .map(JsResponse::from_core)
            .map_err(napi_error)
    }

    /// Walk the pages of this resource per its pagination.
    #[napi]
    pub fn pages(&self) -> JsPages {
        JsPages {
            inner: Some(self.inner.pages()),
        }
    }

    /// The resource this request names, as an `IOBase` leaf: a read is a
    /// `GET`, a ranged read a ranged `GET`, a write a `PUT`, a removal a
    /// `DELETE`, each carrying this request's headers.
    #[napi(js_name = "intoIOBase")]
    pub fn into_iobase(&self) -> JsIOBase {
        JsIOBase::from_core(Holder::HttpRequest(self.inner.clone()))
    }

    /// The request as `<METHOD> <URL>`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!("{} {}", self.inner.method(), self.inner.url())
    }
}

/// One redirect hop of an answer's history.
#[napi(object, object_from_js = false)]
pub struct HttpHop {
    /// The status code.
    pub status_code: u32,
    /// The status's reason phrase.
    pub reason: String,
    /// The URL that answered.
    pub url: String,
    /// The headers it answered with.
    pub headers: JsHeaders,
}

/// One HTTP answer and its body.
///
/// The body is read whole (`send`) or left on the wire (`stream`); either way
/// `content()` is the decoded bytes, `text()` the text and `scalar()` the
/// document. `intoIOBase()` moves the answer into the `IOBase` over its body
/// as sent, after which this object answers nothing.
#[napi(js_name = "Response")]
pub struct JsResponse {
    inner: Option<Response>,
}

impl JsResponse {
    pub(crate) fn from_core(inner: Response) -> Self {
        Self { inner: Some(inner) }
    }

    fn held(&self) -> Result<&Response> {
        self.inner
            .as_ref()
            .ok_or_else(|| napi_error("this Response was moved into an IOBase by intoIOBase()"))
    }

    fn held_mut(&mut self) -> Result<&mut Response> {
        self.inner
            .as_mut()
            .ok_or_else(|| napi_error("this Response was moved into an IOBase by intoIOBase()"))
    }
}

#[napi]
impl JsResponse {
    /// Let go of a streaming body's transfer and its connection, keeping
    /// the cursor: the response stands alone, so a later read re-opens where
    /// this one stopped. A held body has nothing to let go.
    #[napi]
    pub fn close(&mut self) -> Result<()> {
        yggdryl::IOBase::close(self.held_mut()?).map_err(napi_error)
    }

    /// Re-open a closed streaming body at its cursor now: one ranged `GET`
    /// naming the first answer's validator.
    #[napi]
    pub fn open(&mut self) -> Result<()> {
        yggdryl::IOBase::open(self.held_mut()?).map_err(napi_error)
    }

    /// Whether a live transfer is held.
    #[napi(getter)]
    pub fn opened(&self) -> Result<bool> {
        Ok(yggdryl::IOBase::opened(self.held()?))
    }

    /// Parse one response message: status line, headers, framed body.
    #[napi(factory)]
    pub fn from_bytes(wire: Uint8Array) -> Result<Self> {
        Response::from_bytes(&wire)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Render the response message, its body as sent.
    #[napi]
    pub fn into_bytes(&self) -> Result<Buffer> {
        self.held()?
            .into_bytes()
            .map(Buffer::from)
            .map_err(napi_error)
    }

    /// The response as one record: `status`, `reason`, `version`, `url`,
    /// `headers` and `body`.
    #[napi]
    pub fn into_scalar(&self) -> Result<JsScalar> {
        self.held()?
            .into_scalar()
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// The status code.
    #[napi(getter)]
    pub fn status_code(&self) -> Result<u32> {
        Ok(u32::from(self.held()?.status().code()))
    }

    /// The status's reason phrase.
    #[napi(getter)]
    pub fn reason(&self) -> Result<String> {
        Ok(self.held()?.status().reason().to_owned())
    }

    /// Whether the status is below 400.
    #[napi(getter)]
    pub fn ok(&self) -> Result<bool> {
        Ok(self.held()?.is_ok())
    }

    /// Whether the status is a redirect.
    #[napi(getter)]
    pub fn is_redirect(&self) -> Result<bool> {
        Ok(self.held()?.is_redirect())
    }

    /// The HTTP version the answer stated.
    #[napi(getter)]
    pub fn version(&self) -> Result<String> {
        Ok(self.held()?.version().as_str().to_owned())
    }

    /// The final URL, after redirects.
    #[napi(getter)]
    pub fn url(&self) -> Result<JsUrl> {
        Ok(JsUrl::from_core(self.held()?.url().clone()))
    }

    /// The headers.
    #[napi(getter)]
    pub fn headers(&self) -> Result<JsHeaders> {
        Ok(JsHeaders::from_core(self.held()?.headers().clone()))
    }

    /// The request this answers.
    #[napi(getter)]
    pub fn request(&self) -> Result<JsRequest> {
        Ok(JsRequest {
            inner: self.held()?.request().clone(),
        })
    }

    /// The redirect hops before this answer, oldest first.
    #[napi(getter)]
    pub fn history(&self) -> Result<Vec<HttpHop>> {
        Ok(self
            .held()?
            .history()
            .iter()
            .map(|hop| HttpHop {
                status_code: u32::from(hop.status().code()),
                reason: hop.status().reason().to_owned(),
                url: hop.url().to_string(),
                headers: JsHeaders::from_core(hop.headers().clone()),
            })
            .collect())
    }

    /// How long the exchange took, redirects and retries included, in
    /// milliseconds.
    #[napi(getter)]
    pub fn elapsed(&self) -> Result<f64> {
        Ok(as_millis(self.held()?.elapsed()))
    }

    /// The `Link` header's link values.
    #[napi(getter)]
    pub fn links(&self) -> Result<Vec<HttpLink>> {
        self.held()?.links().map(links_of).map_err(napi_error)
    }

    /// The request for the next page per the request's pagination - the
    /// same request at the next URL - or `null` on the last; reading it
    /// reads the body.
    #[napi(getter)]
    pub fn next(&self) -> Result<Option<JsRequest>> {
        Ok(self
            .held()?
            .next_request()
            .map_err(napi_error)?
            .map(|inner| JsRequest { inner }))
    }

    /// The charset `Content-Type` declares.
    #[napi(getter)]
    pub fn encoding(&self) -> Result<Option<String>> {
        Ok(self
            .held()?
            .encoding()
            .map(|charset| charset.as_str().to_owned()))
    }

    /// The media type the headers state.
    #[napi(getter)]
    pub fn media_type(&self) -> Result<JsMediaType> {
        Ok(JsMediaType::from_core(self.held()?.media_type().clone()))
    }

    /// The `Content-Length` the headers state.
    #[napi(getter)]
    pub fn content_length(&self) -> Result<Option<f64>> {
        count(self.held()?.content_length(), "contentLength")
    }

    /// Every `Set-Cookie` the answer carried that parses.
    #[napi(getter)]
    pub fn cookies(&self) -> Result<Vec<HttpCookie>> {
        Ok(self
            .held()?
            .cookies()
            .into_iter()
            .map(HttpCookie::from_core)
            .collect())
    }

    /// The whole decoded body.
    #[napi]
    pub fn content(&self) -> Result<Buffer> {
        Ok(Buffer::from(
            self.held()?.bytes().map_err(napi_error)?.to_vec(),
        ))
    }

    /// The decoded body as text, in the charset `Content-Type` declares.
    #[napi]
    pub fn text(&self) -> Result<String> {
        self.held()?.text().map_err(napi_error)
    }

    /// The body parsed under its media type - JSON, JSON Lines, YAML, TOML or
    /// XML - and, when `field` is given, read under it.
    #[napi]
    pub fn scalar(&self, field: Option<FieldInput<'_>>) -> Result<JsScalar> {
        let response = self.held()?;
        let value = match field {
            Some(field) => response.scalar_with_field(&field_from_input(field)?),
            None => response.scalar(),
        };
        value.map(JsScalar::from_core).map_err(napi_error)
    }

    /// Throw the refusal a status of 400 or more is.
    #[napi(js_name = "_raiseForStatusNative", skip_typescript)]
    pub fn raise_for_status(&self) -> Result<()> {
        self.held()?
            .raise_for_status()
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Move this answer into the `IOBase` over its body as sent: reads,
    /// ranged reads and the record readers answer from it.
    #[napi(js_name = "intoIOBase")]
    pub fn into_iobase(&mut self) -> Result<JsIOBase> {
        self.held()?;
        let response = self.inner.take().ok_or_else(|| napi_error("unreachable"))?;
        Ok(JsIOBase::from_core(response.into_holder()))
    }

    /// The response as `Response(<status> <URL>)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        match &self.inner {
            Some(response) => format!("Response({} {})", response.status(), response.url()),
            None => "Response(moved)".to_owned(),
        }
    }
}

/// The answers of `Session.sendAll`, in the order the requests were given:
/// requests are in flight on the session's threads while the caller reads
/// the ones before them.
#[napi(js_name = "SendAllAnswers")]
pub struct JsSendAllAnswers {
    answers: Box<dyn Iterator<Item = yggdryl::Result<Response>> + Send>,
}

#[napi]
impl JsSendAllAnswers {
    /// The next answer - a `Response`, or the failure's message in its place
    /// - or `null` once every request answered.
    #[napi(js_name = "_nextNative", skip_typescript)]
    pub fn next_answer(&mut self) -> Option<Either<JsResponse, String>> {
        self.answers.next().map(|answer| match answer {
            Ok(response) => Either::A(JsResponse::from_core(response)),
            Err(error) => Either::B(error.to_string()),
        })
    }
}

/// A walk of a paginated resource, one `Response` per page.
///
/// A page answering 400 or more is yielded and ends the walk; a transport
/// failure throws where the page would have been.
#[napi(js_name = "Pages")]
pub struct JsPages {
    inner: Option<Pages>,
}

impl JsPages {
    fn walk(&mut self) -> Result<&mut Pages> {
        self.inner.as_mut().ok_or_else(|| {
            napi_error("these Pages were consumed by intoArrowReader; a walk is read once")
        })
    }
}

#[napi]
impl JsPages {
    /// The next page, or `null` once the walk is over.
    #[napi(js_name = "_nextNative", skip_typescript)]
    pub fn next_page(&mut self) -> Result<Option<JsResponse>> {
        match self.walk()?.next() {
            None => Ok(None),
            Some(Ok(page)) => Ok(Some(JsResponse::from_core(page))),
            Some(Err(error)) => Err(napi_error(error)),
        }
    }

    /// Every remaining page as Arrow batches under one root: `field` when
    /// given, else the record the first page's rows infer. One batch per page,
    /// split at `batchRowSize` rows when given.
    #[napi]
    pub fn into_arrow_reader(
        &mut self,
        field: Option<FieldInput<'_>>,
        batch_row_size: Option<f64>,
    ) -> Result<JsBatchReader> {
        let field = field.map(field_from_input).transpose()?;
        let chunk = match batch_row_size {
            Some(rows) => usize::try_from(exact_u64(rows, "batchRowSize")?).map_err(napi_error)?,
            None => 0,
        };
        self.walk()?;
        let pages = self.inner.take().ok_or_else(|| napi_error("unreachable"))?;
        let root_name = field
            .as_ref()
            .map_or(DEFAULT_ROOT_NAME, yggdryl::Field::name)
            .to_owned();
        let reader = pages
            .into_arrow_reader(field.as_ref(), chunk)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }
}

/// How a server reads, frames and answers.
#[napi(object, object_to_js = false)]
pub struct HttpServerOptions {
    /// How long a connection waits for the next byte of a request, and the
    /// longest a request head may take, in milliseconds.
    pub read_timeout: Option<f64>,
    /// How long writing an answer waits on a peer that stopped reading, in
    /// milliseconds.
    pub write_timeout: Option<f64>,
    /// The most connections served at once; past it a connection is closed
    /// unread.
    pub max_connections: Option<f64>,
    /// The most bytes a request head may take.
    pub max_head_size: Option<f64>,
    /// The most bytes a request body may take; a larger one is `413`.
    pub max_body_size: Option<f64>,
    /// Whether a connection serves several requests.
    pub keep_alive: Option<bool>,
    /// Whether handled requests are kept in `requests`.
    pub recording: Option<bool>,
    /// Whether a mounted leaf is served with an `ETag`.
    pub etag: Option<bool>,
    /// Whether a `CONNECT` is tunnelled, the server standing in for a
    /// forward proxy.
    pub tunnel: Option<bool>,
    /// The `Server` header every answer carries.
    pub server_header: Option<String>,
}

fn server_options_of(input: HttpServerOptions) -> Result<ServerOptions> {
    let mut options = ServerOptions::default();
    if let Some(timeout) = input.read_timeout {
        options = options.with_read_timeout(millis(timeout, "readTimeout")?);
    }
    if let Some(timeout) = input.write_timeout {
        options = options.with_write_timeout(millis(timeout, "writeTimeout")?);
    }
    if let Some(connections) = input.max_connections {
        options = options.with_max_connections(
            usize::try_from(exact_u64(connections, "maxConnections")?).map_err(napi_error)?,
        );
    }
    if let Some(size) = input.max_head_size {
        options = options.with_max_head_size(
            usize::try_from(exact_u64(size, "maxHeadSize")?).map_err(napi_error)?,
        );
    }
    if let Some(size) = input.max_body_size {
        options = options.with_max_body_size(exact_u64(size, "maxBodySize")?);
    }
    if let Some(keep_alive) = input.keep_alive {
        options = options.with_keep_alive(keep_alive);
    }
    if let Some(recording) = input.recording {
        options = options.with_recording(recording);
    }
    if let Some(etag) = input.etag {
        options = options.with_etag(etag);
    }
    if let Some(tunnel) = input.tunnel {
        options = options.with_tunnel(tunnel);
    }
    if let Some(header) = input.server_header {
        options = options.with_server_header(header);
    }
    Ok(options)
}

/// A fault a server applies to a path's next requests, in one of four
/// spellings: `'closeBeforeAnswer'`, `{ cutBodyAt }`, `{ refuse, retryAfter }`
/// or `{ delay }`.
#[napi(object, object_to_js = false)]
pub struct HttpFault {
    /// Write the head and this many body bytes, then close the connection.
    pub cut_body_at: Option<f64>,
    /// Answer this status with an empty body.
    pub refuse: Option<u32>,
    /// With `refuse`: the `Retry-After` delay, in milliseconds.
    pub retry_after: Option<f64>,
    /// Sleep this many milliseconds before answering.
    pub delay: Option<f64>,
}

fn fault_of(fault: Either<String, HttpFault>) -> Result<Fault> {
    match fault {
        Either::A(name) if name == "closeBeforeAnswer" => Ok(Fault::CloseBeforeAnswer),
        Either::A(name) => Err(napi_error(format!(
            "expected the fault 'closeBeforeAnswer', {{ cutBodyAt }}, {{ refuse, retryAfter }} or {{ delay }}, got {name:?}"
        ))),
        Either::B(HttpFault {
            cut_body_at: Some(bytes),
            refuse: None,
            retry_after: None,
            delay: None,
        }) => Ok(Fault::CutBodyAt(exact_u64(bytes, "cutBodyAt")?)),
        Either::B(HttpFault {
            cut_body_at: None,
            refuse: Some(status),
            retry_after,
            delay: None,
        }) => Ok(Fault::Refuse {
            status: status_of(status)?,
            retry_after: retry_after
                .map(|pause| millis(pause, "retryAfter"))
                .transpose()?,
        }),
        Either::B(HttpFault {
            cut_body_at: None,
            refuse: None,
            retry_after: None,
            delay: Some(pause),
        }) => Ok(Fault::Delay(millis(pause, "delay")?)),
        Either::B(_) => Err(napi_error(
            "a fault is exactly one of { cutBodyAt }, { refuse, retryAfter } or { delay }",
        )),
    }
}

/// One request a server handled.
#[napi(object, object_from_js = false)]
pub struct HttpRecorded {
    /// The method, upper case.
    pub method: String,
    /// The request target exactly as sent, query included.
    pub target: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    /// The query pairs, percent-decoded, in wire order.
    pub query: Vec<(String, String)>,
    /// The request headers.
    pub headers: JsHeaders,
    /// The request body's length in bytes.
    pub body_length: f64,
    /// The status answered; `499` when the connection closed unanswered.
    pub status_code: u32,
    /// Whether the connection was closed before any byte of an answer.
    pub closed: bool,
}

impl HttpRecorded {
    fn from_core(recorded: Recorded) -> Result<Self> {
        Ok(Self {
            closed: recorded.is_closed(),
            method: recorded.method.as_str().to_owned(),
            target: recorded.target,
            path: recorded.path,
            query: recorded.query,
            headers: JsHeaders::from_core(recorded.headers),
            body_length: crate::exact_f64(recorded.body_len, "bodyLength")?,
            status_code: u32::from(recorded.status.code()),
        })
    }
}

/// An HTTP/1.1 server hosting `IOBase` handles and fixed answers, answering
/// from its own threads.
///
/// A route answered by a JavaScript callback is not offered: the connection
/// thread would have to wait on the JavaScript thread, which deadlocks the
/// moment that thread is the one waiting on the answer - a synchronous
/// request from the same isolate.
#[napi(js_name = "Server")]
pub struct JsServer {
    inner: Option<Server>,
}

impl JsServer {
    fn live(&self) -> Result<&Server> {
        self.inner
            .as_ref()
            .ok_or_else(|| napi_error("this Server was shut down"))
    }
}

#[napi]
impl JsServer {
    /// Bind `address` (`127.0.0.1:0`, any loopback port, by default) and
    /// start accepting.
    #[napi(factory)]
    pub fn bind(address: Option<String>, options: Option<HttpServerOptions>) -> Result<Self> {
        let address = address.as_deref().unwrap_or("127.0.0.1:0");
        let options = options
            .map(server_options_of)
            .transpose()?
            .unwrap_or_default();
        Server::bind_with(address, options)
            .map(|server| Self {
                inner: Some(server),
            })
            .map_err(napi_error)
    }

    /// `http://<address>/`.
    #[napi(getter)]
    pub fn url(&self) -> Result<JsUrl> {
        Ok(JsUrl::from_core(self.live()?.url().clone()))
    }

    /// The bound port.
    #[napi(getter)]
    pub fn port(&self) -> Result<u32> {
        Ok(u32::from(self.live()?.port()))
    }

    /// The bound address, `host:port`.
    #[napi(getter)]
    pub fn address(&self) -> Result<String> {
        Ok(self.live()?.address().to_string())
    }

    /// Connections accepted so far.
    #[napi(getter)]
    pub fn connections(&self) -> Result<f64> {
        crate::exact_f64(self.live()?.connections(), "connections")
    }

    /// Serve the location `handle` names under `prefix`: the prefix itself
    /// is the handle, a path below it the child at that path. A second
    /// handle on the same location is mounted, so `handle` stays usable; an
    /// in-memory handle, which has no location, is served as a copy of its
    /// bytes under its media type.
    #[napi]
    pub fn mount(&self, prefix: String, handle: &JsIOBase) -> Result<()> {
        let holder = handle.mountable()?;
        self.live()?.mount(&prefix, holder).map_err(napi_error)
    }

    /// Stop serving `prefix`; whether something was mounted there.
    #[napi]
    pub fn unmount(&self, prefix: String) -> Result<bool> {
        Ok(self.live()?.unmount(&prefix))
    }

    /// Serve `path` under its mount as `mediaType`, whatever its name says.
    #[napi]
    pub fn set_media_type(&self, path: String, media_type: MediaTypeInput<'_>) -> Result<()> {
        self.live()?
            .set_media_type(&path, media_type_from_input(media_type)?)
            .map_err(napi_error)
    }

    /// Answer `path` with `status`, `headers` and `body` every time.
    #[napi(js_name = "_respondNative", skip_typescript)]
    pub fn respond(
        &self,
        path: String,
        status: u32,
        headers: Vec<(String, String)>,
        body: Option<Uint8Array>,
        method: Option<String>,
    ) -> Result<()> {
        let method = method.as_deref().map(method_of).transpose()?;
        let mut response = Response::new(status_of(status)?)
            .with_headers(headers_of(headers)?)
            .map_err(napi_error)?;
        if let Some(body) = body {
            response = response.with_body(body.to_vec());
        }
        self.live()?.respond(method, &path, response);
        Ok(())
    }

    /// Forget the answer of `method` (every method when omitted) at `path`;
    /// whether there was one.
    #[napi]
    pub fn unroute(&self, path: String, method: Option<String>) -> Result<bool> {
        let method = method.as_deref().map(method_of).transpose()?;
        Ok(self.live()?.unroute(method, &path))
    }

    /// Apply `fault` to the next `times` requests of `path` (one by default,
    /// `0` for every request), before any answer or mount.
    #[napi]
    pub fn inject(
        &self,
        path: String,
        #[napi(ts_arg_type = "'closeBeforeAnswer' | HttpFault")] fault: Either<String, HttpFault>,
        times: Option<u32>,
    ) -> Result<()> {
        let fault = fault_of(fault)?;
        self.live()?.inject(&path, fault, times.unwrap_or(1));
        Ok(())
    }

    /// Forget every injected fault.
    #[napi]
    pub fn clear_faults(&self) -> Result<()> {
        self.live()?.clear_faults();
        Ok(())
    }

    /// Every request handled while recording, in order.
    #[napi(getter)]
    pub fn requests(&self) -> Result<Vec<HttpRecorded>> {
        self.live()?
            .requests()
            .into_iter()
            .map(HttpRecorded::from_core)
            .collect()
    }

    /// Requests handled since the last `clearRequests`, recording or not.
    #[napi(getter)]
    pub fn request_count(&self) -> Result<f64> {
        let count = u64::try_from(self.live()?.request_count()).unwrap_or(u64::MAX);
        crate::exact_f64(count, "requestCount")
    }

    /// Forget the recorded requests and zero the count.
    #[napi]
    pub fn clear_requests(&self) -> Result<()> {
        self.live()?.clear_requests();
        Ok(())
    }

    /// Keep handled requests in `requests` (`true`) or only count them.
    #[napi]
    pub fn set_recording(&self, on: bool) -> Result<()> {
        self.live()?.set_recording(on);
        Ok(())
    }

    /// Whether `shutdown` has run.
    #[napi(getter)]
    pub fn closed(&self) -> bool {
        self.inner.is_none()
    }

    /// Stop accepting and join the accept thread; connections being served
    /// finish their request. A second call does nothing.
    #[napi]
    pub fn shutdown(&mut self) -> Result<()> {
        match self.inner.take() {
            Some(server) => server.shutdown().map_err(napi_error),
            None => Ok(()),
        }
    }

    /// The server as `Server(<URL>)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        match &self.inner {
            Some(server) => format!("Server({})", server.url()),
            None => "Server(shut down)".to_owned(),
        }
    }
}
