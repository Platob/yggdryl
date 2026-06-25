//! The `HttpSession` and `HttpResponse` pyclasses (a `requests`-like client).

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{LocalPath as CoreLocalPath, Path};
use yggdryl_http::{
    HttpRequest as CoreHttpRequest, HttpResponse as CoreHttpResponse,
    HttpSession as CoreHttpSession, Method,
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
}

#[pymethods]
impl HttpResponse {
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
}

/// A connection-pooling HTTP client, like :class:`requests.Session`.
#[pyclass(name = "HttpSession", module = "yggdryl")]
pub struct HttpSession {
    inner: CoreHttpSession,
}

impl HttpSession {
    /// Runs `build` to issue a request and drains the response body — all while
    /// the GIL is **released** (`allow_threads`), so a Python server thread can
    /// answer a blocking request without deadlocking — then buffers the result.
    fn run(
        &self,
        py: Python<'_>,
        build: impl FnOnce(&CoreHttpSession) -> Result<CoreHttpResponse, yggdryl_http::HttpError> + Send,
    ) -> PyResult<HttpResponse> {
        let (status, url, headers, body, sent_at, received_at) = py
            .allow_threads(|| {
                // The closures issue a buffered (`stream = false`) send, so the body
                // is drained inside `send` and `received_at` is already stamped here.
                let response = build(&self.inner)?;
                let status = response.status();
                let url = response.url().to_string();
                let headers = response
                    .headers()
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect::<Vec<_>>();
                let sent_at = response.sent_at();
                let received_at = response.received_at();
                let body = response.bytes()?;
                Ok::<_, yggdryl_http::HttpError>((status, url, headers, body, sent_at, received_at))
            })
            .map_err(http_err)?;
        Ok(HttpResponse {
            status,
            url,
            headers,
            body,
            sent_at,
            received_at,
        })
    }
}

#[pymethods]
impl HttpSession {
    /// Create a session, optionally with a default ``user_agent`` and default
    /// ``headers`` sent with every request.
    #[new]
    #[pyo3(signature = (*, user_agent = None, headers = None, max_redirects = None))]
    fn new(
        user_agent: Option<String>,
        headers: Option<HashMap<String, String>>,
        max_redirects: Option<usize>,
    ) -> Self {
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
        HttpSession { inner }
    }

    /// The maximum number of 3xx redirect hops followed per request.
    #[getter]
    fn max_redirects(&self) -> usize {
        self.inner.max_redirects()
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

    /// ``GET url``.
    fn get(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| {
            session.send(CoreHttpRequest::get(url)?, true, true, false)
        })
    }

    /// ``HEAD url``.
    fn head(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| {
            session.send(CoreHttpRequest::head(url)?, true, true, false)
        })
    }

    /// ``DELETE url``.
    fn delete(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| {
            session.send(CoreHttpRequest::delete(url)?, true, true, false)
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
            session.send(
                apply_body(CoreHttpRequest::post(url)?, body),
                true,
                true,
                false,
            )
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
            session.send(
                apply_body(CoreHttpRequest::put(url)?, body),
                true,
                true,
                false,
            )
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
            session.send(
                apply_body(CoreHttpRequest::patch(url)?, body),
                true,
                true,
                false,
            )
        })
    }

    /// Issue an arbitrary ``method`` request, with optional ``headers`` and
    /// ``body`` (``bytes`` or an `Io` handle). ``raise_error`` (default ``True``)
    /// raises ``ValueError`` on a 4xx/5xx status; pass ``False`` to receive the
    /// response whatever its status. ``keep_alive`` (default ``True``) pools the
    /// connection for reuse (skipping the next TLS handshake); pass ``False`` to
    /// close it after the response. ``allow_redirect`` (default ``True``) follows
    /// 3xx redirects (up to the session's ``max_redirects``); pass ``False`` to
    /// receive the 3xx response itself. The body is always buffered (the response
    /// exposes :attr:`content` repeatedly), so the connection is released at once.
    #[pyo3(signature = (method, url, headers = None, body = None, *, raise_error = true, keep_alive = true, allow_redirect = true))]
    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        py: Python<'_>,
        method: &str,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<Bound<'_, PyAny>>,
        raise_error: bool,
        keep_alive: bool,
        allow_redirect: bool,
    ) -> PyResult<HttpResponse> {
        let method = Method::from_str(method).map_err(http_err)?;
        let body = extract_body(body.as_ref())?;
        self.run(py, move |session| {
            let mut request =
                CoreHttpRequest::new(method, url)?.with_allow_redirect(allow_redirect);
            if let Some(headers) = headers {
                request = request.with_headers(headers);
            }
            request = apply_body(request, body);
            session.send(request, raise_error, keep_alive, false)
        })
    }
}
