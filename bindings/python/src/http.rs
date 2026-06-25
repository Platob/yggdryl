//! The `HttpSession` and `HttpResponse` pyclasses (a `requests`-like client).

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_http::{
    HttpRequest as CoreHttpRequest, HttpResponse as CoreHttpResponse,
    HttpSession as CoreHttpSession, Method,
};

use crate::http_err;

/// A received HTTP response, modelled on :class:`requests.Response`. The body is
/// read eagerly (and decompressed) when the response is returned, so
/// :attr:`content` / :meth:`text` are cheap to read repeatedly.
#[pyclass(name = "HttpResponse", module = "yggdryl")]
pub struct HttpResponse {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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
        let (status, url, headers, body) = py
            .allow_threads(|| {
                let response = build(&self.inner)?;
                let status = response.status();
                let url = response.url().to_string();
                let headers = response.headers().to_vec();
                let body = response.bytes()?;
                Ok::<_, yggdryl_http::HttpError>((status, url, headers, body))
            })
            .map_err(http_err)?;
        Ok(HttpResponse {
            status,
            url,
            headers,
            body,
        })
    }
}

#[pymethods]
impl HttpSession {
    /// Create a session, optionally with a default ``user_agent`` and default
    /// ``headers`` sent with every request.
    #[new]
    #[pyo3(signature = (*, user_agent = None, headers = None))]
    fn new(user_agent: Option<String>, headers: Option<HashMap<String, String>>) -> Self {
        let mut inner = CoreHttpSession::new();
        if let Some(user_agent) = user_agent {
            inner = inner.with_user_agent(user_agent);
        }
        if let Some(headers) = headers {
            for (key, value) in headers {
                inner = inner.with_header(key, value);
            }
        }
        HttpSession { inner }
    }

    /// ``GET url``.
    fn get(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| session.get(url))
    }

    /// ``HEAD url``.
    fn head(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| session.head(url))
    }

    /// ``DELETE url``.
    fn delete(&self, py: Python<'_>, url: &str) -> PyResult<HttpResponse> {
        self.run(py, |session| session.delete(url))
    }

    /// ``POST url`` with an optional byte ``body``.
    #[pyo3(signature = (url, body = None))]
    fn post(&self, py: Python<'_>, url: &str, body: Option<Vec<u8>>) -> PyResult<HttpResponse> {
        let body = body.unwrap_or_default();
        self.run(py, move |session| session.post(url, body))
    }

    /// ``PUT url`` with a byte ``body``.
    #[pyo3(signature = (url, body = None))]
    fn put(&self, py: Python<'_>, url: &str, body: Option<Vec<u8>>) -> PyResult<HttpResponse> {
        let body = body.unwrap_or_default();
        self.run(py, move |session| session.put(url, body))
    }

    /// ``PATCH url`` with a byte ``body``.
    #[pyo3(signature = (url, body = None))]
    fn patch(&self, py: Python<'_>, url: &str, body: Option<Vec<u8>>) -> PyResult<HttpResponse> {
        let body = body.unwrap_or_default();
        self.run(py, move |session| session.patch(url, body))
    }

    /// Issue an arbitrary ``method`` request, with optional ``headers`` and
    /// ``body``.
    #[pyo3(signature = (method, url, headers = None, body = None))]
    fn request(
        &self,
        py: Python<'_>,
        method: &str,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<Vec<u8>>,
    ) -> PyResult<HttpResponse> {
        let method = Method::from_str(method).map_err(http_err)?;
        self.run(py, move |session| {
            let mut request = CoreHttpRequest::new(method, url)?;
            if let Some(headers) = headers {
                request = request.with_headers(headers);
            }
            if let Some(body) = body {
                request = request.with_body(body);
            }
            session.request(request)
        })
    }
}
