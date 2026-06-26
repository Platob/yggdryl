//! The `HttpSession` and `HttpResponse` napi classes (a `requests`-like client).
//!
//! Requests run on the libuv thread pool and return a `Promise`, so a blocking
//! HTTP call never stalls the Node event loop (and same-process test servers
//! answer normally).

use std::collections::HashMap;
use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi::{Env, Task};
use napi_derive::napi;
use yggdryl_core::{
    BytesIO as CoreBytesIO, Compression as CoreCompression, LocalPath as CoreLocalPath,
    MimeType as CoreMimeType, Path, Url as CoreUrl,
};

use crate::bytesio::BytesIO;
use yggdryl_http::{
    HttpRequest as CoreHttpRequest, HttpSession as CoreHttpSession, HttpVersion, Method,
};

use crate::localpath::LocalPath;

fn to_napi(err: yggdryl_http::HttpError) -> Error {
    Error::from_reason(err.to_string())
}

/// The process-wide shared session that backs the module-level `get` / `post` / …
/// verbs (the singleton mirroring Python's `requests` default session). Replaceable
/// with `setBaseUrl` to give those verbs a base URL.
fn shared_session() -> Arc<CoreHttpSession> {
    CoreHttpSession::shared()
}

/// Builds a [`RequestTask`] from the full set of verb arguments, parsing the
/// JS-typed inputs (method, HTTP version, basic-auth pair). The single place every
/// verb — instance and module-level — turns its signature args into a task.
#[allow(clippy::too_many_arguments)]
fn make_task(
    session: Arc<CoreHttpSession>,
    method: &str,
    url: String,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    body: BodyArg,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    let method = Method::from_str(method).map_err(to_napi)?;
    let http_version = http_version
        .map(|v| HttpVersion::from_str(&v).map_err(to_napi))
        .transpose()?;
    Ok(AsyncTask::new(RequestTask {
        session,
        method,
        url,
        headers: headers.map(|m| m.into_iter().collect()).unwrap_or_default(),
        params: params.map(|m| m.into_iter().collect()).unwrap_or_default(),
        body,
        basic_auth: basic_auth_pair(basic_auth)?,
        bearer_auth,
        raise_error: raise_error.unwrap_or(true),
        keep_alive: keep_alive.unwrap_or(300.0),
        allow_redirect: allow_redirect.unwrap_or(true),
        http_version,
        send: send.unwrap_or(true),
    }))
}

/// A request body: raw bytes, or a `LocalPath` streamed straight off disk.
/// `Clone`, so a copy is kept for the response's `request`.
#[derive(Clone)]
enum BodyArg {
    Empty,
    Bytes(Vec<u8>),
    File(String),
}

/// Extracts a body argument (a `Buffer`, or a `LocalPath` to stream from disk).
fn body_arg(body: Option<Either<Buffer, &LocalPath>>) -> BodyArg {
    match body {
        None => BodyArg::Empty,
        Some(Either::A(buffer)) => BodyArg::Bytes(buffer.to_vec()),
        Some(Either::B(path)) => BodyArg::File(path.inner.location().to_string()),
    }
}

/// Applies a [`BodyArg`] to a request — a `File` streams from disk via `Io`.
fn apply_body(request: CoreHttpRequest, body: BodyArg) -> CoreHttpRequest {
    match body {
        BodyArg::Empty => request,
        BodyArg::Bytes(bytes) => request.with_body(bytes),
        BodyArg::File(location) => request.with_body_io(CoreLocalPath::open(location)),
    }
}

/// Extracts an optional `[username, password]` pair, erroring on a wrong shape.
fn basic_auth_pair(basic_auth: Option<Vec<String>>) -> Result<Option<(String, String)>> {
    match basic_auth {
        None => Ok(None),
        Some(pair) => match pair.as_slice() {
            [username, password] => Ok(Some((username.clone(), password.clone()))),
            _ => Err(Error::from_reason(
                "basicAuth expects a [username, password] pair",
            )),
        },
    }
}

/// Builds a core request from the full set of verb arguments — the single place
/// the bindings assemble a request from signature args. `session` resolves the
/// target against its `baseUrl` when given; otherwise the URL must be absolute.
#[allow(clippy::too_many_arguments)]
fn build_core_request(
    session: Option<&CoreHttpSession>,
    method: Method,
    url: &str,
    headers: Vec<(String, String)>,
    params: Vec<(String, String)>,
    body: BodyArg,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<HttpVersion>,
) -> Result<CoreHttpRequest> {
    let url = match session {
        Some(session) => session.resolve_url(url).map_err(to_napi)?,
        None => CoreUrl::from_str(url).map_err(|e| Error::from_reason(e.to_string()))?,
    };
    let mut request = CoreHttpRequest::from_url(method, url)
        .with_allow_redirect(allow_redirect)
        .with_keep_alive(keep_alive)
        .with_headers(headers);
    if let Some(http_version) = http_version {
        request = request.with_http_version(http_version);
    }
    for (key, value) in params {
        request = request.with_param(key, value);
    }
    if let Some((username, password)) = basic_auth {
        request = request.with_basic_auth(&username, &password);
    }
    if let Some(token) = bearer_auth {
        request = request.with_bearer_auth(&token);
    }
    Ok(apply_body(request, body))
}

/// The built-request snapshot shared by `HttpRequest` (the JS class) and a
/// response's `request` accessor: the prepared method / URL / headers / settings
/// plus the binding's own body copy. `Send`, so it crosses the libuv worker.
#[derive(Clone)]
struct RequestData {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: BodyArg,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<String>,
}

impl RequestData {
    /// Snapshots a core request (after it was prepared), keeping the binding's own
    /// `body` (the core copy may have dropped a stream).
    fn from_core(request: &CoreHttpRequest, body: BodyArg) -> RequestData {
        RequestData {
            method: request.method().as_str().to_string(),
            url: request.url().to_string(),
            headers: request
                .headers()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            body,
            allow_redirect: request.allow_redirect(),
            keep_alive: request.keep_alive(),
            http_version: request.http_version().map(|v| v.as_str().to_string()),
        }
    }

    /// Builds a [`RequestTask`] that dispatches this request through `session`.
    fn task(
        &self,
        session: Arc<CoreHttpSession>,
        raise_error: bool,
    ) -> Result<AsyncTask<RequestTask>> {
        let method = Method::from_str(&self.method).map_err(to_napi)?;
        let http_version = self
            .http_version
            .as_deref()
            .map(|v| HttpVersion::from_str(v).map_err(to_napi))
            .transpose()?;
        Ok(AsyncTask::new(RequestTask {
            session,
            method,
            url: self.url.clone(),
            headers: self.headers.clone(),
            params: Vec::new(),
            body: self.body.clone(),
            basic_auth: None,
            bearer_auth: None,
            raise_error,
            keep_alive: self.keep_alive,
            allow_redirect: self.allow_redirect,
            http_version,
            send: true,
        }))
    }
}

/// A built HTTP request, modelled on `requests.PreparedRequest`. It is what a verb
/// returns when `send=false` (via `HttpResponse.request`), and can be dispatched on
/// its own with `send`.
#[napi]
pub struct HttpRequest {
    data: RequestData,
}

#[napi]
impl HttpRequest {
    /// Build a request explicitly: `method` and `url` plus the same optional
    /// `headers` / `body` / `params` / `basicAuth` / `bearerAuth` / `allowRedirect`
    /// / `keepAlive` / `httpVersion` the verbs accept. `body` is a `Buffer` or a
    /// `LocalPath` (streamed off disk).
    #[napi(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        method: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<Either<Buffer, &LocalPath>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
    ) -> Result<Self> {
        let method = Method::from_str(&method).map_err(to_napi)?;
        let http_version = http_version
            .map(|v| HttpVersion::from_str(&v).map_err(to_napi))
            .transpose()?;
        let body = body_arg(body);
        let request = build_core_request(
            None,
            method,
            &url,
            headers.map(|m| m.into_iter().collect()).unwrap_or_default(),
            params.map(|m| m.into_iter().collect()).unwrap_or_default(),
            body.clone(),
            basic_auth_pair(basic_auth)?,
            bearer_auth,
            allow_redirect.unwrap_or(true),
            keep_alive.unwrap_or(300.0),
            http_version,
        )?;
        Ok(HttpRequest {
            data: RequestData::from_core(&request, body),
        })
    }

    /// The request method (e.g. `"GET"`).
    #[napi(getter)]
    pub fn method(&self) -> String {
        self.data.method.clone()
    }

    /// The request URL.
    #[napi(getter)]
    pub fn url(&self) -> String {
        self.data.url.clone()
    }

    /// The request headers as an object (lower-cased names).
    #[napi(getter)]
    pub fn headers(&self) -> HashMap<String, String> {
        self.data.headers.iter().cloned().collect()
    }

    /// Look up a header by name (case-insensitive).
    #[napi]
    pub fn header(&self, name: String) -> Option<String> {
        self.data
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&name))
            .map(|(_, value)| value.clone())
    }

    /// Whether `HttpSession.send` follows 3xx redirects for this request.
    #[napi(getter, js_name = "allowRedirect")]
    pub fn allow_redirect(&self) -> bool {
        self.data.allow_redirect
    }

    /// The keep-alive idle TTL in seconds (`0` disables pooling).
    #[napi(getter, js_name = "keepAlive")]
    pub fn keep_alive(&self) -> f64 {
        self.data.keep_alive
    }

    /// The pinned HTTP protocol version (e.g. `"HTTP/2"`), or `null` to inherit the
    /// session default.
    #[napi(getter, js_name = "httpVersion")]
    pub fn http_version(&self) -> Option<String> {
        self.data.http_version.clone()
    }

    /// Dispatch this request through the process-wide shared session, returning a
    /// `Promise<HttpResponse>`. `raiseError` (default `true`) rejects on a 4xx/5xx
    /// status.
    #[napi(js_name = "send")]
    pub fn send_request(&self, raise_error: Option<bool>) -> Result<AsyncTask<RequestTask>> {
        self.data
            .task(shared_session(), raise_error.unwrap_or(true))
    }

    /// An independent copy of this request.
    #[napi]
    pub fn copy(&self) -> HttpRequest {
        HttpRequest {
            data: self.data.clone(),
        }
    }
}

/// The data drained from a response on the worker thread, before it is handed
/// back to JS as an [`HttpResponse`].
pub struct ResponseData {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    sent_at: f64,
    received_at: f64,
    http_version: String,
    /// The request that produced this response (the prepared request).
    request: Option<RequestData>,
}

/// The blocking request, run on the libuv thread pool by napi.
pub struct RequestTask {
    session: Arc<CoreHttpSession>,
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    params: Vec<(String, String)>,
    body: BodyArg,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    raise_error: bool,
    keep_alive: f64,
    allow_redirect: bool,
    http_version: Option<HttpVersion>,
    /// With `send`, dispatch the request; otherwise prepare it and return an
    /// **unsent** response carrying only the request.
    send: bool,
}

impl Task for RequestTask {
    type Output = ResponseData;
    type JsValue = HttpResponse;

    fn compute(&mut self) -> Result<ResponseData> {
        // Build the request from the verb args (the single point all verbs funnel
        // through), keeping a copy of the body for the response's `request`.
        let body = std::mem::replace(&mut self.body, BodyArg::Empty);
        let request = build_core_request(
            Some(&self.session),
            self.method,
            &self.url,
            std::mem::take(&mut self.headers),
            std::mem::take(&mut self.params),
            body.clone(),
            self.basic_auth.take(),
            self.bearer_auth.take(),
            self.allow_redirect,
            self.keep_alive,
            self.http_version,
        )?;
        if self.send {
            // Every send streams; we drain the body here (off the event loop) into
            // an owned buffer, decompressed, so `content` / `text` / `json` are cheap.
            let response = self
                .session
                .send(request, self.raise_error)
                .map_err(to_napi)?;
            let status = response.status();
            let url = response.url().to_string();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            let sent_at = response.sent_at();
            let http_version = response.negotiated_version().as_str().to_string();
            let request = response
                .request()
                .map(|request| RequestData::from_core(request, body));
            let (body, received_at) = response.read_all().map_err(to_napi)?;
            Ok(ResponseData {
                status,
                url,
                headers,
                body,
                sent_at,
                received_at,
                http_version,
                request,
            })
        } else {
            // No network call: prepare the request and return it unsent.
            let prepared = self.session.prepare(request);
            let url = prepared.url().to_string();
            // Match the core's `HttpResponse::unsent`: an undispatched request
            // reports its own pinned version, else `Auto` (not the session default,
            // which negotiation has not yet resolved).
            let http_version = prepared
                .http_version()
                .unwrap_or(HttpVersion::Auto)
                .as_str()
                .to_string();
            let request = Some(RequestData::from_core(&prepared, body));
            Ok(ResponseData {
                status: 0,
                url,
                headers: Vec::new(),
                body: Vec::new(),
                sent_at: 0.0,
                received_at: 0.0,
                http_version,
                request,
            })
        }
    }

    fn resolve(&mut self, _env: Env, output: ResponseData) -> Result<HttpResponse> {
        Ok(HttpResponse {
            status: output.status,
            url: output.url,
            headers: output.headers,
            body: output.body,
            sent_at: output.sent_at,
            received_at: output.received_at,
            http_version: output.http_version,
            request: output.request,
        })
    }
}

/// A received HTTP response, modelled on `requests.Response`. The body is read
/// eagerly (and decompressed) when the response resolves.
#[napi]
pub struct HttpResponse {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    sent_at: f64,
    received_at: f64,
    http_version: String,
    /// The request that produced this response (`requests.Response.request`). An
    /// *unsent* response (a verb called with `send=false`) carries only this.
    request: Option<RequestData>,
}

#[napi]
impl HttpResponse {
    /// The HTTP status code (`0` for an unsent response).
    #[napi(getter)]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Whether the response is a success: it was actually dispatched (`isSent`)
    /// **and** its status is below 400 (the `requests` definition of "ok"). An
    /// **unsent** placeholder (status `0`) is *not* ok.
    #[napi(getter)]
    pub fn ok(&self) -> bool {
        self.status != 0 && self.status < 400
    }

    /// Whether this response was actually dispatched. `false` for the **unsent**
    /// placeholder a verb returns with `send=false` (status `0`, empty body), which
    /// carries only the prepared `request`.
    #[napi(getter, js_name = "isSent")]
    pub fn is_sent(&self) -> bool {
        self.status != 0
    }

    /// The originating prepared request that produced this response (similar to
    /// `requests.Response.request`), or `null`. After a redirect it is the *original*
    /// request, not the final hop, so its method/URL may differ from `url`.
    #[napi(getter)]
    pub fn request(&self) -> Option<HttpRequest> {
        self.request.clone().map(|data| HttpRequest { data })
    }

    /// Dispatch this response's `request` through the shared session, returning a
    /// `Promise<HttpResponse>` — how an **unsent** response (a verb called with
    /// `send=false`) is sent later. `raiseError` (default `true`) rejects on a
    /// 4xx/5xx status.
    #[napi(js_name = "send")]
    pub fn send_response(&self, raise_error: Option<bool>) -> Result<AsyncTask<RequestTask>> {
        match &self.request {
            Some(data) => data.task(shared_session(), raise_error.unwrap_or(true)),
            None => Err(Error::from_reason(
                "this response carries no request to send",
            )),
        }
    }

    /// The final request URL.
    #[napi(getter)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// The HTTP protocol version the response was delivered over (e.g.
    /// `"HTTP/1.1"`).
    #[napi(getter, js_name = "httpVersion")]
    pub fn http_version(&self) -> String {
        self.http_version.clone()
    }

    /// UTC Unix-epoch seconds when the request was dispatched (`0.0` if unset).
    #[napi(getter, js_name = "sentAt")]
    pub fn sent_at(&self) -> f64 {
        self.sent_at
    }

    /// UTC Unix-epoch seconds when the connection finished delivering the body
    /// (`0.0` if unset).
    #[napi(getter, js_name = "receivedAt")]
    pub fn received_at(&self) -> f64 {
        self.received_at
    }

    /// The response headers as an object (lower-cased names).
    #[napi(getter)]
    pub fn headers(&self) -> HashMap<String, String> {
        self.headers.iter().cloned().collect()
    }

    /// Look up a header by name (case-insensitive).
    #[napi]
    pub fn header(&self, name: String) -> Option<String> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&name))
            .map(|(_, value)| value.clone())
    }

    /// The `Content-Type` header, if present.
    #[napi(getter, js_name = "contentType")]
    pub fn content_type(&self) -> Option<String> {
        self.header("content-type".to_string())
    }

    /// The `Content-Encoding` header, if present.
    #[napi(getter, js_name = "contentEncoding")]
    pub fn content_encoding(&self) -> Option<String> {
        self.header("content-encoding".to_string())
    }

    /// The single MIME type inferred from `Content-Type` (e.g. `"text/csv"`).
    #[napi(getter, js_name = "mimeType")]
    pub fn mime_type(&self) -> Option<String> {
        self.content_type()
            .and_then(|ct| CoreMimeType::from_str(&ct).ok())
            .map(|mime| mime.to_string())
    }

    /// The layered media type **combining `Content-Type` with `Content-Encoding`** as
    /// an array of MIME strings: the content type is inner, the transfer encoding
    /// outer — e.g. a gzipped CSV reads as `["text/csv", "application/gzip"]`.
    #[napi(getter, js_name = "mediaType")]
    pub fn media_type(&self) -> Option<Vec<String>> {
        let mut types: Vec<String> = self
            .content_type()
            .and_then(|ct| CoreMimeType::from_str(&ct).ok())
            .map(|mime| mime.to_string())
            .into_iter()
            .collect();
        if let Some(mime) = self
            .content_encoding()
            .and_then(|enc| CoreCompression::from_str(&enc).ok())
            .and_then(|codec| codec.mime())
        {
            types.push(mime.to_string());
        }
        (!types.is_empty()).then_some(types)
    }

    /// The compression codec named by `Content-Encoding` (`"gzip"` / `"zstd"` /
    /// `"snappy"` / `"brotli"`), or `null`. The body is already decoded — `content`
    /// / `text` / `json` are the decompressed payload.
    #[napi(getter)]
    pub fn compression(&self) -> Option<String> {
        self.content_encoding()
            .and_then(|enc| CoreCompression::from_str(&enc).ok())
            .filter(|codec| *codec != CoreCompression::None)
            .map(|codec| codec.as_str().to_string())
    }

    /// The decompressed body as a yggdryl `BytesIO` handle — the **performant**
    /// accessor: it stays a Rust-backed, seekable byte buffer, so you can `json()` /
    /// `decompress()` / `read` it (or pass it to another yggdryl call) without copying
    /// the bytes into JS. Use `content` for a native `Buffer` when an API needs one.
    #[napi(getter)]
    pub fn io(&self) -> BytesIO {
        BytesIO {
            inner: CoreBytesIO::from_bytes(self.body.clone()),
        }
    }

    /// The raw response body as a native `Buffer` (already decompressed; a copy out
    /// of Rust — prefer `io` for further Rust-side work).
    #[napi(getter)]
    pub fn content(&self) -> Buffer {
        Buffer::from(self.body.clone())
    }

    /// The response body decoded as UTF-8 text (already decompressed).
    #[napi]
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The response body parsed as JSON (already decompressed).
    #[napi]
    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.body).map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Throw if the status is 4xx/5xx, otherwise do nothing — the `requests`
    /// `raiseForStatus` pattern.
    #[napi(js_name = "raiseForStatus")]
    pub fn raise_for_status(&self) -> Result<()> {
        if self.status >= 400 {
            return Err(Error::from_reason(format!("http status {}", self.status)));
        }
        Ok(())
    }
}

/// A connection-pooling HTTP client, like `requests.Session`. Its request methods
/// return a `Promise<HttpResponse>`.
#[napi]
pub struct HttpSession {
    inner: Arc<CoreHttpSession>,
}

#[napi]
impl HttpSession {
    /// Create a session, optionally with a default `userAgent`, default `headers`
    /// sent with every request, a `maxRedirects` cap on 3xx hops followed, a
    /// `baseUrl` that relative request targets resolve against, and a default
    /// `httpVersion` (`"auto"` / `"1.1"` / `"2"` / `"3"`) for requests that do not
    /// pin one.
    ///
    /// `basicAuth` (a `[username, password]` pair) or `bearerAuth` (a token) set a
    /// default `Authorization` header on every request (HTTP Basic / Bearer); it is
    /// stripped on a cross-origin redirect. `readTimeout` (seconds, default 120)
    /// errors if the server sends no data for that long; `0` removes the bound.
    #[napi(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        user_agent: Option<String>,
        headers: Option<HashMap<String, String>>,
        max_redirects: Option<u32>,
        base_url: Option<String>,
        http_version: Option<String>,
        verify: Option<bool>,
        proxy: Option<String>,
        ca_cert: Option<Buffer>,
        ca_cert_file: Option<String>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        read_timeout: Option<f64>,
    ) -> Result<Self> {
        let mut inner = CoreHttpSession::new();
        if let Some(user_agent) = user_agent {
            inner = inner.with_user_agent(user_agent);
        }
        if let Some(headers) = headers {
            for (key, value) in headers {
                inner = inner.with_header(key, value);
            }
        }
        if let Some(max_redirects) = max_redirects {
            inner = inner.with_max_redirects(max_redirects as usize);
        }
        if let Some(base_url) = base_url {
            let base =
                CoreUrl::from_str(&base_url).map_err(|e| Error::from_reason(e.to_string()))?;
            inner = inner.with_base_url(base);
        }
        if let Some(http_version) = http_version {
            inner = inner.with_http_version(HttpVersion::from_str(&http_version).map_err(to_napi)?);
        }
        if verify == Some(false) {
            inner = inner.with_verify(false);
        }
        if let Some(proxy) = proxy {
            inner = inner.with_proxy(&proxy).map_err(to_napi)?;
        }
        if let Some(ca_cert) = ca_cert {
            inner = inner.with_ca_cert(&ca_cert).map_err(to_napi)?;
        }
        if let Some(ca_cert_file) = ca_cert_file {
            inner = inner.with_ca_cert_file(&ca_cert_file).map_err(to_napi)?;
        }
        if let Some(basic_auth) = basic_auth {
            let [username, password] = basic_auth.as_slice() else {
                return Err(Error::from_reason(
                    "basicAuth expects a [username, password] pair",
                ));
            };
            inner = inner.with_basic_auth(username, password);
        }
        if let Some(bearer_auth) = bearer_auth {
            inner = inner.with_bearer_auth(&bearer_auth);
        }
        if let Some(read_timeout) = read_timeout {
            inner = inner.with_read_timeout(read_timeout);
        }
        Ok(HttpSession {
            inner: Arc::new(inner),
        })
    }

    /// The read timeout in seconds (a request errors if the server sends no data
    /// for this long; `0` means unbounded).
    #[napi(getter, js_name = "readTimeout")]
    pub fn read_timeout(&self) -> f64 {
        self.inner.read_timeout()
    }

    /// An independent copy of this session — same configuration and a snapshot of
    /// the cookie jar, but its own fresh connection pool.
    #[napi]
    pub fn copy(&self) -> HttpSession {
        HttpSession {
            inner: Arc::new(self.inner.copy()),
        }
    }

    /// The maximum number of 3xx redirect hops followed per request.
    #[napi(getter, js_name = "maxRedirects")]
    pub fn max_redirects(&self) -> u32 {
        self.inner.max_redirects() as u32
    }

    /// The session's base URL (relative request targets resolve against it), or
    /// `null`.
    #[napi(getter, js_name = "baseUrl")]
    pub fn base_url(&self) -> Option<String> {
        self.inner.base_url().map(ToString::to_string)
    }

    /// The session's default HTTP protocol version (e.g. `"auto"`, `"HTTP/1.1"`)
    /// applied to requests that do not pin their own.
    #[napi(getter, js_name = "httpVersion")]
    pub fn http_version(&self) -> String {
        self.inner.http_version().as_str().to_string()
    }

    /// Whether TLS certificate verification is performed (`false` accepts any
    /// certificate — insecure, for self-signed / internal hosts).
    #[napi(getter)]
    pub fn verify(&self) -> bool {
        self.inner.verify()
    }

    /// The proxy URL all requests route through, or `null` (defaults to the
    /// environment's `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`).
    #[napi(getter)]
    pub fn proxy(&self) -> Option<String> {
        self.inner.proxy()
    }

    /// The number of installed CA certificates (`0` means the default trust store is
    /// used). Install certificates with the `caCert` / `caCertFile` constructor
    /// arguments.
    #[napi(getter, js_name = "caCertCount")]
    pub fn ca_cert_count(&self) -> u32 {
        self.inner.ca_cert_count() as u32
    }

    /// The session's cookies as an object of `name` to `value` (the jar snapshot —
    /// last value wins for a repeated name).
    #[napi(getter)]
    pub fn cookies(&self) -> HashMap<String, String> {
        self.inner
            .cookies()
            .iter()
            .map(|cookie| (cookie.name().to_string(), cookie.value().to_string()))
            .collect()
    }

    /// Seed a cookie into the session jar, scoped to `url`'s host (host-only) and
    /// path `"/"`, so it is sent on matching requests.
    #[napi(js_name = "setCookie")]
    pub fn set_cookie(&self, url: String, name: String, value: String) -> Result<()> {
        let url =
            yggdryl_core::Url::from_str(&url).map_err(|e| Error::from_reason(e.to_string()))?;
        self.inner.set_cookie(&url, name, value);
        Ok(())
    }

    /// `GET url` (resolved against the session's `baseUrl` when set), configured
    /// from the optional `headers` / `params` / `basicAuth` / `bearerAuth` /
    /// `allowRedirect` / `keepAlive` / `httpVersion`. `raiseError` (default `true`)
    /// rejects on a 4xx/5xx status. With `send=false` no request is dispatched: the
    /// returned `Promise` resolves to an **unsent** `HttpResponse` carrying the
    /// prepared `request` (send it later with `response.send()`).
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            "GET",
            url,
            headers,
            params,
            BodyArg::Empty,
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// `HEAD url` — same options as `get`.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn head(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            "HEAD",
            url,
            headers,
            params,
            BodyArg::Empty,
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// `DELETE url` — same options as `get`.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn delete(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            "DELETE",
            url,
            headers,
            params,
            BodyArg::Empty,
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// `POST url` with an optional `body` — a `Buffer` or a `LocalPath` streamed
    /// straight off disk — and the same options as `get`.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn post(
        &self,
        url: String,
        body: Option<Either<Buffer, &LocalPath>>,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            "POST",
            url,
            headers,
            params,
            body_arg(body),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// `PUT url` with a `body` — same options as `post`.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &self,
        url: String,
        body: Option<Either<Buffer, &LocalPath>>,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            "PUT",
            url,
            headers,
            params,
            body_arg(body),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// `PATCH url` with a `body` — same options as `post`.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn patch(
        &self,
        url: String,
        body: Option<Either<Buffer, &LocalPath>>,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            "PATCH",
            url,
            headers,
            params,
            body_arg(body),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// Issue an arbitrary `method` request, with optional `headers` and `body`
    /// (a `Buffer` or a `LocalPath`) plus the same `params` / `basicAuth` /
    /// `bearerAuth` / `allowRedirect` / `keepAlive` / `httpVersion` / `raiseError` /
    /// `send` options as the other verbs (in that order). `raiseError` (default
    /// `true`) throws on a 4xx/5xx status; `keepAlive` is the keep-alive idle TTL in
    /// seconds (default 300; `0` sends `Connection: close`); `allowRedirect` (default
    /// `true`) follows 3xx redirects; `httpVersion` (e.g. `"2"`) pins the protocol
    /// version. With `send=false` the prepared request is returned as an **unsent**
    /// response (see `get`).
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &self,
        method: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<Either<Buffer, &LocalPath>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<Vec<String>>,
        bearer_auth: Option<String>,
        allow_redirect: Option<bool>,
        keep_alive: Option<f64>,
        http_version: Option<String>,
        raise_error: Option<bool>,
        send: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        make_task(
            self.inner.clone(),
            &method,
            url,
            headers,
            params,
            body_arg(body),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
            raise_error,
            send,
        )
    }

    /// Dispatch a prebuilt `HttpRequest` through this session — the centralised
    /// `send(request) -> HttpResponse` entry point (the dispatch primitive, always
    /// sent; build without sending via a verb's `send=false`). `raiseError` (default
    /// `true`) rejects on a 4xx/5xx status. The request's headers are carried
    /// verbatim (order and duplicates preserved).
    #[napi]
    pub fn send(
        &self,
        request: &HttpRequest,
        raise_error: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        request
            .data
            .task(self.inner.clone(), raise_error.unwrap_or(true))
    }
}

/// `GET url` via the process-wide shared `HttpSession` singleton (the
/// `requests.get` equivalent — rejects on a 4xx/5xx status). Takes the same options
/// as `HttpSession.get`, including `send=false` to return an **unsent** response.
#[napi(js_name = "get")]
#[allow(clippy::too_many_arguments)]
pub fn http_get(
    url: String,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    make_task(
        shared_session(),
        "GET",
        url,
        headers,
        params,
        BodyArg::Empty,
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
        raise_error,
        send,
    )
}

/// `HEAD url` via the shared session singleton — same options as `get`.
#[napi(js_name = "head")]
#[allow(clippy::too_many_arguments)]
pub fn http_head(
    url: String,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    make_task(
        shared_session(),
        "HEAD",
        url,
        headers,
        params,
        BodyArg::Empty,
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
        raise_error,
        send,
    )
}

// NOTE: there is intentionally no module-level `delete` verb — `delete` is a JS
// reserved word the napi-generated `index.js` cannot bind at module scope. Use
// `request('DELETE', url)` (or the `HttpSession.delete` method) instead.

/// `POST url` with an optional `body` (a `Buffer` or `LocalPath`) via the shared
/// session singleton — same options as `HttpSession.post`.
#[napi(js_name = "post")]
#[allow(clippy::too_many_arguments)]
pub fn http_post(
    url: String,
    body: Option<Either<Buffer, &LocalPath>>,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    make_task(
        shared_session(),
        "POST",
        url,
        headers,
        params,
        body_arg(body),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
        raise_error,
        send,
    )
}

/// `PUT url` with a `body` via the shared session singleton.
#[napi(js_name = "put")]
#[allow(clippy::too_many_arguments)]
pub fn http_put(
    url: String,
    body: Option<Either<Buffer, &LocalPath>>,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    make_task(
        shared_session(),
        "PUT",
        url,
        headers,
        params,
        body_arg(body),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
        raise_error,
        send,
    )
}

/// `PATCH url` with a `body` via the shared session singleton.
#[napi(js_name = "patch")]
#[allow(clippy::too_many_arguments)]
pub fn http_patch(
    url: String,
    body: Option<Either<Buffer, &LocalPath>>,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    make_task(
        shared_session(),
        "PATCH",
        url,
        headers,
        params,
        body_arg(body),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
        raise_error,
        send,
    )
}

/// Issue an arbitrary `method` request via the shared session singleton (same
/// arguments as `HttpSession.request`, including `send=false`).
#[napi(js_name = "request")]
#[allow(clippy::too_many_arguments)]
pub fn http_request(
    method: String,
    url: String,
    headers: Option<HashMap<String, String>>,
    body: Option<Either<Buffer, &LocalPath>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<Vec<String>>,
    bearer_auth: Option<String>,
    allow_redirect: Option<bool>,
    keep_alive: Option<f64>,
    http_version: Option<String>,
    raise_error: Option<bool>,
    send: Option<bool>,
) -> Result<AsyncTask<RequestTask>> {
    make_task(
        shared_session(),
        &method,
        url,
        headers,
        params,
        body_arg(body),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
        raise_error,
        send,
    )
}

/// Configure the process-wide shared `HttpSession` singleton with a `baseUrl`
/// (replacing it), so the module-level verbs resolve relative targets — e.g.
/// `setBaseUrl("https://api.example.com")` then `get("/users")`.
#[napi(js_name = "setBaseUrl")]
pub fn set_base_url(base_url: String) -> Result<()> {
    let base = CoreUrl::from_str(&base_url).map_err(|e| Error::from_reason(e.to_string()))?;
    CoreHttpSession::set_shared(CoreHttpSession::new().with_base_url(base));
    Ok(())
}
