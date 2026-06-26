//! The `HttpSession` and `HttpResponse` pyclasses (a `requests`-like client).

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyType};
use yggdryl_core::{BytesIO as CoreBytesIO, LocalPath as CoreLocalPath, Path};
use yggdryl_http::{
    HttpRequest as CoreHttpRequest, HttpSession as CoreHttpSession, HttpVersion, Method,
};

use crate::bytesio::BytesIO;
use crate::http_err;
use crate::localpath::LocalPath;

/// A request body extracted from a Python argument: raw `bytes`, or one of our
/// `Io` handles (a `LocalPath` streams straight off disk; a `BytesIO`'s bytes are
/// taken). `Send`, so it crosses into the GIL-released worker; `Clone`, so a copy
/// is kept for the response's :attr:`request`.
#[derive(Clone)]
pub(crate) enum BodyArg {
    Empty,
    Bytes(Vec<u8>),
    File(String),
}

/// Extracts a body argument: an `Io` handle is preferred (streamed), else bytes.
fn extract_body(body: Option<&Bound<'_, PyAny>>) -> PyResult<BodyArg> {
    let Some(object) = body else {
        return Ok(BodyArg::Empty);
    };
    if object.is_none() {
        return Ok(BodyArg::Empty);
    }
    if let Ok(path) = object.extract::<PyRef<LocalPath>>() {
        return Ok(BodyArg::File(path.inner.location().to_string()));
    }
    if let Ok(buffer) = object.extract::<PyRef<BytesIO>>() {
        return Ok(BodyArg::Bytes(buffer.inner.getvalue().to_vec()));
    }
    Ok(BodyArg::Bytes(object.extract::<Vec<u8>>()?))
}

/// Applies a [`BodyArg`] to a request — a `File` streams from disk via `Io`.
fn apply_body(request: CoreHttpRequest, body: BodyArg) -> CoreHttpRequest {
    match body {
        BodyArg::Empty => request,
        BodyArg::Bytes(bytes) => request.with_body(bytes),
        BodyArg::File(location) => request.with_body_io(CoreLocalPath::open(location)),
    }
}

/// Builds a core request from the full set of verb arguments — the single place
/// the bindings assemble a request from signature args (so every verb configures
/// the request the same way). `session` resolves the target against its
/// ``base_url`` when given; otherwise the URL must be absolute.
#[allow(clippy::too_many_arguments)]
fn build_core_request(
    session: Option<&CoreHttpSession>,
    method: Method,
    url: &str,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    body: BodyArg,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
) -> PyResult<CoreHttpRequest> {
    let url = match session {
        Some(session) => session.resolve_url(url).map_err(http_err)?,
        None => yggdryl_core::Url::from_str(url).map_err(crate::url_err)?,
    };
    let mut request = CoreHttpRequest::from_url(method, url)
        .with_allow_redirect(allow_redirect)
        .with_keep_alive(keep_alive);
    if let Some(http_version) = http_version {
        request = request.with_http_version(HttpVersion::from_str(http_version).map_err(http_err)?);
    }
    if let Some(headers) = headers {
        request = request.with_headers(headers);
    }
    if let Some(params) = params {
        for (key, value) in params {
            request = request.with_param(key, value);
        }
    }
    if let Some((username, password)) = basic_auth {
        request = request.with_basic_auth(&username, &password);
    }
    if let Some(token) = bearer_auth {
        request = request.with_bearer_auth(&token);
    }
    Ok(apply_body(request, body))
}

/// A built HTTP request, modelled on :class:`requests.PreparedRequest`. It is what
/// a verb returns when ``send=False`` (via :attr:`HttpResponse.request`), and can
/// be dispatched on its own with :meth:`send`.
#[pyclass(name = "HttpRequest", module = "yggdryl")]
#[derive(Clone)]
pub struct HttpRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    pub(crate) body: BodyArg,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<String>,
}

impl HttpRequest {
    /// Snapshots a core request (after it was prepared) into the binding type,
    /// keeping the binding's own `body` (the core copy may have dropped a stream).
    fn from_core(request: &CoreHttpRequest, body: BodyArg) -> HttpRequest {
        HttpRequest {
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

    /// Rebuilds the core request from this snapshot (its URL is already absolute),
    /// ready to dispatch.
    fn to_core(&self) -> PyResult<CoreHttpRequest> {
        let method = Method::from_str(&self.method).map_err(http_err)?;
        let url = yggdryl_core::Url::from_str(&self.url).map_err(crate::url_err)?;
        let mut request = CoreHttpRequest::from_url(method, url)
            .with_headers(self.headers.clone())
            .with_allow_redirect(self.allow_redirect)
            .with_keep_alive(self.keep_alive);
        if let Some(http_version) = &self.http_version {
            request =
                request.with_http_version(HttpVersion::from_str(http_version).map_err(http_err)?);
        }
        Ok(apply_body(request, self.body.clone()))
    }
}

#[pymethods]
impl HttpRequest {
    /// Build a request explicitly: ``method`` and ``url`` plus the same optional
    /// ``headers`` / ``params`` / ``basic_auth`` / ``bearer_auth`` /
    /// ``allow_redirect`` / ``keep_alive`` / ``http_version`` the verbs accept.
    /// ``body`` is ``bytes`` or one of our `Io` handles (a :class:`LocalPath`
    /// streams off disk).
    #[new]
    #[pyo3(signature = (method, url, headers = None, body = None, *, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        method: &str,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<Bound<'_, PyAny>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
    ) -> PyResult<HttpRequest> {
        let method = Method::from_str(method).map_err(http_err)?;
        let body = extract_body(body.as_ref())?;
        let request = build_core_request(
            None,
            method,
            url,
            headers,
            params,
            body.clone(),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        Ok(HttpRequest::from_core(&request, body))
    }

    /// The request method (e.g. ``"GET"``).
    #[getter]
    fn method(&self) -> &str {
        &self.method
    }

    /// The request URL.
    #[getter]
    fn url(&self) -> &str {
        &self.url
    }

    /// The request headers as a ``dict`` (lower-cased names).
    #[getter]
    fn headers(&self) -> HashMap<String, String> {
        self.headers.iter().cloned().collect()
    }

    /// Look up a header by name (case-insensitive).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Whether :meth:`HttpSession.send` follows 3xx redirects for this request.
    #[getter]
    fn allow_redirect(&self) -> bool {
        self.allow_redirect
    }

    /// The keep-alive idle TTL in seconds (``0.0`` disables pooling).
    #[getter]
    fn keep_alive(&self) -> f64 {
        self.keep_alive
    }

    /// The pinned HTTP protocol version (e.g. ``"HTTP/2"``), or ``None`` to inherit
    /// the session default.
    #[getter]
    fn http_version(&self) -> Option<&str> {
        self.http_version.as_deref()
    }

    /// Dispatch this request through the process-wide shared session and return the
    /// :class:`HttpResponse`. ``raise_error`` (default ``True``) raises on a
    /// 4xx/5xx status.
    #[pyo3(signature = (raise_error = true))]
    fn send(&self, py: Python<'_>, raise_error: bool) -> PyResult<HttpResponse> {
        let request = self.to_core()?;
        let shared = CoreHttpSession::shared();
        run_verb(py, &shared, request, self.body.clone(), raise_error, true)
    }

    /// An independent copy of this request.
    fn copy(&self) -> HttpRequest {
        self.clone()
    }

    fn __repr__(&self) -> String {
        format!("HttpRequest(method={:?}, url={:?})", self.method, self.url)
    }
}

/// Runs a verb: with ``send`` the `request` is dispatched through `session` (GIL
/// released) and the body buffered; without, it is prepared and returned as an
/// **unsent** :class:`HttpResponse` carrying the request. Always returns a binding
/// `HttpResponse` — the single place the bindings honour the `send` flag, mirroring
/// the core's ``prepare`` → request / ``send`` → response split.
fn run_verb(
    py: Python<'_>,
    session: &CoreHttpSession,
    request: CoreHttpRequest,
    body: BodyArg,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let (status, url, headers, resp_body, sent_at, received_at, http_version, request_spec) = py
        .allow_threads(move || -> Result<_, yggdryl_http::HttpError> {
            if send {
                let response = session.send(request, raise_error)?;
                let status = response.status();
                let url = response.url().to_string();
                let headers = response
                    .headers()
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect::<Vec<_>>();
                let sent_at = response.sent_at();
                let http_version = response.negotiated_version().as_str().to_string();
                let request_spec = response
                    .request()
                    .map(|request| HttpRequest::from_core(request, body));
                let (resp_body, received_at) = response.read_all()?;
                Ok((
                    status,
                    url,
                    headers,
                    resp_body,
                    sent_at,
                    received_at,
                    http_version,
                    request_spec,
                ))
            } else {
                let prepared = session.prepare(request);
                let url = prepared.url().to_string();
                let http_version = prepared
                    .http_version()
                    .unwrap_or_else(|| session.http_version())
                    .as_str()
                    .to_string();
                let request_spec = Some(HttpRequest::from_core(&prepared, body));
                Ok((
                    0,
                    url,
                    Vec::new(),
                    Vec::new(),
                    0.0,
                    0.0,
                    http_version,
                    request_spec,
                ))
            }
        })
        .map_err(http_err)?;
    Ok(HttpResponse {
        status,
        url,
        headers,
        body: resp_body,
        sent_at,
        received_at,
        http_version,
        request: request_spec,
    })
}

/// A received HTTP response, modelled on :class:`requests.Response`. The body is
/// read eagerly (and decompressed) when the response is returned, so
/// :attr:`content` / :meth:`text` are cheap to read repeatedly.
#[pyclass(name = "HttpResponse", module = "yggdryl")]
pub struct HttpResponse {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    sent_at: f64,
    received_at: f64,
    http_version: String,
    /// The request that produced this response (``requests.Response.request``).
    /// An *unsent* response (a verb called with ``send=False``) carries only this.
    request: Option<HttpRequest>,
}

#[pymethods]
impl HttpResponse {
    /// Construct a response explicitly (useful for tests/mocks and for
    /// ``pickle``). ``headers`` is a list of ``(name, value)`` pairs;
    /// ``http_version`` is the negotiated protocol version (default ``"HTTP/1.1"``).
    #[new]
    #[pyo3(signature = (status, url, headers = None, body = None, sent_at = 0.0, received_at = 0.0, http_version = None))]
    fn new(
        status: u16,
        url: String,
        headers: Option<Vec<(String, String)>>,
        body: Option<Vec<u8>>,
        sent_at: f64,
        received_at: f64,
        http_version: Option<String>,
    ) -> Self {
        HttpResponse {
            status,
            url,
            headers: headers.unwrap_or_default(),
            body: body.unwrap_or_default(),
            sent_at,
            received_at,
            http_version: http_version.unwrap_or_else(|| HttpVersion::Http11.as_str().to_string()),
            request: None,
        }
    }

    /// The HTTP status code (``0`` for an unsent response).
    #[getter]
    fn status(&self) -> u16 {
        self.status
    }

    /// Whether the status is below 400 (the ``requests`` definition of "ok").
    #[getter]
    fn ok(&self) -> bool {
        self.status < 400
    }

    /// Whether this response was actually dispatched. ``False`` for the **unsent**
    /// placeholder a verb returns with ``send=False`` (status ``0``, empty body),
    /// which carries only the prepared :attr:`request`.
    #[getter]
    fn is_sent(&self) -> bool {
        self.status != 0
    }

    /// The request that produced this response (the prepared request), mirroring
    /// :attr:`requests.Response.request`, or ``None``.
    #[getter]
    fn request(&self) -> Option<HttpRequest> {
        self.request.clone()
    }

    /// Dispatch this response's :attr:`request` through the shared session and
    /// return the resulting response — how an **unsent** response (a verb called
    /// with ``send=False``) is sent later. ``raise_error`` (default ``True``) raises
    /// on a 4xx/5xx status.
    #[pyo3(signature = (raise_error = true))]
    fn send(&self, py: Python<'_>, raise_error: bool) -> PyResult<HttpResponse> {
        match &self.request {
            Some(request) => request.send(py, raise_error),
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "this response carries no request to send",
            )),
        }
    }

    /// The final request URL.
    #[getter]
    fn url(&self) -> &str {
        &self.url
    }

    /// The HTTP protocol version the response was delivered over (e.g.
    /// ``"HTTP/1.1"``).
    #[getter]
    fn http_version(&self) -> &str {
        &self.http_version
    }

    /// UTC Unix-epoch seconds when the request was dispatched (``0.0`` if unset).
    #[getter]
    fn sent_at(&self) -> f64 {
        self.sent_at
    }

    /// UTC Unix-epoch seconds when the connection finished delivering the body
    /// (``0.0`` if unset).
    #[getter]
    fn received_at(&self) -> f64 {
        self.received_at
    }

    /// The response headers as a ``dict`` (lower-cased names).
    #[getter]
    fn headers(&self) -> HashMap<String, String> {
        self.headers.iter().cloned().collect()
    }

    /// Look up a header by name (case-insensitive).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The ``Content-Type`` header, if present.
    #[getter]
    fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// The ``Content-Encoding`` header, if present.
    #[getter]
    fn content_encoding(&self) -> Option<&str> {
        self.header("content-encoding")
    }

    /// The single MIME type inferred from ``Content-Type`` (e.g. ``"text/csv"``),
    /// or ``None``.
    #[getter]
    fn mime_type(&self) -> Option<String> {
        self.content_type()
            .and_then(|ct| yggdryl_core::MimeType::from_str(ct).ok())
            .map(|mime| mime.to_string())
    }

    /// The layered media type **combining ``Content-Type`` with ``Content-Encoding``**
    /// as a list of MIME strings: the content type is inner, the transfer encoding
    /// outer — e.g. a gzipped CSV reads as ``["text/csv", "application/gzip"]``.
    /// ``None`` when neither header names a known type.
    #[getter]
    fn media_type(&self) -> Option<Vec<String>> {
        let mut types: Vec<String> = self
            .content_type()
            .and_then(|ct| yggdryl_core::MimeType::from_str(ct).ok())
            .map(|mime| mime.to_string())
            .into_iter()
            .collect();
        if let Some(mime) = self
            .content_encoding()
            .and_then(|enc| yggdryl_core::Compression::from_str(enc).ok())
            .and_then(|codec| codec.mime())
        {
            types.push(mime.to_string());
        }
        (!types.is_empty()).then_some(types)
    }

    /// The compression codec named by ``Content-Encoding`` (``"gzip"`` / ``"zstd"``
    /// / ``"snappy"`` / ``"brotli"``), or ``None``. The body is already decoded —
    /// :attr:`content` / :meth:`text` / :meth:`json` are the decompressed payload.
    #[getter]
    fn compression(&self) -> Option<String> {
        self.content_encoding()
            .and_then(|enc| yggdryl_core::Compression::from_str(enc).ok())
            .filter(|codec| *codec != yggdryl_core::Compression::None)
            .map(|codec| codec.as_str().to_string())
    }

    /// The decompressed body as a yggdryl :class:`BytesIO` handle — the
    /// **performant** accessor: it stays a Rust-backed, seekable byte buffer, so you
    /// can ``json()`` / ``decompress()`` / ``read`` it (or pass it to another yggdryl
    /// call) without copying the bytes into Python. Use :attr:`content` to get native
    /// ``bytes`` only when a Python API requires them.
    #[getter]
    fn io(&self) -> BytesIO {
        BytesIO {
            inner: CoreBytesIO::from_bytes(self.body.clone()),
        }
    }

    /// The raw response body as native ``bytes`` (already decompressed; a copy out
    /// of Rust — prefer :attr:`io` for further Rust-side work).
    #[getter]
    fn content<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.body)
    }

    /// The response body decoded as UTF-8 text (already decompressed).
    fn text(&self) -> PyResult<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    /// The response body parsed as JSON (already decompressed), as Python objects.
    fn json(&self, py: Python<'_>) -> PyResult<PyObject> {
        let value: serde_json::Value = serde_json::from_slice(&self.body)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(crate::json_to_py(py, &value))
    }

    /// Raise ``ValueError`` if the status is 4xx/5xx, otherwise do nothing —
    /// the ``requests`` ``raise_for_status`` pattern.
    fn raise_for_status(&self) -> PyResult<()> {
        if self.status >= 400 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "http status {}",
                self.status
            )));
        }
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!("HttpResponse(status={}, url={:?})", self.status, self.url)
    }

    /// Support ``pickle`` / ``copy`` by reconstructing through the constructor —
    /// the buffered body and metadata are carried verbatim.
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        &self,
        py: Python<'py>,
    ) -> (
        Bound<'py, PyType>,
        (
            u16,
            String,
            Vec<(String, String)>,
            Bound<'py, PyBytes>,
            f64,
            f64,
            String,
        ),
    ) {
        (
            py.get_type_bound::<Self>(),
            (
                self.status,
                self.url.clone(),
                self.headers.clone(),
                PyBytes::new_bound(py, &self.body),
                self.sent_at,
                self.received_at,
                self.http_version.clone(),
            ),
        )
    }
}

/// A connection-pooling HTTP client, like :class:`requests.Session`.
#[pyclass(name = "HttpSession", module = "yggdryl")]
pub struct HttpSession {
    inner: CoreHttpSession,
}

#[pymethods]
impl HttpSession {
    /// Create a session, optionally with a default ``user_agent``, default
    /// ``headers`` sent with every request, a ``max_redirects`` cap, a ``base_url``
    /// that relative targets resolve against, and a default ``http_version``
    /// (``"auto"`` / ``"1.1"`` / ``"2"`` / ``"3"``) for requests that do not pin one.
    ///
    /// ``basic_auth=(username, password)`` or ``bearer_auth=token`` set a default
    /// ``Authorization`` header on every request (HTTP Basic / Bearer), like
    /// ``requests``' ``Session.auth``; it is stripped on a cross-origin redirect.
    /// ``read_timeout`` (seconds, default 120) errors if the server sends no data
    /// for that long; ``0`` removes the bound.
    #[new]
    #[pyo3(signature = (*, user_agent = None, headers = None, max_redirects = None, base_url = None, http_version = None, verify = true, proxy = None, ca_cert = None, ca_cert_file = None, basic_auth = None, bearer_auth = None, read_timeout = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        user_agent: Option<String>,
        headers: Option<HashMap<String, String>>,
        max_redirects: Option<usize>,
        base_url: Option<&str>,
        http_version: Option<&str>,
        verify: bool,
        proxy: Option<&str>,
        ca_cert: Option<Vec<u8>>,
        ca_cert_file: Option<&str>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        read_timeout: Option<f64>,
    ) -> PyResult<Self> {
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
            inner = inner.with_max_redirects(max_redirects);
        }
        if let Some(base_url) = base_url {
            inner =
                inner.with_base_url(yggdryl_core::Url::from_str(base_url).map_err(crate::url_err)?);
        }
        if let Some(http_version) = http_version {
            inner = inner.with_http_version(HttpVersion::from_str(http_version).map_err(http_err)?);
        }
        if !verify {
            inner = inner.with_verify(false);
        }
        if let Some(proxy) = proxy {
            inner = inner.with_proxy(proxy).map_err(http_err)?;
        }
        if let Some(ca_cert) = ca_cert {
            inner = inner.with_ca_cert(&ca_cert).map_err(http_err)?;
        }
        if let Some(ca_cert_file) = ca_cert_file {
            inner = inner.with_ca_cert_file(ca_cert_file).map_err(http_err)?;
        }
        if let Some(read_timeout) = read_timeout {
            inner = inner.with_read_timeout(read_timeout);
        }
        if let Some((username, password)) = basic_auth {
            inner = inner.with_basic_auth(&username, &password);
        }
        if let Some(bearer_auth) = bearer_auth {
            inner = inner.with_bearer_auth(&bearer_auth);
        }
        Ok(HttpSession { inner })
    }

    /// The maximum number of 3xx redirect hops followed per request.
    #[getter]
    fn max_redirects(&self) -> usize {
        self.inner.max_redirects()
    }

    /// The session's base URL (relative request targets resolve against it), or
    /// ``None``.
    #[getter]
    fn base_url(&self) -> Option<String> {
        self.inner.base_url().map(ToString::to_string)
    }

    /// The session's default HTTP protocol version (e.g. ``"auto"``, ``"HTTP/1.1"``)
    /// applied to requests that do not pin their own.
    #[getter]
    fn http_version(&self) -> &str {
        self.inner.http_version().as_str()
    }

    /// Whether TLS certificate verification is performed (``False`` accepts any
    /// certificate — insecure, for self-signed / internal hosts).
    #[getter]
    fn verify(&self) -> bool {
        self.inner.verify()
    }

    /// The proxy URL all requests route through, or ``None`` (defaults to the
    /// environment's ``HTTPS_PROXY`` / ``HTTP_PROXY`` / ``ALL_PROXY``).
    #[getter]
    fn proxy(&self) -> Option<String> {
        self.inner.proxy()
    }

    /// The number of installed CA certificates (``0`` means the default trust store
    /// is used). Install certificates with the ``ca_cert`` / ``ca_cert_file``
    /// constructor arguments.
    #[getter]
    fn ca_cert_count(&self) -> usize {
        self.inner.ca_cert_count()
    }

    /// The read timeout in seconds (a request errors if the server sends no data
    /// for this long; ``0.0`` means unbounded).
    #[getter]
    fn read_timeout(&self) -> f64 {
        self.inner.read_timeout()
    }

    /// An independent copy of this session — same configuration and a snapshot of
    /// the cookie jar, but its own fresh connection pool.
    fn copy(&self) -> HttpSession {
        HttpSession {
            inner: self.inner.copy(),
        }
    }

    /// The session's cookies as a ``dict`` of ``name`` to ``value`` (the jar
    /// snapshot — last value wins for a repeated name).
    #[getter]
    fn cookies(&self) -> HashMap<String, String> {
        self.inner
            .cookies()
            .iter()
            .map(|cookie| (cookie.name().to_string(), cookie.value().to_string()))
            .collect()
    }

    /// Seed a cookie into the session jar, scoped to ``url``'s host (host-only)
    /// and path ``"/"``, so it is sent on matching requests.
    fn set_cookie(&self, url: &str, name: String, value: String) -> PyResult<()> {
        let url = yggdryl_core::Url::from_str(url).map_err(crate::url_err)?;
        self.inner.set_cookie(&url, name, value);
        Ok(())
    }

    /// ``GET url`` (resolved against the session's ``base_url`` when set),
    /// configured from the optional ``headers`` / ``params`` / ``basic_auth`` /
    /// ``bearer_auth`` / ``allow_redirect`` / ``keep_alive`` / ``http_version``.
    /// ``raise_error`` (default ``True``) raises on a 4xx/5xx status. With
    /// ``send=False`` no request is dispatched: an **unsent** :class:`HttpResponse`
    /// carrying the prepared :attr:`~HttpResponse.request` is returned (send it
    /// later with :meth:`HttpResponse.send`).
    #[pyo3(signature = (url, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn get(
        &self,
        py: Python<'_>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let request = build_core_request(
            Some(&self.inner),
            Method::Get,
            url,
            headers,
            params,
            BodyArg::Empty,
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, BodyArg::Empty, raise_error, send)
    }

    /// ``HEAD url`` — same options as :meth:`get`.
    #[pyo3(signature = (url, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn head(
        &self,
        py: Python<'_>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let request = build_core_request(
            Some(&self.inner),
            Method::Head,
            url,
            headers,
            params,
            BodyArg::Empty,
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, BodyArg::Empty, raise_error, send)
    }

    /// ``DELETE url`` — same options as :meth:`get`.
    #[pyo3(signature = (url, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn delete(
        &self,
        py: Python<'_>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let request = build_core_request(
            Some(&self.inner),
            Method::Delete,
            url,
            headers,
            params,
            BodyArg::Empty,
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, BodyArg::Empty, raise_error, send)
    }

    /// ``POST url`` with an optional ``body`` — ``bytes`` or one of our `Io`
    /// handles (a :class:`LocalPath` streams the upload straight off disk) — and the
    /// same options as :meth:`get`.
    #[pyo3(signature = (url, body = None, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn post(
        &self,
        py: Python<'_>,
        url: &str,
        body: Option<Bound<'_, PyAny>>,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let body = extract_body(body.as_ref())?;
        let request = build_core_request(
            Some(&self.inner),
            Method::Post,
            url,
            headers,
            params,
            body.clone(),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, body, raise_error, send)
    }

    /// ``PUT url`` with a ``body`` — same options as :meth:`post`.
    #[pyo3(signature = (url, body = None, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn put(
        &self,
        py: Python<'_>,
        url: &str,
        body: Option<Bound<'_, PyAny>>,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let body = extract_body(body.as_ref())?;
        let request = build_core_request(
            Some(&self.inner),
            Method::Put,
            url,
            headers,
            params,
            body.clone(),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, body, raise_error, send)
    }

    /// ``PATCH url`` with a ``body`` — same options as :meth:`post`.
    #[pyo3(signature = (url, body = None, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn patch(
        &self,
        py: Python<'_>,
        url: &str,
        body: Option<Bound<'_, PyAny>>,
        headers: Option<HashMap<String, String>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let body = extract_body(body.as_ref())?;
        let request = build_core_request(
            Some(&self.inner),
            Method::Patch,
            url,
            headers,
            params,
            body.clone(),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, body, raise_error, send)
    }

    /// Issue an arbitrary ``method`` request, configured from ``headers`` /
    /// ``body`` (``bytes`` or an `Io` handle) / ``params`` / ``basic_auth`` /
    /// ``bearer_auth`` / ``allow_redirect`` / ``keep_alive`` / ``http_version``.
    /// ``raise_error`` (default ``True``) raises ``ValueError`` on a 4xx/5xx status;
    /// pass ``False`` to receive the response whatever its status. With
    /// ``send=False`` the prepared request is returned as an **unsent** response
    /// (see :meth:`get`).
    #[pyo3(signature = (method, url, headers = None, body = None, *, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        py: Python<'_>,
        method: &str,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<Bound<'_, PyAny>>,
        params: Option<HashMap<String, String>>,
        basic_auth: Option<(String, String)>,
        bearer_auth: Option<String>,
        allow_redirect: bool,
        keep_alive: f64,
        http_version: Option<&str>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let method = Method::from_str(method).map_err(http_err)?;
        let body = extract_body(body.as_ref())?;
        let request = build_core_request(
            Some(&self.inner),
            method,
            url,
            headers,
            params,
            body.clone(),
            basic_auth,
            bearer_auth,
            allow_redirect,
            keep_alive,
            http_version,
        )?;
        run_verb(py, &self.inner, request, body, raise_error, send)
    }

    /// Dispatch a prebuilt :class:`HttpRequest` through this session — the
    /// centralised ``send(request) -> HttpResponse`` entry point. ``raise_error``
    /// (default ``True``) raises on a 4xx/5xx status; with ``send=False`` the
    /// prepared request is returned as an **unsent** response instead.
    #[pyo3(signature = (request, *, raise_error = true, send = true))]
    fn send(
        &self,
        py: Python<'_>,
        request: PyRef<'_, HttpRequest>,
        raise_error: bool,
        send: bool,
    ) -> PyResult<HttpResponse> {
        let core = request.to_core()?;
        run_verb(
            py,
            &self.inner,
            core,
            request.body.clone(),
            raise_error,
            send,
        )
    }
}

/// ``GET url`` via the process-wide shared :class:`HttpSession` singleton (the
/// ``requests.get`` equivalent — raises on a 4xx/5xx status). ``url`` is resolved
/// against the shared session's ``base_url`` when one is set (see
/// :func:`set_base_url`). Takes the same options as :meth:`HttpSession.get`,
/// including ``send=False`` to return an **unsent** response.
#[pyfunction]
#[pyo3(name = "get", signature = (url, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_get(
    py: Python<'_>,
    url: &str,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let request = build_core_request(
        Some(&session),
        Method::Get,
        url,
        headers,
        params,
        BodyArg::Empty,
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, BodyArg::Empty, raise_error, send)
}

/// ``HEAD url`` via the shared session singleton — same options as :func:`get`.
#[pyfunction]
#[pyo3(name = "head", signature = (url, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_head(
    py: Python<'_>,
    url: &str,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let request = build_core_request(
        Some(&session),
        Method::Head,
        url,
        headers,
        params,
        BodyArg::Empty,
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, BodyArg::Empty, raise_error, send)
}

/// ``DELETE url`` via the shared session singleton — same options as :func:`get`.
#[pyfunction]
#[pyo3(name = "delete", signature = (url, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_delete(
    py: Python<'_>,
    url: &str,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let request = build_core_request(
        Some(&session),
        Method::Delete,
        url,
        headers,
        params,
        BodyArg::Empty,
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, BodyArg::Empty, raise_error, send)
}

/// ``POST url`` with an optional ``body`` (``bytes`` or an `Io` handle) via the
/// shared session singleton — same options as :meth:`HttpSession.post`.
#[pyfunction]
#[pyo3(name = "post", signature = (url, body = None, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_post(
    py: Python<'_>,
    url: &str,
    body: Option<Bound<'_, PyAny>>,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let body = extract_body(body.as_ref())?;
    let request = build_core_request(
        Some(&session),
        Method::Post,
        url,
        headers,
        params,
        body.clone(),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, body, raise_error, send)
}

/// ``PUT url`` with a ``body`` via the shared session singleton.
#[pyfunction]
#[pyo3(name = "put", signature = (url, body = None, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_put(
    py: Python<'_>,
    url: &str,
    body: Option<Bound<'_, PyAny>>,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let body = extract_body(body.as_ref())?;
    let request = build_core_request(
        Some(&session),
        Method::Put,
        url,
        headers,
        params,
        body.clone(),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, body, raise_error, send)
}

/// ``PATCH url`` with a ``body`` via the shared session singleton.
#[pyfunction]
#[pyo3(name = "patch", signature = (url, body = None, *, headers = None, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_patch(
    py: Python<'_>,
    url: &str,
    body: Option<Bound<'_, PyAny>>,
    headers: Option<HashMap<String, String>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let body = extract_body(body.as_ref())?;
    let request = build_core_request(
        Some(&session),
        Method::Patch,
        url,
        headers,
        params,
        body.clone(),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, body, raise_error, send)
}

/// Configure the process-wide shared :class:`HttpSession` singleton with a
/// ``base_url`` (replacing it), so the module-level verbs resolve relative targets
/// — e.g. ``set_base_url("https://api.example.com")`` then ``get("/users")``.
#[pyfunction]
#[pyo3(name = "set_base_url")]
pub fn set_base_url(base_url: &str) -> PyResult<()> {
    let base = yggdryl_core::Url::from_str(base_url).map_err(crate::url_err)?;
    CoreHttpSession::set_shared(CoreHttpSession::new().with_base_url(base));
    Ok(())
}

/// Issue an arbitrary ``method`` request via the shared session singleton (same
/// arguments as :meth:`HttpSession.request`, including ``send=False``).
#[pyfunction]
#[pyo3(name = "request", signature = (method, url, headers = None, body = None, *, params = None, basic_auth = None, bearer_auth = None, allow_redirect = true, keep_alive = 300.0, http_version = None, raise_error = true, send = true))]
#[allow(clippy::too_many_arguments)]
pub fn http_request(
    py: Python<'_>,
    method: &str,
    url: &str,
    headers: Option<HashMap<String, String>>,
    body: Option<Bound<'_, PyAny>>,
    params: Option<HashMap<String, String>>,
    basic_auth: Option<(String, String)>,
    bearer_auth: Option<String>,
    allow_redirect: bool,
    keep_alive: f64,
    http_version: Option<&str>,
    raise_error: bool,
    send: bool,
) -> PyResult<HttpResponse> {
    let session = CoreHttpSession::shared();
    let method = Method::from_str(method).map_err(http_err)?;
    let body = extract_body(body.as_ref())?;
    let request = build_core_request(
        Some(&session),
        method,
        url,
        headers,
        params,
        body.clone(),
        basic_auth,
        bearer_auth,
        allow_redirect,
        keep_alive,
        http_version,
    )?;
    run_verb(py, &session, request, body, raise_error, send)
}
