//! Python views over the core HTTP client, its sessions, requests, responses,
//! resumable streams and paginated walks, and the server that hosts a handle.
//!
//! The four storage roles - `Session`, `Request`, `Response`, `Stream` - are
//! `IOBase` subclasses over their `Holder` variants, exactly as `S3File` is,
//! so every byte and record verb answers on them through the core. What this
//! file adds is the requests-shaped spelling of the core's own verbs: it
//! coerces Python arguments once into `Headers`, `Body`, `Authorization` and a
//! `Duration`, and redirects. Every call that reaches the network releases the
//! interpreter, so Python threads issue requests side by side; the session is
//! a shared handle and a response's body sits behind the core's own lock.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{
    PyByteArray, PyBytes, PyDelta, PyDict, PyIterator, PyList, PyMapping, PyMemoryView, PyString,
    PyTuple,
};

use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{
    Authorization, Client, ContentRange, Fault, Headers, HttpOptions, Link, Method, Pages,
    Pagination, Recorded, Request, Response, Server, ServerOptions, Session, StatsSnapshot, Status,
    Stream,
};
use yggdryl::{Charset, Codec, FieldPath, IOBase as _, IOKind};

use crate::holder::fs::storage_error;
use crate::iobase::{PyIOBase, describe};
use crate::scalar::{PyScalar, as_py, from_py};
use crate::uri::{PyUri, core_url_from_value, url_object};

/// Report a handle whose holder is no longer the role its class names.
fn no_longer(role: &str) -> PyErr {
    PyValueError::new_err(format!("this handle is no longer an HTTP {role}"))
}

/// Report a lock a panicking thread left behind.
fn poisoned(what: &str) -> PyErr {
    PyValueError::new_err(format!(
        "the {what} lock was poisoned by a panicking thread"
    ))
}

/// The session a `Session` handle holds, cloned out: a shared handle, so the
/// clone is one reference count and the interpreter can be released.
fn session_of(base: &PyIOBase) -> PyResult<Session> {
    match base.inner()? {
        Holder::HttpSession(session) => Ok(session.clone()),
        _ => Err(no_longer("session")),
    }
}

/// The request a `Request` handle holds, cloned out.
fn request_of(base: &PyIOBase) -> PyResult<Request> {
    match base.inner()? {
        Holder::HttpRequest(request) => Ok(request.clone()),
        _ => Err(no_longer("request")),
    }
}

/// Borrow the response a `Response` handle holds.
fn response_of(base: &PyIOBase) -> PyResult<&Response> {
    match base.inner()? {
        Holder::HttpResponse(response) => Ok(response),
        _ => Err(no_longer("response")),
    }
}

/// Borrow the stream a `Stream` handle holds.
fn stream_of(base: &PyIOBase) -> PyResult<&Stream> {
    match base.inner()? {
        Holder::HttpStream(stream) => Ok(stream),
        _ => Err(no_longer("stream")),
    }
}

/// Read the session out of any `Session` handle a caller passed.
fn session_from(value: &Bound<'_, PyAny>) -> PyResult<Session> {
    let handle = value.cast::<PyIOBase>().map_err(|_| {
        PyTypeError::new_err(format!(
            "expected a yggdryl.http.Session, got {}",
            type_name(value)
        ))
    })?;
    session_of(&handle.borrow())
}

/// Read the request out of any `Request` handle a caller passed.
fn request_from(value: &Bound<'_, PyAny>) -> PyResult<Request> {
    let handle = value.cast::<PyIOBase>().map_err(|_| {
        PyTypeError::new_err(format!(
            "expected a yggdryl.http.Request, got {}",
            type_name(value)
        ))
    })?;
    request_of(&handle.borrow())
}

fn type_name(value: &Bound<'_, PyAny>) -> String {
    value
        .get_type()
        .name()
        .map_or_else(|_| "an object".to_owned(), |name| name.to_string())
}

/// Read a URL argument: text, or a `Url` whose canonical spelling is taken.
fn url_text(value: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(text.to_str()?.to_owned());
    }
    if value.is_instance_of::<PyUri>() {
        return Ok(value.str()?.to_str()?.to_owned());
    }
    Err(PyTypeError::new_err(format!(
        "expected a URL string or yggdryl.Url, got {}",
        type_name(value)
    )))
}

/// Read a duration: a `datetime.timedelta`, or seconds as a number.
fn duration_of(value: &Bound<'_, PyAny>, name: &str) -> PyResult<Duration> {
    if value.is_instance_of::<PyDelta>() {
        return value.extract::<Duration>().map_err(|_| {
            PyValueError::new_err(format!(
                "expected a non-negative timedelta for {name}, got {}",
                value
                    .repr()
                    .map_or_else(|_| "one".to_owned(), |text| text.to_string())
            ))
        });
    }
    let seconds = value.extract::<f64>().map_err(|_| {
        PyTypeError::new_err(format!(
            "expected seconds or a datetime.timedelta for {name}, got {}",
            type_name(value)
        ))
    })?;
    Duration::try_from_secs_f64(seconds).map_err(|_| {
        PyValueError::new_err(format!(
            "expected a non-negative, finite number of seconds for {name}, got {seconds}"
        ))
    })
}

/// Append the text one query or form value spells: a string, an integer, a
/// float, or a list of those repeating the name; `None` sends nothing.
fn push_values(
    pairs: &mut Vec<(String, String)>,
    what: &str,
    name: &str,
    value: &Bound<'_, PyAny>,
    nested: bool,
) -> PyResult<()> {
    if value.is_none() {
        return Ok(());
    }
    if let Ok(text) = value.cast::<PyString>() {
        pairs.push((name.to_owned(), text.to_str()?.to_owned()));
        return Ok(());
    }
    if !value.is_instance_of::<pyo3::types::PyBool>()
        && (value.is_instance_of::<pyo3::types::PyInt>()
            || value.is_instance_of::<pyo3::types::PyFloat>())
    {
        pairs.push((name.to_owned(), value.str()?.to_str()?.to_owned()));
        return Ok(());
    }
    if !nested && (value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>()) {
        for member in value.try_iter()? {
            push_values(pairs, what, name, &member?, true)?;
        }
        return Ok(());
    }
    Err(PyTypeError::new_err(format!(
        "expected a str, int, float, or a list of them for the {what} value {name:?}, got {}",
        type_name(value)
    )))
}

/// Read `(name, value)` pairs out of a mapping or an iterable of pairs.
fn pairs_of(value: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec<(String, String)>> {
    let items = if value.hasattr("items")? {
        value.call_method0("items")?
    } else {
        value.clone()
    };
    let mut pairs = Vec::new();
    for item in items.try_iter()? {
        let (name, member): (String, Bound<'_, PyAny>) = item?.extract().map_err(|_| {
            PyTypeError::new_err(format!(
                "expected a mapping or (name, value) pairs for {what}"
            ))
        })?;
        push_values(&mut pairs, what, &name, &member, false)?;
    }
    Ok(pairs)
}

/// Read a header section: a `Headers`, a mapping, or `(name, value)` pairs.
fn headers_of(value: &Bound<'_, PyAny>) -> PyResult<Headers> {
    if let Ok(headers) = value.cast::<PyHeaders>() {
        return Ok(headers.get().inner.clone());
    }
    Headers::from_entries(pairs_of(value, "headers")?).map_err(storage_error)
}

/// Read a credential: `(user, password)` is Basic, a string a Bearer token.
fn authorization_of(value: &Bound<'_, PyAny>) -> PyResult<Authorization> {
    if let Ok(token) = value.cast::<PyString>() {
        return Ok(Authorization::bearer(token.to_str()?));
    }
    if let Ok((user, password)) = value.extract::<(String, String)>() {
        return Ok(Authorization::basic(user, password));
    }
    Err(PyTypeError::new_err(format!(
        "expected auth as a (user, password) pair or a bearer token string, got {}",
        type_name(value)
    )))
}

/// Read bytes a caller hands over as a body: bytes-like, or text as UTF-8.
fn body_bytes(value: &Bound<'_, PyAny>) -> PyResult<Option<Vec<u8>>> {
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(Some(bytes.as_bytes().to_vec()));
    }
    if let Ok(bytes) = value.cast::<PyByteArray>() {
        return Ok(Some(bytes.to_vec()));
    }
    if value.is_instance_of::<PyMemoryView>() {
        return Ok(Some(value.call_method0("tobytes")?.extract()?));
    }
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(Some(text.to_str()?.as_bytes().to_vec()));
    }
    Ok(None)
}

/// Read a mapping of option names to values, each value taken as its text,
/// `None` skipped: `HttpOptions::from_properties` reads the names.
fn property_pairs(value: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    let items = if value.hasattr("items")? {
        value.call_method0("items")?
    } else {
        value.clone()
    };
    let mut pairs = Vec::new();
    for item in items.try_iter()? {
        let (name, member): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
        if member.is_none() {
            continue;
        }
        pairs.push((
            name.str()?.to_str()?.to_owned(),
            member.str()?.to_str()?.to_owned(),
        ));
    }
    Ok(pairs)
}

/// What a session starts every request from, as the constructor spells it.
struct Settings<'a, 'py> {
    base_url: Option<&'a Bound<'py, PyAny>>,
    headers: Option<&'a Bound<'py, PyAny>>,
    auth: Option<&'a Bound<'py, PyAny>>,
    timeout: Option<&'a Bound<'py, PyAny>>,
    options: Option<&'a Bound<'py, PyAny>>,
}

impl Settings<'_, '_> {
    /// Resolve the settings into one `HttpOptions`: the mapping first, then
    /// each named argument over it.
    fn resolve(&self) -> PyResult<HttpOptions> {
        let mut options = match self.options {
            None => HttpOptions::default(),
            Some(mapping) => {
                HttpOptions::from_properties(property_pairs(mapping)?).map_err(storage_error)?
            }
        };
        if let Some(base_url) = self.base_url {
            options = options.with_base_url(core_url_from_value(base_url)?);
        }
        if let Some(headers) = self.headers {
            let merged = headers_of(headers)?
                .merge_with(options.headers())
                .map_err(storage_error)?;
            options = options.with_headers(merged);
        }
        if let Some(auth) = self.auth {
            options = options.with_authorization(authorization_of(auth)?);
        }
        if let Some(timeout) = self.timeout {
            options = options.with_timeout(duration_of(timeout, "timeout")?);
        }
        Ok(options)
    }

    /// A session with these settings over `client`, or over a client built
    /// for their transport.
    fn session(&self, client: Option<&Client>) -> PyResult<Session> {
        let options = self.resolve()?;
        match client {
            Some(client) => Session::with_client(client.clone(), options),
            None => Session::with_options(options),
        }
        .map_err(storage_error)
    }
}

/// The parts of one request a requests-shaped call names beside its URL.
#[derive(Default)]
struct Parts<'a, 'py> {
    params: Option<&'a Bound<'py, PyAny>>,
    headers: Option<&'a Bound<'py, PyAny>>,
    data: Option<&'a Bound<'py, PyAny>>,
    json: Option<&'a Bound<'py, PyAny>>,
    auth: Option<&'a Bound<'py, PyAny>>,
    timeout: Option<&'a Bound<'py, PyAny>>,
    allow_redirects: Option<bool>,
    pagination: Option<&'a str>,
    records: Option<&'a str>,
}

impl Parts<'_, '_> {
    /// Apply every part to `request`, each through its core setter.
    fn apply(&self, mut request: Request) -> PyResult<Request> {
        if let Some(params) = self.params {
            request = request
                .with_query(pairs_of(params, "params")?)
                .map_err(storage_error)?;
        }
        if let Some(headers) = self.headers {
            request = request.with_headers(headers_of(headers)?);
        }
        match (self.data, self.json) {
            (Some(_), Some(_)) => {
                return Err(PyValueError::new_err(
                    "expected one body, got both data= and json=; pass the document as json= \
                     or its bytes as data=",
                ));
            }
            (Some(data), None) => {
                request = match body_bytes(data)? {
                    Some(bytes) => request.with_body(bytes),
                    None => request.with_form(pairs_of(data, "data")?),
                };
            }
            (None, Some(json)) => {
                request = request.with_json(&from_py(json)?).map_err(storage_error)?;
            }
            (None, None) => {}
        }
        if let Some(auth) = self.auth {
            request = request.with_authorization(authorization_of(auth)?);
        }
        if let Some(timeout) = self.timeout {
            request = request.with_timeout(duration_of(timeout, "timeout")?);
        }
        if let Some(follow) = self.allow_redirects {
            request = request.with_follow_redirects(follow);
        }
        if let Some(pagination) = self.pagination {
            request =
                request.with_pagination(Pagination::from_str(pagination).map_err(storage_error)?);
        }
        if let Some(records) = self.records {
            request = request.with_records(FieldPath::from_str(records).map_err(storage_error)?);
        }
        Ok(request)
    }
}

/// Build the request `method` at `url` on `session` with `parts` applied.
fn prepared(
    session: &Session,
    method: &str,
    url: &Bound<'_, PyAny>,
    parts: &Parts<'_, '_>,
) -> PyResult<Request> {
    let method = Method::from_str(method).map_err(storage_error)?;
    let request = session
        .request(method, &url_text(url)?)
        .map_err(storage_error)?;
    parts.apply(request)
}

/// The keywords a request spec of `Session.send_all` may name.
const SPEC_KEYS: [&str; 9] = [
    "method",
    "url",
    "params",
    "headers",
    "data",
    "json",
    "auth",
    "timeout",
    "allow_redirects",
];

/// One item of a `send_all` source as a request on `session`: a prepared
/// `Request` as it is, a URL for a `GET`, or a mapping of
/// `Session.request`'s keywords - `method` (`GET` unless named), `url`,
/// `params`, `headers`, `data`, `json`, `auth`, `timeout`,
/// `allow_redirects`. An unknown keyword is refused by name.
fn request_spec(session: &Session, value: &Bound<'_, PyAny>) -> PyResult<Request> {
    if value.is_instance_of::<PyIOBase>() {
        return request_from(value);
    }
    let Ok(spec) = value.cast::<PyMapping>() else {
        return prepared(session, "GET", value, &Parts::default());
    };
    for key in spec.keys()?.iter() {
        let key: String = key.extract()?;
        if !SPEC_KEYS.contains(&key.as_str()) {
            return Err(PyTypeError::new_err(format!(
                "unexpected request keyword {key:?}; expected one of {}",
                SPEC_KEYS.join(", ")
            )));
        }
    }
    let item = |name: &str| -> PyResult<Option<Bound<'_, PyAny>>> {
        if spec.contains(name)? {
            let value = spec.get_item(name)?;
            Ok((!value.is_none()).then_some(value))
        } else {
            Ok(None)
        }
    };
    let url = item("url")?.ok_or_else(|| PyTypeError::new_err("a request spec names its url"))?;
    let method: String = match item("method")? {
        Some(method) => method.extract()?,
        None => "GET".to_owned(),
    };
    let (params, headers, data, json, auth, timeout) = (
        item("params")?,
        item("headers")?,
        item("data")?,
        item("json")?,
        item("auth")?,
        item("timeout")?,
    );
    let allow_redirects = item("allow_redirects")?
        .map(|follow| follow.extract::<bool>())
        .transpose()?;
    let parts = Parts {
        params: params.as_ref(),
        headers: headers.as_ref(),
        data: data.as_ref(),
        json: json.as_ref(),
        auth: auth.as_ref(),
        timeout: timeout.as_ref(),
        allow_redirects,
        ..Parts::default()
    };
    prepared(session, &method, &url, &parts)
}

/// The requests of a `send_all` walk, pulled from a Python iterable only as
/// the workers take them: each pull takes the interpreter for its one item.
/// An item that raises, or that no request reads from, ends the source and
/// is raised where the walk reaches it.
struct Source {
    iterator: Py<PyIterator>,
    session: Session,
    failure: Arc<Mutex<Option<PyErr>>>,
}

impl Iterator for Source {
    type Item = Request;

    fn next(&mut self) -> Option<Request> {
        Python::attach(|py| {
            let answer = match self.iterator.bind(py).clone().next()? {
                Ok(item) => request_spec(&self.session, &item),
                Err(error) => Err(error),
            };
            match answer {
                Ok(request) => Some(request),
                Err(error) => {
                    if let Ok(mut failure) = self.failure.lock() {
                        *failure = Some(error);
                    }
                    None
                }
            }
        })
    }
}

/// Send `request` with the interpreter released and answer the `Response`.
fn sent(py: Python<'_>, request: &Request, stream: bool) -> PyResult<Py<PyAny>> {
    let response = py
        .detach(|| {
            if stream {
                request.stream()
            } else {
                request.send()
            }
        })
        .map_err(storage_error)?;
    describe(py, Holder::HttpResponse(response))
}

/// The counters of a client as a mapping of name to count.
fn stats_dict(py: Python<'_>, stats: StatsSnapshot) -> PyResult<Py<PyDict>> {
    let counts = PyDict::new(py);
    counts.set_item("requests", stats.requests)?;
    counts.set_item("gets", stats.gets)?;
    counts.set_item("heads", stats.heads)?;
    counts.set_item("posts", stats.posts)?;
    counts.set_item("puts", stats.puts)?;
    counts.set_item("patches", stats.patches)?;
    counts.set_item("deletes", stats.deletes)?;
    counts.set_item("others", stats.others)?;
    counts.set_item("retries", stats.retries)?;
    counts.set_item("resumes", stats.resumes)?;
    counts.set_item("redirects", stats.redirects)?;
    counts.set_item("retry_tokens", stats.retry_tokens)?;
    Ok(counts.unbind())
}

/// Parsed `Link` values keyed by each relation they carry, as `requests`
/// lays them out: `{"next": {"url": ..., "rel": "next", ...}}`.
fn links_dict(py: Python<'_>, links: &[Link]) -> PyResult<Py<PyDict>> {
    let keyed = PyDict::new(py);
    for link in links {
        let relations: Vec<&str> = if link.rel.is_empty() {
            vec![link.target.as_str()]
        } else {
            link.rel.iter().map(std::ops::Deref::deref).collect()
        };
        for relation in relations {
            let entry = PyDict::new(py);
            for (name, value) in &link.parameters {
                entry.set_item(name.as_str(), value)?;
            }
            entry.set_item("url", &link.target)?;
            entry.set_item("rel", relation)?;
            keyed.set_item(relation, entry)?;
        }
    }
    Ok(keyed.unbind())
}

/// UTC nanoseconds since the epoch, now: the clock the relative header
/// readers measure against.
fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
        .unwrap_or(0)
}

/// An instant in UTC nanoseconds as the `SystemTime` Python reads as a
/// `datetime`.
fn instant(ns: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(ns.unsigned_abs());
    if ns >= 0 {
        UNIX_EPOCH + magnitude
    } else {
        UNIX_EPOCH - magnitude
    }
}

/// Build one response a handler or a caller states: `status`, a body as
/// bytes or text (text is served as `text/plain; charset=utf-8`), or a JSON
/// document, and the headers given over whatever the body declared.
fn built(
    status: u16,
    headers: Option<&Bound<'_, PyAny>>,
    body: Option<&Bound<'_, PyAny>>,
    json: Option<&Bound<'_, PyAny>>,
) -> PyResult<Response> {
    let mut response = Response::new(Status::new(status).map_err(storage_error)?);
    match (body, json) {
        (Some(body), Some(_)) if !body.is_none() => {
            return Err(PyValueError::new_err(
                "expected one body, got both body= and json=",
            ));
        }
        (_, Some(json)) => {
            response = response.with_json(&from_py(json)?).map_err(storage_error)?;
        }
        (Some(body), None) if !body.is_none() => {
            if let Ok(text) = body.cast::<PyString>() {
                response = response.with_text(text.to_str()?);
            } else {
                let bytes = body_bytes(body)?.ok_or_else(|| {
                    PyTypeError::new_err(format!(
                        "expected the body as bytes or str, got {}",
                        type_name(body)
                    ))
                })?;
                response = response.with_body(bytes);
            }
        }
        _ => {}
    }
    if let Some(headers) = headers
        && !headers.is_none()
    {
        response = response
            .with_headers(headers_of(headers)?)
            .map_err(storage_error)?;
    }
    Ok(response)
}

/// A session: default headers, a credential, a cookie jar and the options
/// every request starts from, over a pooled client.
///
/// As a handle it is a container over its base URL: `session / "users"` is
/// the `Request` for that resource, which reads and writes its bytes and
/// records. Nothing is sent until a verb asks.
#[pyclass(name = "Session", module = "yggdryl._native", extends = PyIOBase, skip_from_py_object)]
pub(crate) struct PySession;

#[pymethods]
#[allow(clippy::too_many_arguments)] // The requests-shaped keywords, each optional.
impl PySession {
    /// A session with the given defaults: a relative URL joins onto
    /// `base_url`, `headers` go under every request's own, `auth` is a
    /// `(user, password)` pair or a bearer token, `timeout` seconds or a
    /// `timedelta`, and `options` any `HttpOptions` property by name.
    #[new]
    #[pyo3(signature = (base_url = None, *, headers = None, auth = None, timeout = None, options = None))]
    fn new(
        base_url: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let session = Settings {
            base_url,
            headers,
            auth,
            timeout,
            options,
        }
        .session(None)?;
        Ok(
            PyClassInitializer::from(PyIOBase::from_core(Holder::HttpSession(session)))
                .add_subclass(Self),
        )
    }

    /// Send `method` to `url` and answer the `Response`.
    ///
    /// `stream=True` leaves the body on the wire, read by `iter_content` or
    /// as the handle's bytes; otherwise it is read whole, bounded by the
    /// session's `max_body_size`. A refusing status is not an error: read
    /// it off the response or call `raise_for_status`.
    #[pyo3(signature = (method, url, *, params = None, headers = None, data = None, json = None, auth = None, timeout = None, allow_redirects = None, stream = false))]
    fn request(
        slf: &Bound<'_, Self>,
        method: &str,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        data: Option<&Bound<'_, PyAny>>,
        json: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        allow_redirects: Option<bool>,
        stream: bool,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            data,
            json,
            auth,
            timeout,
            allow_redirects,
            ..Parts::default()
        };
        Self::call(slf, method, url, &parts, stream)
    }

    /// `GET url`.
    #[pyo3(signature = (url, *, params = None, headers = None, auth = None, timeout = None, allow_redirects = None, stream = false))]
    fn get(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        allow_redirects: Option<bool>,
        stream: bool,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            auth,
            timeout,
            allow_redirects,
            ..Parts::default()
        };
        Self::call(slf, "GET", url, &parts, stream)
    }

    /// `HEAD url`.
    #[pyo3(signature = (url, *, params = None, headers = None, auth = None, timeout = None, allow_redirects = None))]
    fn head(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        allow_redirects: Option<bool>,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            auth,
            timeout,
            allow_redirects,
            ..Parts::default()
        };
        Self::call(slf, "HEAD", url, &parts, false)
    }

    /// `DELETE url`.
    #[pyo3(signature = (url, *, params = None, headers = None, auth = None, timeout = None))]
    fn delete(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            auth,
            timeout,
            ..Parts::default()
        };
        Self::call(slf, "DELETE", url, &parts, false)
    }

    /// `POST` a body to `url`: `data` as bytes, text, or form pairs, or
    /// `json` as a document.
    #[pyo3(signature = (url, data = None, *, json = None, params = None, headers = None, auth = None, timeout = None))]
    fn post(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        data: Option<&Bound<'_, PyAny>>,
        json: Option<&Bound<'_, PyAny>>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            data,
            json,
            auth,
            timeout,
            ..Parts::default()
        };
        Self::call(slf, "POST", url, &parts, false)
    }

    /// `PUT` a body to `url`, spelled as `post` spells it.
    #[pyo3(signature = (url, data = None, *, json = None, params = None, headers = None, auth = None, timeout = None))]
    fn put(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        data: Option<&Bound<'_, PyAny>>,
        json: Option<&Bound<'_, PyAny>>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            data,
            json,
            auth,
            timeout,
            ..Parts::default()
        };
        Self::call(slf, "PUT", url, &parts, false)
    }

    /// `PATCH` a body to `url`, spelled as `post` spells it.
    #[pyo3(signature = (url, data = None, *, json = None, params = None, headers = None, auth = None, timeout = None))]
    fn patch(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        data: Option<&Bound<'_, PyAny>>,
        json: Option<&Bound<'_, PyAny>>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            data,
            json,
            auth,
            timeout,
            ..Parts::default()
        };
        Self::call(slf, "PATCH", url, &parts, false)
    }

    /// `GET url` with the body left on the wire as a resumable stream.
    #[pyo3(signature = (url, *, params = None, headers = None, auth = None, timeout = None))]
    fn stream(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let parts = Parts {
            params,
            headers,
            auth,
            timeout,
            ..Parts::default()
        };
        Self::call(slf, "GET", url, &parts, true)
    }

    /// Send a prepared `Request` on this session.
    #[pyo3(signature = (request, *, stream = false))]
    fn send(
        slf: &Bound<'_, Self>,
        request: &Bound<'_, PyAny>,
        stream: bool,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let session = session_of(slf.borrow().as_super())?;
        let request = request_from(request)?;
        let response = py
            .detach(|| {
                if stream {
                    session.stream(&request)
                } else {
                    session.send(&request)
                }
            })
            .map_err(storage_error)?;
        describe(py, Holder::HttpResponse(response))
    }

    /// Send every request of `requests` on up to `concurrency` threads -
    /// the session's own `concurrency` unless given, `0` read as one - and
    /// iterate the responses in the order the requests were given, each as
    /// soon as the ones before it answered.
    ///
    /// An item is a prepared `Request`, a URL for a `GET`, or a mapping of
    /// `request`'s keywords (`method`, `url`, `params`, `headers`, `data`,
    /// `json`, `auth`, `timeout`, `allow_redirects`). The walk is a stream:
    /// `requests` - a list, a generator, an endless iterator - is pulled
    /// only as far as the requests in flight, and the interpreter is
    /// released while an answer is awaited. A failed request raises at its
    /// own place and iterating on reads the next answer; an item that is no
    /// request raises where the walk reaches it and ends the walk.
    #[pyo3(signature = (requests, concurrency = None))]
    fn send_all(
        slf: &Bound<'_, Self>,
        requests: &Bound<'_, PyAny>,
        concurrency: Option<usize>,
    ) -> PyResult<PyResponses> {
        let session = session_of(slf.borrow().as_super())?;
        let failure = Arc::new(Mutex::new(None));
        let source = Source {
            iterator: requests.try_iter()?.unbind(),
            session: session.clone(),
            failure: Arc::clone(&failure),
        };
        Ok(PyResponses {
            answers: Mutex::new(Box::new(session.send_all(source, concurrency))),
            failure,
        })
    }

    /// Walk the pages of `GET url` per the pagination - `"auto"` unless
    /// `pagination` names one - one `Response` per page.
    #[pyo3(signature = (url, *, params = None, headers = None, auth = None, timeout = None, pagination = None, records = None))]
    fn pages(
        slf: &Bound<'_, Self>,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        pagination: Option<&str>,
        records: Option<&str>,
    ) -> PyResult<PyPages> {
        let session = session_of(slf.borrow().as_super())?;
        let parts = Parts {
            params,
            headers,
            auth,
            timeout,
            pagination,
            records,
            ..Parts::default()
        };
        let request = prepared(&session, "GET", url, &parts)?;
        Ok(PyPages::from_core(session.pages(request)))
    }

    /// The headers every request carries unless it sets its own.
    #[getter]
    fn headers(slf: &Bound<'_, Self>) -> PyResult<PyHeaders> {
        Ok(PyHeaders::from_core(
            session_of(slf.borrow().as_super())?
                .options()
                .headers()
                .clone(),
        ))
    }

    /// The whole-request timeout.
    #[getter]
    fn timeout(slf: &Bound<'_, Self>) -> PyResult<Duration> {
        Ok(session_of(slf.borrow().as_super())?.options().timeout())
    }

    /// How many requests `send_all` sends side by side.
    #[getter]
    fn concurrency(slf: &Bound<'_, Self>) -> PyResult<usize> {
        Ok(session_of(slf.borrow().as_super())?.options().concurrency())
    }

    /// The client's request counters.
    #[getter]
    fn stats(slf: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        stats_dict(slf.py(), session_of(slf.borrow().as_super())?.stats())
    }

    /// The cookies the jar holds, name to value.
    #[getter]
    fn cookies(slf: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        let py = slf.py();
        let jar = PyDict::new(py);
        for cookie in session_of(slf.borrow().as_super())?.cookies() {
            jar.set_item(cookie.name, cookie.value)?;
        }
        Ok(jar.unbind())
    }

    /// Store a cookie covering `domain` and every path below it.
    fn set_cookie(slf: &Bound<'_, Self>, name: &str, value: &str, domain: &str) -> PyResult<()> {
        session_of(slf.borrow().as_super())?
            .set_cookie(yggdryl::http::Cookie::new(name, value, domain));
        Ok(())
    }
}

impl PySession {
    /// Build the request and send it, the interpreter released while it is
    /// on the wire.
    fn call(
        slf: &Bound<'_, Self>,
        method: &str,
        url: &Bound<'_, PyAny>,
        parts: &Parts<'_, '_>,
        stream: bool,
    ) -> PyResult<Py<PyAny>> {
        let session = session_of(slf.borrow().as_super())?;
        let request = prepared(&session, method, url, parts)?;
        sent(slf.py(), &request, stream)
    }
}

/// The process-wide default session every module-level verb sends on.
#[pyfunction]
pub(crate) fn http_session(py: Python<'_>) -> PyResult<Py<PyAny>> {
    describe(py, Holder::HttpSession(yggdryl::http::session()))
}

/// One request, prepared and not yet sent: the resource its URL names.
///
/// As a handle it is that resource: `read_bytes` is one `GET`,
/// `read_range_bytes` one ranged `GET`, `write_bytes` one `PUT`, `unlink`
/// one `DELETE`, and a JSON, Parquet or paginated body reads as records.
#[pyclass(name = "Request", module = "yggdryl._native", extends = PyIOBase, skip_from_py_object)]
pub(crate) struct PyRequest;

#[pymethods]
#[allow(clippy::too_many_arguments)] // The requests-shaped keywords, each optional.
impl PyRequest {
    /// Prepare `method` at `url` on `session` - the process-wide default
    /// session when none is given - with the parts named.
    #[new]
    #[pyo3(signature = (method, url, *, params = None, headers = None, data = None, json = None, auth = None, timeout = None, allow_redirects = None, session = None))]
    fn new(
        method: &str,
        url: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        data: Option<&Bound<'_, PyAny>>,
        json: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        allow_redirects: Option<bool>,
        session: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let session = match session {
            Some(session) => session_from(session)?,
            None => yggdryl::http::session(),
        };
        let parts = Parts {
            params,
            headers,
            data,
            json,
            auth,
            timeout,
            allow_redirects,
            ..Parts::default()
        };
        let request = prepared(&session, method, url, &parts)?;
        Ok(
            PyClassInitializer::from(PyIOBase::from_core(Holder::HttpRequest(request)))
                .add_subclass(Self),
        )
    }

    /// The method, upper case.
    #[getter]
    fn method(slf: &Bound<'_, Self>) -> PyResult<&'static str> {
        Ok(request_of(slf.borrow().as_super())?.method().as_str())
    }

    /// The request's own headers; the session's defaults join them on send.
    #[getter]
    fn headers(slf: &Bound<'_, Self>) -> PyResult<PyHeaders> {
        Ok(PyHeaders::from_core(
            request_of(slf.borrow().as_super())?.headers().clone(),
        ))
    }

    /// The body, as it will be sent.
    #[getter]
    fn body<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyBytes>> {
        let request = request_of(slf.borrow().as_super())?;
        Ok(PyBytes::new(slf.py(), request.body().as_bytes()))
    }

    /// The session this request sends on.
    #[getter]
    fn session(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let session = request_of(slf.borrow().as_super())?.session().clone();
        describe(slf.py(), Holder::HttpSession(session))
    }

    /// Send, reading the whole body.
    fn send(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let request = request_of(slf.borrow().as_super())?;
        sent(slf.py(), &request, false)
    }

    /// Send, leaving the body on the wire as a resumable stream.
    fn stream(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let request = request_of(slf.borrow().as_super())?;
        sent(slf.py(), &request, true)
    }

    /// Walk the pages of this request per its pagination.
    #[pyo3(signature = (*, pagination = None, records = None))]
    fn pages(
        slf: &Bound<'_, Self>,
        pagination: Option<&str>,
        records: Option<&str>,
    ) -> PyResult<PyPages> {
        let parts = Parts {
            pagination,
            records,
            ..Parts::default()
        };
        let request = parts.apply(request_of(slf.borrow().as_super())?)?;
        Ok(PyPages::from_core(request.pages()))
    }
}

/// One answer: the status, the headers, and the body - held whole, or on
/// the wire when the request streamed.
///
/// As a handle it is the body as sent - coded, under a media type keeping
/// the coding - so `into_media()` composes the coding and the record
/// encoding; `content`, `text` and `json()` are the decoded views.
#[pyclass(name = "Response", module = "yggdryl._native", extends = PyIOBase, skip_from_py_object)]
pub(crate) struct PyResponse;

impl PyResponse {
    /// Run `work` on the held response with the interpreter released.
    fn detached<T: Send>(
        slf: &Bound<'_, Self>,
        work: impl FnOnce(&Response) -> yggdryl::Result<T> + Send,
    ) -> PyResult<T> {
        let this = slf.borrow();
        let response = response_of(this.as_super())?;
        slf.py().detach(|| work(response)).map_err(storage_error)
    }

    /// Read something the response already holds, without the network.
    fn read<T>(slf: &Bound<'_, Self>, read: impl FnOnce(&Response) -> T) -> PyResult<T> {
        let this = slf.borrow();
        Ok(read(response_of(this.as_super())?))
    }
}

#[pymethods]
impl PyResponse {
    /// A response stated by hand, as a route handler answers: `status`,
    /// `headers`, and a body as bytes, text, or a `json` document.
    #[new]
    #[pyo3(signature = (status = 200, headers = None, body = None, *, json = None))]
    fn new(
        status: u16,
        headers: Option<&Bound<'_, PyAny>>,
        body: Option<&Bound<'_, PyAny>>,
        json: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let response = built(status, headers, body, json)?;
        Ok(
            PyClassInitializer::from(PyIOBase::from_core(Holder::HttpResponse(response)))
                .add_subclass(Self),
        )
    }

    /// The status code.
    #[getter]
    fn status_code(slf: &Bound<'_, Self>) -> PyResult<u16> {
        Self::read(slf, |response| response.status().code())
    }

    /// The status's reason phrase.
    #[getter]
    fn reason(slf: &Bound<'_, Self>) -> PyResult<&'static str> {
        Self::read(slf, |response| response.status().reason())
    }

    /// Whether the status is below 400.
    #[getter]
    fn ok(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Self::read(slf, Response::is_ok)
    }

    /// Whether the status is a redirect.
    #[getter]
    fn is_redirect(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Self::read(slf, Response::is_redirect)
    }

    /// The HTTP version the answer stated, such as `HTTP/1.1`.
    #[getter]
    fn version(slf: &Bound<'_, Self>) -> PyResult<&'static str> {
        Self::read(slf, |response| response.version().as_str())
    }

    /// The headers, looked up ignoring case.
    #[getter]
    fn headers(slf: &Bound<'_, Self>) -> PyResult<PyHeaders> {
        Self::read(slf, |response| {
            PyHeaders::from_core(response.headers().clone())
        })
    }

    /// The request this answers.
    #[getter]
    fn request(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let request = Self::read(slf, |response| response.request().clone())?;
        describe(slf.py(), Holder::HttpRequest(request))
    }

    /// The redirect hops before this answer, oldest first, as
    /// `(status_code, url)` pairs.
    #[getter]
    fn history(slf: &Bound<'_, Self>) -> PyResult<Vec<(u16, Py<crate::uri::PyUrl>)>> {
        let py = slf.py();
        let hops = Self::read(slf, |response| {
            response
                .history()
                .iter()
                .map(|hop| (hop.status().code(), hop.url().clone()))
                .collect::<Vec<_>>()
        })?;
        hops.into_iter()
            .map(|(code, url)| Ok((code, url_object(py, url)?)))
            .collect()
    }

    /// How long the exchange took, redirects and retries included.
    #[getter]
    fn elapsed(slf: &Bound<'_, Self>) -> PyResult<Duration> {
        Self::read(slf, Response::elapsed)
    }

    /// The whole decoded body, read once and held.
    #[getter]
    fn content<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = Self::detached(slf, Response::bytes)?;
        Ok(PyBytes::new(slf.py(), &bytes))
    }

    /// The decoded body as text, in the charset `Content-Type` declares,
    /// UTF-8 when it declares none.
    #[getter]
    fn text(slf: &Bound<'_, Self>) -> PyResult<String> {
        Self::detached(slf, Response::text)
    }

    /// The charset `Content-Type` declares, or `None`.
    #[getter]
    fn encoding(slf: &Bound<'_, Self>) -> PyResult<Option<&'static str>> {
        Self::read(slf, |response| response.encoding().map(Charset::as_str))
    }

    /// Every `Set-Cookie` the answer carried, name to value.
    #[getter]
    fn cookies(slf: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        let py = slf.py();
        let jar = PyDict::new(py);
        for cookie in Self::read(slf, Response::cookies)? {
            jar.set_item(cookie.name, cookie.value)?;
        }
        Ok(jar.unbind())
    }

    /// The `Link` header, keyed by relation.
    #[getter]
    fn links(slf: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        let links = Self::read(slf, Response::links)?.map_err(storage_error)?;
        links_dict(slf.py(), &links)
    }

    /// The request for the next page per the request's pagination - the
    /// same request at the next URL - or `None` on the last.
    #[getter]
    fn next(slf: &Bound<'_, Self>) -> PyResult<Option<Py<PyAny>>> {
        let py = slf.py();
        let following = Self::detached(slf, Response::next_request)?;
        following
            .map(|request| describe(py, Holder::HttpRequest(request)))
            .transpose()
    }

    /// The body parsed under its media type (JSON, JSON Lines, YAML, TOML
    /// or XML), as native Python values.
    fn json(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let document = Self::detached(slf, Response::scalar)?;
        as_py(slf.py(), &document)
    }

    /// The body parsed under its media type as a `Scalar`, read under
    /// `field` when one is given.
    #[pyo3(signature = (field = None))]
    fn scalar(slf: &Bound<'_, Self>, field: Option<&Bound<'_, PyAny>>) -> PyResult<PyScalar> {
        let field = field.map(crate::field::core_field_from_value).transpose()?;
        let document = Self::detached(slf, |response| match &field {
            Some(field) => response.scalar_with_field(field),
            None => response.scalar(),
        })?;
        Ok(PyScalar::from_inner(document))
    }

    /// This response, or the refusal a status of 400 or more is.
    fn raise_for_status(slf: &Bound<'_, Self>) -> PyResult<Py<Self>> {
        Self::detached(slf, |response| response.raise_for_status().map(drop))?;
        Ok(slf.clone().unbind())
    }

    /// The decoded body in chunks of `chunk_size` bytes - the session's
    /// `stream_batch_size` unless given, at most 64 MiB - read off the wire
    /// as they arrive when the request streamed and the body is uncoded.
    #[pyo3(signature = (chunk_size = None))]
    fn iter_content(
        slf: &Bound<'_, Self>,
        chunk_size: Option<usize>,
    ) -> PyResult<PyContentIterator> {
        Ok(PyContentIterator {
            chunks: Chunks::new(slf, chunk_size)?,
        })
    }

    /// The decoded body one line at a time, the line break taken off.
    #[pyo3(signature = (chunk_size = None))]
    fn iter_lines(slf: &Bound<'_, Self>, chunk_size: Option<usize>) -> PyResult<PyLineIterator> {
        Ok(PyLineIterator {
            chunks: Chunks::new(slf, chunk_size)?,
            pending: Vec::new(),
        })
    }

    /// The body as a resumable byte stream, spending this handle.
    fn into_stream(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let base = slf.as_super();
        if !matches!(base.inner()?, Holder::HttpResponse(_)) {
            return Err(no_longer("response"));
        }
        let Holder::HttpResponse(response) = base.take()? else {
            return Err(no_longer("response"));
        };
        let stream = py
            .detach(|| response.into_stream())
            .map_err(storage_error)?;
        describe(py, Holder::HttpStream(stream))
    }
}

/// The byte walk behind `iter_content` and `iter_lines`: a held body is
/// sliced out of its decoded bytes, a streaming one is read forward off the
/// wire, the interpreter released while a read waits.
struct Chunks {
    response: Py<PyResponse>,
    chunk_size: usize,
    position: u64,
    held: Option<Arc<[u8]>>,
    /// The room a streamed chunk is read into, kept across chunks so each
    /// costs no fresh zeroed allocation.
    scratch: Vec<u8>,
    done: bool,
}

impl Chunks {
    fn new(slf: &Bound<'_, PyResponse>, chunk_size: Option<usize>) -> PyResult<Self> {
        let chunk_size = match chunk_size {
            Some(0) => {
                return Err(PyValueError::new_err(
                    "expected a positive chunk_size, got 0; pass None for the session's \
                     stream_batch_size",
                ));
            }
            Some(size) => size.min(MAX_CHUNK_SIZE),
            None => PyResponse::read(slf, |response| {
                response.request().session().options().stream_batch_size()
            })?,
        };
        Ok(Self {
            response: slf.clone().unbind(),
            chunk_size,
            position: 0,
            held: None,
            scratch: Vec::new(),
            done: false,
        })
    }

    /// The next chunk as Python bytes: a streamed one read straight into the
    /// bytes object, the interpreter released while it fills, so a chunk is
    /// copied once out of the transport and never again.
    fn pull_bytes<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        if self.done {
            return Ok(None);
        }
        let handle = self.response.bind(py).borrow();
        let response = response_of(handle.as_super())?;
        if !reads_off_the_wire(response) {
            drop(handle);
            return Ok(self.pull(py)?.map(|chunk| PyBytes::new(py, &chunk)));
        }
        let position = self.position;
        let mut filled = Ok(0);
        let chunk = PyBytes::new_with(py, self.chunk_size, |buffer| {
            filled = py.detach(|| fill_from(response, position, buffer));
            Ok(())
        })?;
        let filled = filled.map_err(storage_error)?;
        if filled == 0 {
            self.done = true;
            return Ok(None);
        }
        self.position += filled as u64;
        Ok(Some(if filled == self.chunk_size {
            chunk
        } else {
            // The last chunk of the body, shorter than the rest.
            PyBytes::new(py, &chunk.as_bytes()[..filled])
        }))
    }

    /// The next chunk, or `None` once the body is spent.
    fn pull(&mut self, py: Python<'_>) -> PyResult<Option<Vec<u8>>> {
        if self.done {
            return Ok(None);
        }
        let handle = self.response.bind(py).borrow();
        let response = response_of(handle.as_super())?;
        let chunk = if reads_off_the_wire(response) {
            // A whole chunk, as `requests` yields one: reads repeat until it
            // is full or the body ends, since one read answers what the
            // transport holds at that moment.
            self.scratch.resize(self.chunk_size, 0);
            let (position, buffer) = (self.position, &mut self.scratch);
            let filled = py
                .detach(|| fill_from(response, position, buffer))
                .map_err(storage_error)?;
            self.scratch[..filled].to_vec()
        } else {
            let bytes = if let Some(bytes) = &self.held {
                Arc::clone(bytes)
            } else {
                let bytes = py.detach(|| response.bytes()).map_err(storage_error)?;
                self.held = Some(Arc::clone(&bytes));
                bytes
            };
            let start = usize::try_from(self.position)
                .unwrap_or(usize::MAX)
                .min(bytes.len());
            let end = start.saturating_add(self.chunk_size).min(bytes.len());
            bytes[start..end].to_vec()
        };
        if chunk.is_empty() {
            self.done = true;
            self.held = None;
            return Ok(None);
        }
        self.position += chunk.len() as u64;
        Ok(Some(chunk))
    }
}

/// A copy of `holder`'s bytes under its media type, for a server to mount.
fn copied(holder: &Holder) -> PyResult<Holder> {
    let mut buffer = Buffer::from_bytes(holder.read_all_bytes().map_err(storage_error)?);
    buffer.set_media_type(holder.media_type().clone());
    Ok(Holder::Buffer(buffer))
}

/// The most one chunk of `iter_content` or `iter_lines` holds, whatever
/// `chunk_size` asks: a chunk is allocated whole before it is filled.
const MAX_CHUNK_SIZE: usize = 64 << 20;

/// Whether a body's chunks are read straight off the wire: a streaming body
/// carrying no content coding. A coded one - a server that coded what was
/// asked for as `identity` - is decoded whole first, so a chunk is always
/// the body's own bytes, as `requests` yields them.
fn reads_off_the_wire(response: &Response) -> bool {
    response.kind() == IOKind::File
        && response
            .headers()
            .content_encoding()
            .is_ok_and(|codings| codings.iter().all(|coding| coding.is_identity()))
}

/// Fill `buffer` from `response` at `position`: reads repeat until it is full
/// or the body ends, since one read answers what the transport holds at that
/// moment. Answers how many bytes landed.
fn fill_from(response: &Response, position: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = response.pread(position + filled as u64, &mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// The chunks of one response body.
#[pyclass(name = "_ContentIterator", module = "yggdryl._native")]
pub(crate) struct PyContentIterator {
    chunks: Chunks,
}

#[pymethods]
impl PyContentIterator {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        self.chunks.pull_bytes(py)
    }
}

/// The lines of one response body.
#[pyclass(name = "_LineIterator", module = "yggdryl._native")]
pub(crate) struct PyLineIterator {
    chunks: Chunks,
    pending: Vec<u8>,
}

#[pymethods]
impl PyLineIterator {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        loop {
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.pending.drain(..=end).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(PyBytes::new(py, &line)));
            }
            match self.chunks.pull(py)? {
                Some(chunk) => self.pending.extend_from_slice(&chunk),
                None if self.pending.is_empty() => return Ok(None),
                None => {
                    let line = std::mem::take(&mut self.pending);
                    return Ok(Some(PyBytes::new(py, &line)));
                }
            }
        }
    }
}

/// The answers of `Session.send_all`, in request order.
#[pyclass(name = "_ResponseIterator", module = "yggdryl._native")]
pub(crate) struct PyResponses {
    answers: Mutex<Box<dyn Iterator<Item = yggdryl::Result<Response>> + Send>>,
    /// What stopped the source, raised once the answers before it are out.
    failure: Arc<Mutex<Option<PyErr>>>,
}

impl Drop for PyResponses {
    fn drop(&mut self) {
        // Dropping the walk joins its workers, each finishing the request it
        // holds; a worker logging through the interpreter needs it, and every
        // other thread should not wait on the network, so they are joined
        // with the interpreter released.
        let answers = std::mem::replace(
            self.answers
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Box::new(std::iter::empty()),
        );
        Python::attach(|py| py.detach(move || drop(answers)));
    }
}

#[pymethods]
impl PyResponses {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let next = py.detach(|| {
            self.answers
                .lock()
                .map(|mut answers| answers.next())
                .map_err(|_| poisoned("send_all"))
        })?;
        match next {
            None => match self
                .failure
                .lock()
                .map_err(|_| poisoned("send_all"))?
                .take()
            {
                Some(error) => Err(error),
                None => Ok(None),
            },
            Some(answer) => {
                describe(py, Holder::HttpResponse(answer.map_err(storage_error)?)).map(Some)
            }
        }
    }
}

/// A body left on the wire: read forward, resumed from the delivered byte
/// with a `Range` when the transfer is cut and the resource allows it.
#[pyclass(name = "Stream", module = "yggdryl._native", extends = PyIOBase, skip_from_py_object)]
pub(crate) struct PyStream;

#[pymethods]
impl PyStream {
    /// Bytes handed to the caller so far: the resume cursor.
    #[getter]
    fn delivered(slf: &Bound<'_, Self>) -> PyResult<u64> {
        Ok(stream_of(slf.borrow().as_super())?.delivered())
    }

    /// The length the headers stated, or `None`.
    #[getter]
    fn total(slf: &Bound<'_, Self>) -> PyResult<Option<u64>> {
        Ok(stream_of(slf.borrow().as_super())?.total())
    }

    /// How many times the transfer was re-opened.
    #[getter]
    fn resumes(slf: &Bound<'_, Self>) -> PyResult<u32> {
        Ok(stream_of(slf.borrow().as_super())?.resumes())
    }

    /// The headers of the answer the stream reads.
    #[getter]
    fn headers(slf: &Bound<'_, Self>) -> PyResult<PyHeaders> {
        Ok(PyHeaders::from_core(
            stream_of(slf.borrow().as_super())?.headers().clone(),
        ))
    }

    /// Read up to `size` bytes forward, or the rest when `size` is negative;
    /// empty at the end.
    #[pyo3(signature = (size = -1))]
    fn read<'py>(slf: &Bound<'py, Self>, size: i64) -> PyResult<Bound<'py, PyBytes>> {
        let py = slf.py();
        let this = slf.borrow();
        let stream = stream_of(this.as_super())?;
        let bytes = py
            .detach(|| -> yggdryl::Result<Vec<u8>> {
                let position = stream.delivered();
                if let Ok(size) = usize::try_from(size) {
                    // One read answers what the transport holds, so a room
                    // larger than a batch would only be zeroed and not filled.
                    let mut buffer = vec![0_u8; size.min(yggdryl::DEFAULT_STREAM_BATCH_SIZE)];
                    let read = stream.pread(position, &mut buffer)?;
                    buffer.truncate(read);
                    return Ok(buffer);
                }
                let mut rest = Vec::new();
                let mut chunk = vec![0_u8; yggdryl::DEFAULT_STREAM_BATCH_SIZE];
                loop {
                    let read = stream.pread(position + rest.len() as u64, &mut chunk)?;
                    if read == 0 {
                        return Ok(rest);
                    }
                    rest.extend_from_slice(&chunk[..read]);
                }
            })
            .map_err(storage_error)?;
        Ok(PyBytes::new(py, &bytes))
    }
}

/// The pages of one paginated resource, one `Response` per page.
#[pyclass(
    name = "Pages",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyPages {
    pages: Mutex<Option<Pages>>,
}

impl PyPages {
    fn from_core(pages: Pages) -> Self {
        Self {
            pages: Mutex::new(Some(pages)),
        }
    }

    fn held(&self) -> PyResult<MutexGuard<'_, Option<Pages>>> {
        self.pages.lock().map_err(|_| poisoned("pages"))
    }

    /// Take the walk out, for a conversion that consumes it.
    fn take(&self) -> PyResult<Pages> {
        self.held()?.take().ok_or_else(|| {
            PyValueError::new_err("these pages were already handed over to a reader")
        })
    }
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
impl PyPages {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let next = py.detach(|| {
            self.pages
                .lock()
                .map(|mut pages| pages.as_mut().and_then(Iterator::next))
                .map_err(drop)
        });
        match next.map_err(|()| poisoned("pages"))? {
            None => Ok(None),
            Some(page) => {
                describe(py, Holder::HttpResponse(page.map_err(storage_error)?)).map(Some)
            }
        }
    }

    /// Every page as one record column per page, under `field` or the root
    /// the first page's rows infer, as a `SerieReader`.
    #[pyo3(signature = (field = None))]
    fn read_arrow(
        &self,
        py: Python<'_>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<crate::serie::PySerieReader> {
        let field = field.map(crate::field::core_field_from_value).transpose()?;
        let pages = self.take()?;
        py.detach(|| pages.into_serie_reader(field.as_ref()))
            .map(crate::serie::PySerieReader::from)
            .map_err(storage_error)
    }

    /// Every page as Arrow batches under one root, as a
    /// `pyarrow.RecordBatchReader`: one batch per page, split at
    /// `batch_row_size` rows when that is not zero.
    #[pyo3(signature = (field = None, batch_row_size = 0))]
    fn into_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
        batch_row_size: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let field = field.map(crate::field::core_field_from_value).transpose()?;
        let pages = self.take()?;
        let reader = py
            .detach(|| pages.into_arrow_reader(field.as_ref(), batch_row_size))
            .map_err(storage_error)?;
        crate::iomedia::batch_reader_to_pyarrow(py, reader)
    }
}

/// A client: the connection pool and the transport knobs requests share.
#[pyclass(
    name = "Client",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyClient {
    inner: Client,
}

#[pymethods]
impl PyClient {
    /// A client built for the transport `options` name - timeouts, proxy,
    /// CA bundle, attempts - read by `HttpOptions` property name.
    #[new]
    #[pyo3(signature = (options = None))]
    fn new(options: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let options = match options {
            None => HttpOptions::default(),
            Some(mapping) => {
                HttpOptions::from_properties(property_pairs(mapping)?).map_err(storage_error)?
            }
        };
        Ok(Self {
            inner: Client::with_options(&options).map_err(storage_error)?,
        })
    }

    /// The request counters.
    #[getter]
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        stats_dict(py, self.inner.stats())
    }

    /// A session on this client's pool, spelled as the `Session`
    /// constructor spells one.
    #[pyo3(signature = (base_url = None, *, headers = None, auth = None, timeout = None, options = None))]
    fn session(
        &self,
        py: Python<'_>,
        base_url: Option<&Bound<'_, PyAny>>,
        headers: Option<&Bound<'_, PyAny>>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let session = Settings {
            base_url,
            headers,
            auth,
            timeout,
            options,
        }
        .session(Some(&self.inner))?;
        describe(py, Holder::HttpSession(session))
    }
}

/// A `Content-Range` as Python reads it: `(start, end, total)`.
type Span = (Option<u64>, Option<u64>, Option<u64>);

/// A header section: names compared ignoring case, a name repeated joined
/// into one value, and the typed readers of RFC 9110 beside the mapping.
#[pyclass(
    name = "Headers",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyHeaders {
    inner: Headers,
}

impl PyHeaders {
    const fn from_core(inner: Headers) -> Self {
        Self { inner }
    }
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
impl PyHeaders {
    /// A section from a mapping or `(name, value)` pairs, in order.
    #[new]
    #[pyo3(signature = (value = None))]
    fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self::from_core(match value {
            None => Headers::new(),
            Some(value) => headers_of(value)?,
        }))
    }

    fn __getitem__(&self, name: &str) -> PyResult<String> {
        self.inner
            .get(name)
            .map(str::to_owned)
            .ok_or_else(|| PyKeyError::new_err(name.to_owned()))
    }

    /// Whether a field of `name` is present; anything but a string names
    /// none, as a mapping answers for a key of another type.
    fn __contains__(&self, name: &Bound<'_, PyAny>) -> bool {
        name.extract::<String>()
            .is_ok_and(|name| self.inner.contains_key(&name))
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyIterator>> {
        PyList::new(py, self.inner.iter().map(|(name, _)| name))?.try_iter()
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> Py<PyAny> {
        let py = other.py();
        let Ok(other) = other.cast::<Self>() else {
            return py.NotImplemented();
        };
        let equal = other.get().inner == self.inner;
        match operation {
            CompareOp::Eq => equal
                .into_pyobject(py)
                .map_or_else(|_| py.None(), |value| value.to_owned().into_any().unbind()),
            CompareOp::Ne => (!equal)
                .into_pyobject(py)
                .map_or_else(|_| py.None(), |value| value.to_owned().into_any().unbind()),
            _ => py.NotImplemented(),
        }
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __repr__(&self) -> String {
        format!("Headers({})", self.inner)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    /// The value of `name`, or `default`.
    #[pyo3(signature = (name, default = None))]
    fn get(&self, name: &str, default: Option<String>) -> Option<String> {
        self.inner.get(name).map(str::to_owned).or(default)
    }

    /// Every member of `name`'s value: comma-separated members, and each
    /// `Set-Cookie` apart.
    fn get_all(&self, name: &str) -> Vec<String> {
        self.inner.get_all(name).map(str::to_owned).collect()
    }

    /// The names, lower case, in lexical order.
    fn keys(&self) -> Vec<String> {
        self.inner.iter().map(|(name, _)| name.to_owned()).collect()
    }

    /// The values, in the order of `keys`.
    fn values(&self) -> Vec<String> {
        self.inner
            .iter()
            .map(|(_, value)| value.to_owned())
            .collect()
    }

    /// The `(name, value)` pairs, in the order of `keys`.
    fn items(&self) -> Vec<(String, String)> {
        self.inner
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    /// The section as the JSON object of bare names to values.
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(storage_error)
    }

    /// The stable XXH3-64 hash of the section.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// `Content-Length`, or `None`.
    #[getter]
    fn content_length(&self) -> PyResult<Option<u64>> {
        self.inner.content_length().map_err(storage_error)
    }

    /// `Content-Type` as written, or `None`.
    #[getter]
    fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_owned)
    }

    /// The MIME type `Content-Type` names.
    #[getter]
    fn mime_type(&self) -> PyResult<crate::enums::PyMimeType> {
        self.inner
            .mime_type()
            .map(crate::enums::PyMimeType::from_core)
            .map_err(storage_error)
    }

    /// The media type `Content-Type`, its charset and `Content-Encoding`
    /// state together.
    #[getter]
    fn media_type(&self) -> PyResult<crate::enums::PyMediaType> {
        self.inner
            .media_type()
            .map(crate::enums::PyMediaType::from_core)
            .map_err(storage_error)
    }

    /// The charset `Content-Type` declares, or `None`.
    #[getter]
    fn charset(&self) -> PyResult<Option<&'static str>> {
        Ok(self
            .inner
            .charset()
            .map_err(storage_error)?
            .map(Charset::as_str))
    }

    /// The content codings `Content-Encoding` lists, in order applied.
    #[getter]
    fn content_encoding(&self) -> PyResult<Vec<&'static str>> {
        Ok(self
            .inner
            .content_encoding()
            .map_err(storage_error)?
            .into_iter()
            .map(Codec::as_str)
            .collect())
    }

    /// `Content-Range` as `(start, end, total)`, `end` inclusive; the
    /// unsatisfied form is `(None, None, total)`.
    #[getter]
    fn content_range(&self) -> PyResult<Option<Span>> {
        Ok(self
            .inner
            .content_range()
            .map_err(storage_error)?
            .map(|range| match range {
                ContentRange::Bytes { start, end, total } => (Some(start), Some(end), total),
                ContentRange::Unsatisfied { total } => (None, None, Some(total)),
            }))
    }

    /// Whether `Accept-Ranges` says `bytes`.
    #[getter]
    fn accept_ranges(&self) -> bool {
        self.inner.accept_ranges()
    }

    /// The `ETag`, quoted and marked weak as it was written, or `None`.
    #[getter]
    fn etag(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .etag()
            .map_err(storage_error)?
            .map(|etag| etag.to_string()))
    }

    /// `Last-Modified` as a UTC `datetime`, or `None`.
    #[getter]
    fn last_modified(&self) -> PyResult<Option<SystemTime>> {
        Ok(self
            .inner
            .last_modified()
            .map_err(storage_error)?
            .map(instant))
    }

    /// `Date` as a UTC `datetime`, or `None`.
    #[getter]
    fn date(&self) -> PyResult<Option<SystemTime>> {
        Ok(self.inner.date().map_err(storage_error)?.map(instant))
    }

    /// `Location` as written, or `None`.
    #[getter]
    fn location(&self) -> Option<String> {
        self.inner.location().map(str::to_owned)
    }

    /// How long `Retry-After` asks to wait from now, or `None`.
    #[getter]
    fn retry_after(&self) -> PyResult<Option<Duration>> {
        self.inner.retry_after(now_ns()).map_err(storage_error)
    }

    /// How long an exhausted rate limit asks to wait from now, or `None`.
    #[getter]
    fn rate_limit_pause(&self) -> PyResult<Option<Duration>> {
        self.inner.rate_limit_pause(now_ns()).map_err(storage_error)
    }

    /// The `Link` header, keyed by relation.
    #[getter]
    fn links(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let links = self.inner.links().map_err(storage_error)?;
        links_dict(py, &links)
    }

    /// The target of the `Link` whose relation is `next`, or `None`.
    #[getter]
    fn next_link(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .next_link()
            .map_err(storage_error)?
            .map(str::to_owned))
    }

    /// Whether `Transfer-Encoding` ends in `chunked`.
    #[getter]
    fn transfer_encoding_chunked(&self) -> bool {
        self.inner.transfer_encoding_chunked()
    }

    /// Whether `Connection` lists `close`.
    #[getter]
    fn connection_close(&self) -> bool {
        self.inner.connection_close()
    }

    /// Every `Set-Cookie` value, apart.
    #[getter]
    fn set_cookies(&self) -> Vec<String> {
        self.inner
            .set_cookies()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

/// Read one fault as Python spells it: `"close_before_answer"`,
/// `("cut_body_at", n)`, `("refuse", status[, retry_after])` or
/// `("delay", seconds)`.
fn fault_of(value: &Bound<'_, PyAny>) -> PyResult<Fault> {
    let refused = || {
        PyValueError::new_err(format!(
            "expected a fault as \"close_before_answer\", (\"cut_body_at\", n), \
             (\"refuse\", status[, retry_after]) or (\"delay\", seconds), got {}",
            value
                .repr()
                .map_or_else(|_| "another value".to_owned(), |text| text.to_string())
        ))
    };
    if let Ok(name) = value.cast::<PyString>() {
        return match name.to_str()? {
            "close_before_answer" => Ok(Fault::CloseBeforeAnswer),
            _ => Err(refused()),
        };
    }
    let tuple = value.cast::<PyTuple>().map_err(|_| refused())?;
    let name: String = tuple.get_item(0).map_err(|_| refused())?.extract()?;
    match (name.as_str(), tuple.len()) {
        ("close_before_answer", 1) => Ok(Fault::CloseBeforeAnswer),
        ("cut_body_at", 2) => Ok(Fault::CutBodyAt(tuple.get_item(1)?.extract()?)),
        ("refuse", 2 | 3) => {
            let status = Status::new(tuple.get_item(1)?.extract()?).map_err(storage_error)?;
            let retry_after = match tuple.get_item(2) {
                Ok(delay) if !delay.is_none() => Some(duration_of(&delay, "retry_after")?),
                _ => None,
            };
            Ok(Fault::Refuse {
                status,
                retry_after,
            })
        }
        ("delay", 2) => Ok(Fault::Delay(duration_of(&tuple.get_item(1)?, "delay")?)),
        _ => Err(refused()),
    }
}

/// Read what a route handler answered: a `Response`, or a
/// `(status, headers, body)` tuple.
fn answer_of(value: &Bound<'_, PyAny>) -> PyResult<Response> {
    if let Ok(handle) = value.cast::<PyIOBase>() {
        let handle = handle.borrow();
        if let Holder::HttpResponse(response) = handle.inner()? {
            // The handler may answer the same object again, so the server
            // is handed what it writes - the status, the headers, the body
            // as sent - and the Python object stays whole.
            let body = response.read_all_bytes().map_err(storage_error)?;
            return Response::new(response.status())
                .with_headers(response.headers().clone())
                .map(|answer| answer.with_body(body))
                .map_err(storage_error);
        }
    }
    if let Ok((status, headers, body)) =
        value.extract::<(u16, Bound<'_, PyAny>, Bound<'_, PyAny>)>()
    {
        return built(status, Some(&headers), Some(&body), None);
    }
    Err(PyTypeError::new_err(format!(
        "expected a route handler to answer a yggdryl.http.Response or a (status, headers, \
         body) tuple, got {}",
        type_name(value)
    )))
}

/// A recorded request as a mapping of its parts.
fn recorded_dict(py: Python<'_>, recorded: Recorded) -> PyResult<Py<PyDict>> {
    let entry = PyDict::new(py);
    entry.set_item("method", recorded.method.as_str())?;
    entry.set_item("target", recorded.target)?;
    entry.set_item("path", recorded.path)?;
    entry.set_item("query", recorded.query)?;
    entry.set_item("headers", PyHeaders::from_core(recorded.headers))?;
    entry.set_item("body_len", recorded.body_len)?;
    entry.set_item("status", recorded.status.code())?;
    Ok(entry.unbind())
}

/// Read an optional method name; `None` answers every method.
fn method_of(method: Option<&str>) -> PyResult<Option<Method>> {
    method
        .map(|method| Method::from_str(method).map_err(storage_error))
        .transpose()
}

/// An HTTP/1.1 server hosting `IOBase` handles and programmable routes, on
/// its own accept thread and one thread per connection.
#[pyclass(
    name = "Server",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyServer {
    inner: Mutex<Option<Server>>,
}

impl PyServer {
    fn with<T>(&self, work: impl FnOnce(&Server) -> T) -> PyResult<T> {
        let held = self.inner.lock().map_err(|_| poisoned("server"))?;
        held.as_ref()
            .map(work)
            .ok_or_else(|| PyValueError::new_err("this server was shut down"))
    }
}

#[pymethods]
impl PyServer {
    /// Bind `address` - `127.0.0.1:0` for any loopback port - and start
    /// accepting. Each option left out keeps the core's default: the
    /// timeouts in seconds or a `timedelta`, the sizes in bytes.
    #[staticmethod]
    #[pyo3(signature = (
        address = "127.0.0.1:0",
        *,
        read_timeout = None,
        write_timeout = None,
        max_connections = None,
        max_head_size = None,
        max_body_size = None,
        keep_alive = None,
        recording = None,
        etag = None,
        tunnel = None,
        server_header = None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn bind(
        py: Python<'_>,
        address: &str,
        read_timeout: Option<&Bound<'_, PyAny>>,
        write_timeout: Option<&Bound<'_, PyAny>>,
        max_connections: Option<usize>,
        max_head_size: Option<usize>,
        max_body_size: Option<u64>,
        keep_alive: Option<bool>,
        recording: Option<bool>,
        etag: Option<bool>,
        tunnel: Option<bool>,
        server_header: Option<String>,
    ) -> PyResult<Self> {
        let mut options = ServerOptions::default();
        if let Some(timeout) = read_timeout {
            options = options.with_read_timeout(duration_of(timeout, "read_timeout")?);
        }
        if let Some(timeout) = write_timeout {
            options = options.with_write_timeout(duration_of(timeout, "write_timeout")?);
        }
        if let Some(connections) = max_connections {
            options = options.with_max_connections(connections);
        }
        if let Some(size) = max_head_size {
            options = options.with_max_head_size(size);
        }
        if let Some(size) = max_body_size {
            options = options.with_max_body_size(size);
        }
        if let Some(keep_alive) = keep_alive {
            options = options.with_keep_alive(keep_alive);
        }
        if let Some(recording) = recording {
            options = options.with_recording(recording);
        }
        if let Some(etag) = etag {
            options = options.with_etag(etag);
        }
        if let Some(tunnel) = tunnel {
            options = options.with_tunnel(tunnel);
        }
        if let Some(header) = server_header {
            options = options.with_server_header(header);
        }
        let server = py
            .detach(|| Server::bind_with(address, options))
            .map_err(storage_error)?;
        Ok(Self {
            inner: Mutex::new(Some(server)),
        })
    }

    /// `http://<address>/`.
    #[getter]
    fn url(&self, py: Python<'_>) -> PyResult<Py<crate::uri::PyUrl>> {
        url_object(py, self.with(|server| server.url().clone())?)
    }

    /// The bound port.
    #[getter]
    fn port(&self) -> PyResult<u16> {
        self.with(Server::port)
    }

    /// `path` resolved against `url`.
    fn url_of(&self, py: Python<'_>, path: &str) -> PyResult<Py<crate::uri::PyUrl>> {
        let url = self
            .with(|server| server.url_of(path))?
            .map_err(storage_error)?;
        url_object(py, url)
    }

    /// Serve `handle` under `prefix`: the prefix is the handle, a path below
    /// it the child that path names. The caller's handle stays usable: a
    /// located one is served from its location, an in-memory one from a
    /// copy of its bytes.
    fn mount(&self, prefix: &str, handle: &Bound<'_, PyAny>) -> PyResult<()> {
        let handle = handle.cast::<PyIOBase>().map_err(|_| {
            PyTypeError::new_err(format!(
                "expected a yggdryl.IOBase to mount, got {}",
                type_name(handle)
            ))
        })?;
        let handle = handle.borrow();
        let inner = handle.inner()?;
        // A session or a request is mounted as itself, its state and its
        // role kept; a response and anything held in memory as a copy of its
        // bytes, since asking its URL again would not be asking what it
        // answered; anything else is rebuilt on its location.
        let holder = match inner {
            Holder::HttpSession(session) => Holder::HttpSession(session.clone()),
            Holder::HttpRequest(request) => Holder::HttpRequest(request.clone()),
            Holder::HttpResponse(_) | Holder::HttpStream(_) => copied(inner)?,
            _ if inner.kind() == IOKind::Memory => copied(inner)?,
            _ => handle.rebuilt()?,
        };
        self.with(|server| server.mount(prefix, holder))?
            .map_err(storage_error)
    }

    /// Stop serving `prefix`; whether something was mounted there.
    fn unmount(&self, prefix: &str) -> PyResult<bool> {
        self.with(|server| server.unmount(prefix))
    }

    /// Answer `path` with `handler`, for `method` or every method: the
    /// callable runs on the connection's thread, takes the `Request` and
    /// answers a `Response` or a `(status, headers, body)` tuple. A handler
    /// that raises answers `500`.
    #[pyo3(signature = (path, handler, method = None))]
    fn route(&self, path: &str, handler: Py<PyAny>, method: Option<&str>) -> PyResult<()> {
        let method = method_of(method)?;
        self.with(|server| {
            server.route(method, path, move |request: &Request| {
                Python::attach(|py| {
                    let argument = describe(py, Holder::HttpRequest(request.clone()))?;
                    let answer = handler.bind(py).call1((argument,))?;
                    answer_of(&answer)
                })
                .map_err(|error| yggdryl::Error::Io(std::io::Error::other(error.to_string())))
            });
        })
    }

    /// Answer `path` with the same response every time.
    #[pyo3(signature = (path, status, headers = None, body = None, method = None))]
    fn respond(
        &self,
        path: &str,
        status: u16,
        headers: Option<&Bound<'_, PyAny>>,
        body: Option<&Bound<'_, PyAny>>,
        method: Option<&str>,
    ) -> PyResult<()> {
        let method = method_of(method)?;
        let response = built(status, headers, body, None)?;
        self.with(|server| server.respond(method, path, response))
    }

    /// Forget the route of `method` at `path`; whether there was one.
    #[pyo3(signature = (path, method = None))]
    fn unroute(&self, path: &str, method: Option<&str>) -> PyResult<bool> {
        let method = method_of(method)?;
        self.with(|server| server.unroute(method, path))
    }

    /// Apply `fault` to the next `times` requests of `path` (`0`: every
    /// request), before any route or mount answers.
    #[pyo3(signature = (path, fault, times = 1))]
    fn inject(&self, path: &str, fault: &Bound<'_, PyAny>, times: u32) -> PyResult<()> {
        let fault = fault_of(fault)?;
        self.with(|server| server.inject(path, fault, times))
    }

    /// Forget every injected fault.
    fn clear_faults(&self) -> PyResult<()> {
        self.with(Server::clear_faults)
    }

    /// Every request handled since the last `clear_requests`, in order.
    #[getter]
    fn requests(&self, py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
        self.with(Server::requests)?
            .into_iter()
            .map(|recorded| recorded_dict(py, recorded))
            .collect()
    }

    /// Requests handled since the last `clear_requests`.
    #[getter]
    fn request_count(&self) -> PyResult<usize> {
        self.with(Server::request_count)
    }

    /// Forget the recorded requests and zero the count.
    fn clear_requests(&self) -> PyResult<()> {
        self.with(Server::clear_requests)
    }

    /// Connections accepted so far.
    #[getter]
    fn connections(&self) -> PyResult<u64> {
        self.with(Server::connections)
    }

    /// Stop accepting and join the accept thread; a second call does
    /// nothing.
    fn shutdown(&self, py: Python<'_>) -> PyResult<()> {
        let server = self.inner.lock().map_err(|_| poisoned("server"))?.take();
        match server {
            Some(server) => py.detach(|| server.shutdown()).map_err(storage_error),
            None => Ok(()),
        }
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    #[pyo3(signature = (exception_type = None, exception = None, traceback = None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        exception_type: Option<&Bound<'_, PyAny>>,
        exception: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let _ = (exception_type, exception, traceback);
        self.shutdown(py)?;
        Ok(false)
    }
}

/// Register every HTTP class and the default-session door.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PySession>()?;
    module.add_class::<PyRequest>()?;
    module.add_class::<PyResponse>()?;
    module.add_class::<PyStream>()?;
    module.add_class::<PyPages>()?;
    module.add_class::<PyClient>()?;
    module.add_class::<PyHeaders>()?;
    module.add_class::<PyServer>()?;
    module.add_class::<PyContentIterator>()?;
    module.add_class::<PyLineIterator>()?;
    module.add_class::<PyResponses>()?;
    module.add_function(wrap_pyfunction!(http_session, module)?)?;
    Ok(())
}
