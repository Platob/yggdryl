//! The `HttpSession` and `HttpResponse` pyclasses (a `requests`-like client).

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyType};
use yggdryl_core::{LocalPath as CoreLocalPath, Path};
use yggdryl_http::{
    HttpRequest as CoreHttpRequest, HttpResponse as CoreHttpResponse,
    HttpSession as CoreHttpSession, HttpVersion, Method,
};

use crate::bytesio::BytesIO;
use crate::http_err;
use crate::localpath::LocalPath;

/// A request body extracted from a Python argument: raw `bytes`, or one of our
/// `Io` handles (a `LocalPath` streams straight off disk; a `BytesIO`'s bytes are
/// taken). `Send`, so it crosses into the GIL-released worker.
enum BodyArg {
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
        }
    }

    /// The HTTP status code.
    #[getter]
    fn status(&self) -> u16 {
        self.status
    }

    /// Whether the status is below 400 (the ``requests`` definition of "ok").
    #[getter]
    fn ok(&self) -> bool {
        self.status < 400
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

    /// The raw response body as ``bytes``.
    #[getter]
    fn content<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.body)
    }

    /// The response body decoded as UTF-8 text.
    fn text(&self) -> PyResult<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
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

/// Issues a request via `build` and drains the response body — all while the GIL
/// is **released** (`allow_threads`), so a Python server thread can answer a
/// blocking request without deadlocking — then buffers it into an [`HttpResponse`].
/// Shared by [`HttpSession`]'s methods and the module-level [`get`] / [`post`] / …
/// functions (which build over the shared singleton).
fn buffer_response(
    py: Python<'_>,
    build: impl FnOnce() -> Result<CoreHttpResponse, yggdryl_http::HttpError> + Send,
) -> PyResult<HttpResponse> {
    let (status, url, headers, body, sent_at, received_at, http_version) = py
        .allow_threads(|| {
            // Every send streams; we drain the body here (off the GIL) into an
            // owned buffer so `content` / `text` are cheap to read repeatedly.
            // `read_all` drains and reports `received_at` (stamped at EOF) together.
            let response = build()?;
            let status = response.status();
            let url = response.url().to_string();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect::<Vec<_>>();
            let sent_at = response.sent_at();
            let http_version = response.negotiated_version().as_str().to_string();
            let (body, received_at) = response.read_all()?;
            Ok::<_, yggdryl_http::HttpError>((
                status,
                url,
                headers,
                body,
                sent_at,
                received_at,
                http_version,
            ))
        })
        .map_err(http_err)?;
    Ok(HttpResponse {
        status,
        url,
        headers,
        body,
        sent_at,
        received_at,
        http_version,
    })
}

impl HttpSession {
    /// Runs `build` over this session's inner client and buffers the response
    /// (see [`buffer_response`]).
    fn run(
        &self,
        py: Python<'_>,
        build: impl FnOnce(&CoreHttpSession) -> Result<CoreHttpResponse, yggdryl_http::HttpError> + Send,
    ) -> PyResult<HttpResponse> {
        buffer_response(py, || build(&self.inner))
    }
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

    /// ``GET url`` (resolved against the session's ``base_url`` when set).
    fn get(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| {
            let request = CoreHttpRequest::from_url(Method::Get, session.resolve_url(url)?);
            session.send(request, true)
        })
    }

    /// ``HEAD url``.
    fn head(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| {
            let request = CoreHttpRequest::from_url(Method::Head, session.resolve_url(url)?);
            session.send(request, true)
        })
    }

    /// ``DELETE url``.
    fn delete(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| {
            let request = CoreHttpRequest::from_url(Method::Delete, session.resolve_url(url)?);
            session.send(request, true)
        })
    }

    /// ``POST url`` with an optional ``body`` — ``bytes`` or one of our `Io`
    /// handles (a :class:`LocalPath` streams the upload straight off disk).
    #[pyo3(signature = (url, body = None))]
    fn post(
        &self,
        py: Python<'_>,
        url: &str,
        body: Option<Bound<'_, PyAny>>,
    ) -> PyResult<HttpResponse> {
        let body = extract_body(body.as_ref())?;
        self.run(py, move |session| {
            let request = CoreHttpRequest::from_url(Method::Post, session.resolve_url(url)?);
            session.send(apply_body(request, body), true)
        })
    }

    /// ``PUT url`` with a ``body`` — ``bytes`` or an `Io` handle.
    #[pyo3(signature = (url, body = None))]
    fn put(
        &self,
        py: Python<'_>,
        url: &str,
        body: Option<Bound<'_, PyAny>>,
    ) -> PyResult<HttpResponse> {
        let body = extract_body(body.as_ref())?;
        self.run(py, move |session| {
            let request = CoreHttpRequest::from_url(Method::Put, session.resolve_url(url)?);
            session.send(apply_body(request, body), true)
        })
    }

    /// ``PATCH url`` with a ``body`` — ``bytes`` or an `Io` handle.
    #[pyo3(signature = (url, body = None))]
    fn patch(
        &self,
        py: Python<'_>,
        url: &str,
        body: Option<Bound<'_, PyAny>>,
    ) -> PyResult<HttpResponse> {
        let body = extract_body(body.as_ref())?;
        self.run(py, move |session| {
            let request = CoreHttpRequest::from_url(Method::Patch, session.resolve_url(url)?);
            session.send(apply_body(request, body), true)
        })
    }

    /// Issue an arbitrary ``method`` request, with optional ``headers`` and
    /// ``body`` (``bytes`` or an `Io` handle). ``raise_error`` (default ``True``)
    /// raises ``ValueError`` on a 4xx/5xx status; pass ``False`` to receive the
    /// response whatever its status. ``keep_alive`` is the keep-alive idle TTL in
    /// seconds (default 300 — 5 minutes; ``0`` sends ``Connection: close``).
    /// ``allow_redirect`` (default ``True``) follows
    /// 3xx redirects (up to the session's ``max_redirects``); pass ``False`` to
    /// receive the 3xx response itself. ``http_version`` (e.g. ``"2"``) pins the
    /// protocol version for this request, overriding the session default. The body
    /// is always buffered (the response exposes :attr:`content` repeatedly), so the
    /// connection is released at once.
    #[pyo3(signature = (method, url, headers = None, body = None, *, raise_error = true, keep_alive = 300.0, allow_redirect = true, http_version = None))]
    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        py: Python<'_>,
        method: &str,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<Bound<'_, PyAny>>,
        raise_error: bool,
        keep_alive: f64,
        allow_redirect: bool,
        http_version: Option<&str>,
    ) -> PyResult<HttpResponse> {
        let method = Method::from_str(method).map_err(http_err)?;
        let http_version = http_version
            .map(|value| HttpVersion::from_str(value).map_err(http_err))
            .transpose()?;
        let body = extract_body(body.as_ref())?;
        self.run(py, move |session| {
            let mut request = CoreHttpRequest::from_url(method, session.resolve_url(url)?)
                .with_allow_redirect(allow_redirect);
            if let Some(http_version) = http_version {
                request = request.with_http_version(http_version);
            }
            if let Some(headers) = headers {
                request = request.with_headers(headers);
            }
            request = apply_body(request, body);
            session.send(request.with_keep_alive(keep_alive), raise_error)
        })
    }
}

/// ``GET url`` via the process-wide shared :class:`HttpSession` singleton (the
/// ``requests.get`` equivalent — raises on a 4xx/5xx status). ``url`` is resolved
/// against the shared session's ``base_url`` when one is set (see
/// :func:`set_base_url`).
#[pyfunction]
#[pyo3(name = "get")]
pub fn http_get(py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let request = CoreHttpRequest::from_url(Method::Get, session.resolve_url(url)?);
        session.send(request, true)
    })
}

/// ``HEAD url`` via the shared session singleton (raises on a 4xx/5xx status).
#[pyfunction]
#[pyo3(name = "head")]
pub fn http_head(py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let request = CoreHttpRequest::from_url(Method::Head, session.resolve_url(url)?);
        session.send(request, true)
    })
}

/// ``DELETE url`` via the shared session singleton (raises on a 4xx/5xx status).
#[pyfunction]
#[pyo3(name = "delete")]
pub fn http_delete(py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let request = CoreHttpRequest::from_url(Method::Delete, session.resolve_url(url)?);
        session.send(request, true)
    })
}

/// ``POST url`` with an optional ``body`` (``bytes`` or an `Io` handle) via the
/// shared session singleton.
#[pyfunction]
#[pyo3(name = "post", signature = (url, body = None))]
pub fn http_post(
    py: Python<'_>,
    url: &str,
    body: Option<Bound<'_, PyAny>>,
) -> PyResult<HttpResponse> {
    let body = extract_body(body.as_ref())?;
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let request = CoreHttpRequest::from_url(Method::Post, session.resolve_url(url)?);
        session.send(apply_body(request, body), true)
    })
}

/// ``PUT url`` with a ``body`` via the shared session singleton.
#[pyfunction]
#[pyo3(name = "put", signature = (url, body = None))]
pub fn http_put(
    py: Python<'_>,
    url: &str,
    body: Option<Bound<'_, PyAny>>,
) -> PyResult<HttpResponse> {
    let body = extract_body(body.as_ref())?;
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let request = CoreHttpRequest::from_url(Method::Put, session.resolve_url(url)?);
        session.send(apply_body(request, body), true)
    })
}

/// ``PATCH url`` with a ``body`` via the shared session singleton.
#[pyfunction]
#[pyo3(name = "patch", signature = (url, body = None))]
pub fn http_patch(
    py: Python<'_>,
    url: &str,
    body: Option<Bound<'_, PyAny>>,
) -> PyResult<HttpResponse> {
    let body = extract_body(body.as_ref())?;
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let request = CoreHttpRequest::from_url(Method::Patch, session.resolve_url(url)?);
        session.send(apply_body(request, body), true)
    })
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
/// arguments as :meth:`HttpSession.request`).
#[pyfunction]
#[pyo3(name = "request", signature = (method, url, headers = None, body = None, *, raise_error = true, keep_alive = 300.0, allow_redirect = true, http_version = None))]
#[allow(clippy::too_many_arguments)]
pub fn http_request(
    py: Python<'_>,
    method: &str,
    url: &str,
    headers: Option<HashMap<String, String>>,
    body: Option<Bound<'_, PyAny>>,
    raise_error: bool,
    keep_alive: f64,
    allow_redirect: bool,
    http_version: Option<&str>,
) -> PyResult<HttpResponse> {
    let method = Method::from_str(method).map_err(http_err)?;
    let http_version = http_version
        .map(|value| HttpVersion::from_str(value).map_err(http_err))
        .transpose()?;
    let body = extract_body(body.as_ref())?;
    buffer_response(py, move || {
        let session = CoreHttpSession::shared();
        let mut request = CoreHttpRequest::from_url(method, session.resolve_url(url)?)
            .with_allow_redirect(allow_redirect);
        if let Some(http_version) = http_version {
            request = request.with_http_version(http_version);
        }
        if let Some(headers) = headers {
            request = request.with_headers(headers);
        }
        request = apply_body(request, body);
        session.send(request.with_keep_alive(keep_alive), raise_error)
    })
}
