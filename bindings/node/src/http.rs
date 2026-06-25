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
use yggdryl_core::{LocalPath as CoreLocalPath, Path, Url as CoreUrl};
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

/// Builds a [`RequestTask`] over the shared session singleton.
#[allow(clippy::too_many_arguments)]
fn shared_task(
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: BodyArg,
    raise_error: bool,
    keep_alive: bool,
    allow_redirect: bool,
    http_version: Option<HttpVersion>,
) -> AsyncTask<RequestTask> {
    AsyncTask::new(RequestTask {
        session: shared_session(),
        method,
        url,
        headers,
        body,
        raise_error,
        keep_alive,
        allow_redirect,
        http_version,
    })
}

/// A request body: raw bytes, or a `LocalPath` streamed straight off disk.
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
}

/// The blocking request, run on the libuv thread pool by napi.
pub struct RequestTask {
    session: Arc<CoreHttpSession>,
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: BodyArg,
    raise_error: bool,
    keep_alive: bool,
    allow_redirect: bool,
    http_version: Option<HttpVersion>,
}

impl Task for RequestTask {
    type Output = ResponseData;
    type JsValue = HttpResponse;

    fn compute(&mut self) -> Result<ResponseData> {
        // Resolve the target against the session's base URL (the single point all
        // verbs — instance and module-level — funnel through), then build it.
        let url = self.session.resolve_url(&self.url).map_err(to_napi)?;
        let mut request =
            CoreHttpRequest::from_url(self.method, url).with_allow_redirect(self.allow_redirect);
        if let Some(http_version) = self.http_version {
            request = request.with_http_version(http_version);
        }
        request = request.with_headers(std::mem::take(&mut self.headers));
        request = match std::mem::replace(&mut self.body, BodyArg::Empty) {
            BodyArg::Empty => request,
            BodyArg::Bytes(bytes) => request.with_body(bytes),
            BodyArg::File(location) => request.with_body_io(CoreLocalPath::open(location)),
        };
        // A buffered (`stream = false`) send drains the body now and releases the
        // connection, so `received_at` is already stamped before `bytes()`.
        let response = self
            .session
            .send(request, self.raise_error, self.keep_alive, false)
            .map_err(to_napi)?;
        let status = response.status();
        let url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        let sent_at = response.sent_at();
        let received_at = response.received_at();
        let http_version = response.negotiated_version().as_str().to_string();
        let body = response.bytes().map_err(to_napi)?;
        Ok(ResponseData {
            status,
            url,
            headers,
            body,
            sent_at,
            received_at,
            http_version,
        })
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
}

#[napi]
impl HttpResponse {
    /// The HTTP status code.
    #[napi(getter)]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Whether the status is below 400 (the `requests` definition of "ok").
    #[napi(getter)]
    pub fn ok(&self) -> bool {
        self.status < 400
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

    /// The raw response body.
    #[napi(getter)]
    pub fn content(&self) -> Buffer {
        Buffer::from(self.body.clone())
    }

    /// The response body decoded as UTF-8 text.
    #[napi]
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).map_err(|e| Error::from_reason(e.to_string()))
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
        Ok(HttpSession {
            inner: Arc::new(inner),
        })
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

    #[allow(clippy::too_many_arguments)]
    fn task(
        &self,
        method: Method,
        url: String,
        headers: Vec<(String, String)>,
        body: BodyArg,
        raise_error: bool,
        keep_alive: bool,
        allow_redirect: bool,
        http_version: Option<HttpVersion>,
    ) -> AsyncTask<RequestTask> {
        AsyncTask::new(RequestTask {
            session: self.inner.clone(),
            method,
            url,
            headers,
            body,
            raise_error,
            keep_alive,
            allow_redirect,
            http_version,
        })
    }

    /// `GET url` (raises on a 4xx/5xx status).
    #[napi]
    pub fn get(&self, url: String) -> AsyncTask<RequestTask> {
        self.task(
            Method::Get,
            url,
            Vec::new(),
            BodyArg::Empty,
            true,
            true,
            true,
            None,
        )
    }

    /// `HEAD url` (raises on a 4xx/5xx status).
    #[napi]
    pub fn head(&self, url: String) -> AsyncTask<RequestTask> {
        self.task(
            Method::Head,
            url,
            Vec::new(),
            BodyArg::Empty,
            true,
            true,
            true,
            None,
        )
    }

    /// `DELETE url` (raises on a 4xx/5xx status).
    #[napi]
    pub fn delete(&self, url: String) -> AsyncTask<RequestTask> {
        self.task(
            Method::Delete,
            url,
            Vec::new(),
            BodyArg::Empty,
            true,
            true,
            true,
            None,
        )
    }

    /// `POST url` with an optional `body` — a `Buffer` or a `LocalPath` streamed
    /// straight off disk (raises on a 4xx/5xx status).
    #[napi]
    pub fn post(
        &self,
        url: String,
        body: Option<Either<Buffer, &LocalPath>>,
    ) -> AsyncTask<RequestTask> {
        self.task(
            Method::Post,
            url,
            Vec::new(),
            body_arg(body),
            true,
            true,
            true,
            None,
        )
    }

    /// `PUT url` with a `body` — a `Buffer` or a `LocalPath` (raises on 4xx/5xx).
    #[napi]
    pub fn put(
        &self,
        url: String,
        body: Option<Either<Buffer, &LocalPath>>,
    ) -> AsyncTask<RequestTask> {
        self.task(
            Method::Put,
            url,
            Vec::new(),
            body_arg(body),
            true,
            true,
            true,
            None,
        )
    }

    /// `PATCH url` with a `body` — a `Buffer` or a `LocalPath` (raises on 4xx/5xx).
    #[napi]
    pub fn patch(
        &self,
        url: String,
        body: Option<Either<Buffer, &LocalPath>>,
    ) -> AsyncTask<RequestTask> {
        self.task(
            Method::Patch,
            url,
            Vec::new(),
            body_arg(body),
            true,
            true,
            true,
            None,
        )
    }

    /// Issue an arbitrary `method` request, with optional `headers` and `body`
    /// (a `Buffer` or a `LocalPath`). `raiseError` (default `true`) throws on a
    /// 4xx/5xx status. `keepAlive` (default `true`) pools the connection for reuse
    /// (skipping the next TLS handshake); pass `false` to close it after.
    /// `allowRedirect` (default `true`) follows 3xx redirects (up to the session's
    /// `maxRedirects`); pass `false` to receive the 3xx response itself.
    /// `httpVersion` (e.g. `"2"`) pins the protocol version for this request,
    /// overriding the session default.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &self,
        method: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<Either<Buffer, &LocalPath>>,
        raise_error: Option<bool>,
        keep_alive: Option<bool>,
        allow_redirect: Option<bool>,
        http_version: Option<String>,
    ) -> Result<AsyncTask<RequestTask>> {
        let method = Method::from_str(&method).map_err(to_napi)?;
        let http_version = http_version
            .map(|value| HttpVersion::from_str(&value).map_err(to_napi))
            .transpose()?;
        let headers = headers
            .map(|map| map.into_iter().collect())
            .unwrap_or_default();
        Ok(self.task(
            method,
            url,
            headers,
            body_arg(body),
            raise_error.unwrap_or(true),
            keep_alive.unwrap_or(true),
            allow_redirect.unwrap_or(true),
            http_version,
        ))
    }
}

/// `GET url` via the process-wide shared `HttpSession` singleton (the
/// `requests.get` equivalent — rejects on a 4xx/5xx status).
#[napi(js_name = "get")]
pub fn http_get(url: String) -> AsyncTask<RequestTask> {
    shared_task(
        Method::Get,
        url,
        Vec::new(),
        BodyArg::Empty,
        true,
        true,
        true,
        None,
    )
}

/// `HEAD url` via the shared session singleton (rejects on a 4xx/5xx status).
#[napi(js_name = "head")]
pub fn http_head(url: String) -> AsyncTask<RequestTask> {
    shared_task(
        Method::Head,
        url,
        Vec::new(),
        BodyArg::Empty,
        true,
        true,
        true,
        None,
    )
}

// NOTE: there is intentionally no module-level `delete` verb — `delete` is a JS
// reserved word the napi-generated `index.js` cannot bind at module scope. Use
// `request('DELETE', url)` (or the `HttpSession.delete` method) instead.

/// `POST url` with an optional `body` (a `Buffer` or `LocalPath`) via the shared
/// session singleton.
#[napi(js_name = "post")]
pub fn http_post(url: String, body: Option<Either<Buffer, &LocalPath>>) -> AsyncTask<RequestTask> {
    shared_task(
        Method::Post,
        url,
        Vec::new(),
        body_arg(body),
        true,
        true,
        true,
        None,
    )
}

/// `PUT url` with a `body` via the shared session singleton.
#[napi(js_name = "put")]
pub fn http_put(url: String, body: Option<Either<Buffer, &LocalPath>>) -> AsyncTask<RequestTask> {
    shared_task(
        Method::Put,
        url,
        Vec::new(),
        body_arg(body),
        true,
        true,
        true,
        None,
    )
}

/// `PATCH url` with a `body` via the shared session singleton.
#[napi(js_name = "patch")]
pub fn http_patch(url: String, body: Option<Either<Buffer, &LocalPath>>) -> AsyncTask<RequestTask> {
    shared_task(
        Method::Patch,
        url,
        Vec::new(),
        body_arg(body),
        true,
        true,
        true,
        None,
    )
}

/// Issue an arbitrary `method` request via the shared session singleton (same
/// arguments as `HttpSession.request`).
#[napi(js_name = "request")]
#[allow(clippy::too_many_arguments)]
pub fn http_request(
    method: String,
    url: String,
    headers: Option<HashMap<String, String>>,
    body: Option<Either<Buffer, &LocalPath>>,
    raise_error: Option<bool>,
    keep_alive: Option<bool>,
    allow_redirect: Option<bool>,
    http_version: Option<String>,
) -> Result<AsyncTask<RequestTask>> {
    let method = Method::from_str(&method).map_err(to_napi)?;
    let http_version = http_version
        .map(|value| HttpVersion::from_str(&value).map_err(to_napi))
        .transpose()?;
    let headers = headers
        .map(|map| map.into_iter().collect())
        .unwrap_or_default();
    Ok(shared_task(
        method,
        url,
        headers,
        body_arg(body),
        raise_error.unwrap_or(true),
        keep_alive.unwrap_or(true),
        allow_redirect.unwrap_or(true),
        http_version,
    ))
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
