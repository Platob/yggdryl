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
use yggdryl_http::{HttpRequest as CoreHttpRequest, HttpSession as CoreHttpSession, Method};

fn to_napi(err: yggdryl_http::HttpError) -> Error {
    Error::from_reason(err.to_string())
}

/// The data drained from a response on the worker thread, before it is handed
/// back to JS as an [`HttpResponse`].
pub struct ResponseData {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// The blocking request, run on the libuv thread pool by napi.
pub struct RequestTask {
    session: Arc<CoreHttpSession>,
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    raise_error: bool,
}

impl Task for RequestTask {
    type Output = ResponseData;
    type JsValue = HttpResponse;

    fn compute(&mut self) -> Result<ResponseData> {
        let mut request = CoreHttpRequest::new(self.method, &self.url).map_err(to_napi)?;
        request = request.with_headers(std::mem::take(&mut self.headers));
        if let Some(body) = self.body.take() {
            request = request.with_body(body);
        }
        let response = self
            .session
            .request(request, self.raise_error)
            .map_err(to_napi)?;
        let status = response.status();
        let url = response.url().to_string();
        let headers = response.headers().to_vec();
        let body = response.bytes().map_err(to_napi)?;
        Ok(ResponseData {
            status,
            url,
            headers,
            body,
        })
    }

    fn resolve(&mut self, _env: Env, output: ResponseData) -> Result<HttpResponse> {
        Ok(HttpResponse {
            status: output.status,
            url: output.url,
            headers: output.headers,
            body: output.body,
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
    /// Create a session, optionally with a default `userAgent` and default
    /// `headers` sent with every request.
    #[napi(constructor)]
    pub fn new(user_agent: Option<String>, headers: Option<HashMap<String, String>>) -> Self {
        let mut inner = CoreHttpSession::new();
        if let Some(user_agent) = user_agent {
            inner = inner.with_user_agent(user_agent);
        }
        if let Some(headers) = headers {
            for (key, value) in headers {
                inner = inner.with_header(key, value);
            }
        }
        HttpSession {
            inner: Arc::new(inner),
        }
    }

    fn task(
        &self,
        method: Method,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        raise_error: bool,
    ) -> AsyncTask<RequestTask> {
        AsyncTask::new(RequestTask {
            session: self.inner.clone(),
            method,
            url,
            headers,
            body,
            raise_error,
        })
    }

    /// `GET url` (raises on a 4xx/5xx status).
    #[napi]
    pub fn get(&self, url: String) -> AsyncTask<RequestTask> {
        self.task(Method::Get, url, Vec::new(), None, true)
    }

    /// `HEAD url` (raises on a 4xx/5xx status).
    #[napi]
    pub fn head(&self, url: String) -> AsyncTask<RequestTask> {
        self.task(Method::Head, url, Vec::new(), None, true)
    }

    /// `DELETE url` (raises on a 4xx/5xx status).
    #[napi]
    pub fn delete(&self, url: String) -> AsyncTask<RequestTask> {
        self.task(Method::Delete, url, Vec::new(), None, true)
    }

    /// `POST url` with an optional byte `body` (raises on a 4xx/5xx status).
    #[napi]
    pub fn post(&self, url: String, body: Option<Buffer>) -> AsyncTask<RequestTask> {
        self.task(
            Method::Post,
            url,
            Vec::new(),
            body.map(|b| b.to_vec()),
            true,
        )
    }

    /// `PUT url` with a byte `body` (raises on a 4xx/5xx status).
    #[napi]
    pub fn put(&self, url: String, body: Option<Buffer>) -> AsyncTask<RequestTask> {
        self.task(Method::Put, url, Vec::new(), body.map(|b| b.to_vec()), true)
    }

    /// `PATCH url` with a byte `body` (raises on a 4xx/5xx status).
    #[napi]
    pub fn patch(&self, url: String, body: Option<Buffer>) -> AsyncTask<RequestTask> {
        self.task(
            Method::Patch,
            url,
            Vec::new(),
            body.map(|b| b.to_vec()),
            true,
        )
    }

    /// Issue an arbitrary `method` request, with optional `headers` and `body`.
    /// `raiseError` (default `true`) throws on a 4xx/5xx status.
    #[napi]
    pub fn request(
        &self,
        method: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<Buffer>,
        raise_error: Option<bool>,
    ) -> Result<AsyncTask<RequestTask>> {
        let method = Method::from_str(&method).map_err(to_napi)?;
        let headers = headers
            .map(|map| map.into_iter().collect())
            .unwrap_or_default();
        Ok(self.task(
            method,
            url,
            headers,
            body.map(|b| b.to_vec()),
            raise_error.unwrap_or(true),
        ))
    }
}
