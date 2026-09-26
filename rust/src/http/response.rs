//! `Response`: one HTTP answer - the status, the headers, the final URL, the
//! redirect history - and its body, held whole or left on the wire.
//!
//! The body has two states, behind one lock so every reader takes `&self`
//! and two threads may share a handle: *held*, the bytes as they came off
//! the wire with the decoded form cached beside them once asked for, or
//! *streaming*, a [`Stream`] still on the wire. [`Response::bytes`] is the
//! decoded body, [`Response::text`] that body read in the charset the
//! `Content-Type` declares, [`Response::scalar`] that body parsed under its
//! media type through the crate's structured-text codecs. As an [`IOBase`]
//! the response is its body *as sent* - the coded bytes under the media type
//! the headers state - so `Holder::HttpResponse(response).into_declared_media()`
//! composes the coding and the record encoding exactly as it does for a
//! file whose name says `.json.gz`. The lock is never held across a network
//! read: a streaming body is read through its own [`Stream`], borrowed out
//! of the lock by reference count.

use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::client::Answer;
use super::stream::{io_into_error, oversized_body, reader_at, refuse_write};
use super::wire::{ResponseHead, parse_response, render_response};
use super::{Body, Cookie, Headers, HttpVersion, Link, Method, NextPage, Request, Status, Stream};
use crate::holder::{Buffer, Holder};
use crate::{
    ByteStream, Charset, Codec, Error, Field, IOBase, IOKind, Listing, MediaType, Result, Scalar,
    Uri, Url,
};

/// The most of a redirect hop's body that is read before it is dropped.
const DRAIN_LIMIT: u64 = 1024 * 1024;

/// The location a response built by hand, or parsed off the wire, answers.
const PLACEHOLDER_URL: &str = "about:blank";

/// One HTTP answer and its body.
///
/// A response parsed off the wire holds its body whole; one a request sent
/// holds it whole (`send`) or streams it (`stream`). Either way the decoded
/// body is [`Self::bytes`], the text [`Self::text`] and the document
/// [`Self::scalar`].
///
/// ```
/// use yggdryl::http::{Response, Status};
///
/// # fn main() -> yggdryl::Result<()> {
/// // Header names as a section renders them: lower case, in lexical order.
/// let wire = b"HTTP/1.1 200 OK\r\ncontent-length: 16\r\ncontent-type: application/json\r\n\r\n{\"orders\":[1,2]}";
/// let response = Response::from_bytes(wire)?;
///
/// assert_eq!(response.status(), Status::OK);
/// assert_eq!(response.headers().get("content-type"), Some("application/json"));
/// assert_eq!(response.text()?, "{\"orders\":[1,2]}");
/// assert_eq!(response.into_bytes()?, wire);
/// # Ok(())
/// # }
/// ```
pub struct Response {
    request: Request,
    url: Url,
    status: Status,
    version: HttpVersion,
    reason: String,
    headers: Headers,
    media_type: MediaType,
    codings: Vec<Codec>,
    history: Vec<Response>,
    elapsed: Duration,
    body: Mutex<BodyState>,
}

/// The body, held or on the wire.
enum BodyState {
    /// The bytes as sent, and the decoded form once it was asked for.
    Held {
        raw: Arc<[u8]>,
        decoded: Option<Arc<[u8]>>,
    },
    /// Still on the wire.
    Streaming(Arc<Stream>),
}

impl Response {
    /// An empty `status` answer with no headers, for a handler or a test to
    /// build on; its request is a placeholder no one sends.
    pub fn new(status: Status) -> Self {
        let url = placeholder_url();
        Self {
            request: Request::new(Method::Get, url.clone()),
            url,
            status,
            version: HttpVersion::Http11,
            reason: status.reason().to_owned(),
            headers: Headers::new(),
            media_type: MediaType::default(),
            codings: vec![Codec::Identity],
            history: Vec::new(),
            elapsed: Duration::ZERO,
            body: Mutex::new(BodyState::Held {
                raw: Arc::from([]),
                decoded: None,
            }),
        }
    }

    /// The same response under another status.
    #[must_use]
    pub fn with_status(mut self, status: Status) -> Self {
        self.status = status;
        self.reason = status.reason().to_owned();
        self
    }

    /// The same response with one header set.
    ///
    /// # Errors
    ///
    /// As [`Headers::insert`], and a `Content-Type` or `Content-Encoding`
    /// the crate cannot read.
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self> {
        self.headers.insert(name, value)?;
        self.reread_headers()?;
        Ok(self)
    }

    /// The same response with `headers` set over its own, the given winning
    /// a name both carry.
    ///
    /// # Errors
    ///
    /// A `Content-Type` or `Content-Encoding` the crate cannot read.
    pub fn with_headers(mut self, headers: Headers) -> Result<Self> {
        self.headers = headers.merge_with(&self.headers)?;
        self.reread_headers()?;
        Ok(self)
    }

    /// The same response with `body` as sent.
    #[must_use]
    pub fn with_body(self, body: impl Into<Body>) -> Self {
        let raw: Arc<[u8]> = match body.into() {
            Body::Empty => Arc::from([]),
            Body::Bytes(bytes) => bytes,
        };
        self.with_raw(raw)
    }

    /// The same response with `value` as a compact JSON body under
    /// `Content-Type: application/json`.
    ///
    /// # Errors
    ///
    /// A value JSON cannot render.
    pub fn with_json(self, value: &Scalar) -> Result<Self> {
        Ok(self
            .with_header("content-type", "application/json")?
            .with_body(crate::json::into_bytes(value)?))
    }

    /// The same response with `text` as the body under
    /// `Content-Type: text/plain; charset=utf-8`.
    pub fn with_text(self, text: &str) -> Self {
        self.with_header("content-type", "text/plain; charset=utf-8")
            .unwrap_or_else(|_| unreachable!("a constant header value validates"))
            .with_body(text)
    }

    /// The status.
    pub fn status(&self) -> Status {
        self.status
    }

    /// The headers.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// The final URL, after redirects.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The request this answers.
    pub fn request(&self) -> &Request {
        &self.request
    }

    /// The HTTP version the answer stated.
    pub fn version(&self) -> HttpVersion {
        self.version
    }

    /// The redirect hops before this answer, oldest first, bodies drained.
    pub fn history(&self) -> &[Response] {
        &self.history
    }

    /// How long the exchange took, redirects and retries included.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Whether the status is below 400.
    pub fn is_ok(&self) -> bool {
        self.status.code() < 400
    }

    /// Whether the status is a redirect.
    pub fn is_redirect(&self) -> bool {
        self.status.is_redirect()
    }

    /// This response, or the refusal a status of 400 or more is.
    ///
    /// # Errors
    ///
    /// [`Error::Remote`] naming the method, the status, its reason, the
    /// first line of the body and the URL.
    pub fn raise_for_status(&self) -> Result<&Self> {
        if self.is_ok() {
            return Ok(self);
        }
        let body = self.text().unwrap_or_default();
        Err(Error::remote(
            "http",
            self.request.method().as_str(),
            self.status.code(),
            self.status.reason(),
            body.lines().next().unwrap_or_default(),
            &self.url,
        ))
    }

    /// The whole decoded body, materialized once and cached.
    ///
    /// # Errors
    ///
    /// A body over the session's `max_body_size` - as sent or once decoded -
    /// a transport failure, or bytes that are not the coding they claim.
    pub fn bytes(&self) -> Result<Arc<[u8]>> {
        self.materialize()?;
        let mut state = self.state()?;
        let BodyState::Held { raw, decoded } = &mut *state else {
            return Err(poisoned());
        };
        if let Some(decoded) = decoded {
            return Ok(Arc::clone(decoded));
        }
        let limit = self.max_body_size();
        let bytes = if self.codings.iter().all(|coding| coding.is_identity()) {
            Arc::clone(raw)
        } else {
            let mut reader: Box<dyn Read> = Box::new(std::io::Cursor::new(Arc::clone(raw)));
            for coding in self.codings.iter().rev() {
                reader = coding.reader(reader);
            }
            let mut out = Vec::new();
            reader
                .take(limit.saturating_add(1))
                .read_to_end(&mut out)
                .map_err(io_into_error)?;
            if out.len() as u64 > limit {
                return Err(oversized_body(limit, Some(&self.url)));
            }
            Arc::from(out)
        };
        *decoded = Some(Arc::clone(&bytes));
        Ok(bytes)
    }

    /// The decoded body as text, read through [`Charset::transcribe`] in
    /// the charset `Content-Type` declares, UTF-8 when it declares none.
    ///
    /// # Errors
    ///
    /// As [`Self::bytes`].
    pub fn text(&self) -> Result<String> {
        let bytes = self.bytes()?;
        Ok(Charset::from_media_type(&self.media_type)
            .transcribe(&bytes)
            .into_owned())
    }

    /// The decoded body parsed under its media type: JSON, JSON Lines (the
    /// sequence of its documents), YAML, TOML or XML, through the crate's
    /// structured-text codecs.
    ///
    /// ```
    /// use yggdryl::http::Response;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let wire = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 19\r\n\r\n{\"id\":7,\"ok\":true}\n";
    /// let document = Response::from_bytes(wire)?.scalar()?;
    ///
    /// assert_eq!(document.get_key_str("id"), Some(&yggdryl::Scalar::from(7_i64)));
    /// assert_eq!(document.get_key_str("ok"), Some(&yggdryl::Scalar::from(true)));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// As [`Self::bytes`]; a media type naming no structured text format; a
    /// body that is not that format.
    pub fn scalar(&self) -> Result<Scalar> {
        crate::text::from_io(&self.decoded_buffer()?)
    }

    /// [`Self::scalar`], the document read under `field`.
    ///
    /// # Errors
    ///
    /// As [`Self::scalar`], and a document `field` refuses.
    pub fn scalar_with_field(&self, field: &Field) -> Result<Scalar> {
        crate::text::from_io_with_field(&self.decoded_buffer()?, field)
    }

    /// The charset `Content-Type` declares, when it declares one.
    pub fn encoding(&self) -> Option<Charset> {
        self.media_type.charset()
    }

    /// The media type the headers state: `Content-Type`, its charset and
    /// `Content-Encoding`.
    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// The `Content-Length` the headers state.
    pub fn content_length(&self) -> Option<u64> {
        self.headers.content_length().ok().flatten()
    }

    /// Every `Set-Cookie` the answer carried that parses, against the final
    /// URL.
    pub fn cookies(&self) -> Vec<Cookie> {
        let now = self.request.session().now_ns();
        self.headers
            .set_cookies()
            .into_iter()
            .filter_map(|header| Cookie::from_set_cookie(header, &self.url, now).ok())
            .collect()
    }

    /// The `Link` header parsed.
    ///
    /// # Errors
    ///
    /// As [`Headers::links`].
    pub fn links(&self) -> Result<Vec<Link>> {
        self.headers.links()
    }

    /// Where the next page is, per the request's pagination, read off the
    /// headers and - when the body is a structured document - the body.
    ///
    /// # Errors
    ///
    /// A malformed `Link` header or next URL, or a body the pagination
    /// names a path into that is not a document.
    pub fn next_url(&self) -> Result<Option<Url>> {
        let body = self.scalar().ok();
        let rows = body.as_ref().map_or(1, |body| {
            super::pages::rows_of(body, self.request.records()).map_or(1, |rows| rows.len())
        });
        let next =
            self.request
                .pagination()
                .next(&self.url, &self.headers, body.as_ref(), 0, rows)?;
        Ok(match next {
            None => None,
            Some(NextPage::Url(url)) => Some(url),
            Some(NextPage::Parameter { name, value }) => {
                Some(self.request.with_parameter(&name, &value)?.url().clone())
            }
        })
    }

    /// The body as a resumable byte stream: the live one for a streaming
    /// body, a stream over the bytes for a held one.
    ///
    /// # Errors
    ///
    /// A streaming body another reader still holds.
    pub fn into_stream(self) -> Result<Stream> {
        let url = self.url;
        let headers = self.headers;
        match self.body.into_inner().map_err(|_| poisoned())? {
            BodyState::Held { raw, .. } => Ok(Stream::over_bytes(raw, headers).located_at(url)),
            BodyState::Streaming(stream) => Arc::try_unwrap(stream).map_err(|_| {
                Error::Io(std::io::Error::other(
                    "the streaming body is being read by another thread",
                ))
            }),
        }
    }

    /// This response as the [`Holder::HttpResponse`] handle over its body.
    pub fn into_holder(self) -> Holder {
        Holder::HttpResponse(self)
    }

    /// Parse one response message: status line, headers, framed body.
    ///
    /// The request is a placeholder no one sends and the URL `about:blank`.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] with target `http message` at the refusing byte, and
    /// a `Content-Type` or `Content-Encoding` the crate cannot read.
    pub fn from_bytes(wire: &[u8]) -> Result<Self> {
        let (head, body) = parse_response(wire)?;
        let url = placeholder_url();
        let mut response = Self::new(head.status).with_raw(Arc::from(body));
        response.request = Request::new(Method::Get, url.clone());
        response.url = url;
        response.version = head.version;
        response.reason = head.reason;
        response.headers = head.headers;
        response.reread_headers()?;
        Ok(response)
    }

    /// Render one response message: the status line, the headers and the
    /// body as sent, under the framing the headers state.
    ///
    /// # Errors
    ///
    /// A streaming body that could not be read whole.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        let raw = self.raw_bytes()?;
        let head = ResponseHead {
            version: self.version,
            status: self.status,
            reason: self.reason.clone(),
            headers: self.headers.clone(),
        };
        Ok(render_response(&head, &raw))
    }

    /// The response as one record: `status`, `reason`, `version`, `url`,
    /// `headers` (a record of bare names to values) and `body` (the bytes
    /// as sent).
    ///
    /// # Errors
    ///
    /// A streaming body that could not be read whole.
    pub fn into_scalar(&self) -> Result<Scalar> {
        let raw = self.raw_bytes()?;
        let headers = Scalar::from_struct(
            self.headers
                .iter()
                .map(|(name, value)| (name, Scalar::from(value))),
        )?;
        Scalar::from_struct([
            ("status", Scalar::from(i64::from(self.status.code()))),
            ("reason", Scalar::from(self.reason.as_str())),
            ("version", Scalar::from(self.version.as_str())),
            ("url", Scalar::from(self.url.to_string())),
            ("headers", headers),
            ("body", Scalar::from(raw.to_vec())),
        ])
    }

    /// The answer `request` got at `url`, the body read whole here when
    /// `streaming` is false - bounded by the session's `max_body_size` - and
    /// left on the wire inside a [`Stream`] otherwise.
    ///
    /// # Errors
    ///
    /// A `Content-Type` or `Content-Encoding` the crate cannot read, a
    /// body over the bound, or a transport failure reading it.
    pub(crate) fn from_answer(
        request: Request,
        answer: Answer,
        url: Url,
        history: Vec<Response>,
        elapsed: Duration,
        streaming: bool,
    ) -> Result<Self> {
        let media_type = answer.headers.media_type()?;
        let codings = answer.headers.content_encoding()?;
        let limit = request.session().options().max_body_size();
        let status = answer.status;
        let version = answer.version;
        let headers = answer.headers.clone();
        let body = if streaming {
            BodyState::Streaming(Arc::new(Stream::new(
                request.clone(),
                url.clone(),
                answer,
                0,
                None,
            )))
        } else {
            let mut raw = Vec::new();
            answer
                .body
                .take(limit.saturating_add(1))
                .read_to_end(&mut raw)
                .map_err(io_into_error)?;
            if raw.len() as u64 > limit {
                return Err(oversized_body(limit, Some(&url)));
            }
            BodyState::Held {
                raw: Arc::from(raw),
                decoded: None,
            }
        };
        Ok(Self {
            request,
            url,
            status,
            version,
            reason: status.reason().to_owned(),
            headers,
            media_type,
            codings,
            history,
            elapsed,
            body: Mutex::new(body),
        })
    }

    /// A redirect hop for the history: the body read and discarded, bounded.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a hop that cannot be kept.
    pub(crate) fn drained(request: Request, answer: Answer, url: Url) -> Result<Self> {
        let mut sink = Vec::new();
        drop(answer.body.take(DRAIN_LIMIT).read_to_end(&mut sink));
        let headers = answer.headers;
        Ok(Self {
            request,
            url,
            status: answer.status,
            version: answer.version,
            reason: answer.status.reason().to_owned(),
            media_type: headers.media_type().unwrap_or_default(),
            codings: headers
                .content_encoding()
                .unwrap_or_else(|_| vec![Codec::Identity]),
            headers,
            history: Vec::new(),
            elapsed: Duration::ZERO,
            body: Mutex::new(BodyState::Held {
                raw: Arc::from([]),
                decoded: None,
            }),
        })
    }

    /// Re-read what the headers say about the body.
    fn reread_headers(&mut self) -> Result<()> {
        self.media_type = self.headers.media_type()?;
        self.codings = self.headers.content_encoding()?;
        Ok(())
    }

    fn with_raw(self, raw: Arc<[u8]>) -> Self {
        Self {
            body: Mutex::new(BodyState::Held { raw, decoded: None }),
            ..self
        }
    }

    fn max_body_size(&self) -> u64 {
        self.request.session().options().max_body_size()
    }

    fn state(&self) -> Result<MutexGuard<'_, BodyState>> {
        self.body.lock().map_err(|_| poisoned())
    }

    /// The live stream, when the body is one, borrowed out of the lock.
    fn stream(&self) -> Result<Option<Arc<Stream>>> {
        Ok(match &*self.state()? {
            BodyState::Held { .. } => None,
            BodyState::Streaming(stream) => Some(Arc::clone(stream)),
        })
    }

    /// Read a streaming body whole, from its first byte, and hold it.
    fn materialize(&self) -> Result<()> {
        let Some(stream) = self.stream()? else {
            return Ok(());
        };
        let raw = stream.read_from(0, self.max_body_size())?;
        let mut state = self.state()?;
        if let BodyState::Streaming(_) = &*state {
            *state = BodyState::Held {
                raw: Arc::from(raw),
                decoded: None,
            };
        }
        Ok(())
    }

    /// The body as sent, held: what a server writes back out for a handler's
    /// answer, coding and all.
    pub(crate) fn raw_bytes(&self) -> Result<Arc<[u8]>> {
        self.materialize()?;
        match &*self.state()? {
            BodyState::Held { raw, .. } => Ok(Arc::clone(raw)),
            BodyState::Streaming(_) => Err(poisoned()),
        }
    }

    /// The decoded body as a handle carrying the media type without its
    /// codings: the one door the structured-text codecs read through.
    fn decoded_buffer(&self) -> Result<Buffer> {
        let bytes = self.bytes()?;
        let mut media_type = self.media_type.clone();
        media_type.clear_encodings();
        Ok(Buffer::from_bytes(bytes.to_vec()).with_media_type(media_type))
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let body = match self.state() {
            Ok(state) => match &*state {
                BodyState::Held { raw, .. } => format!("{} bytes held", raw.len()),
                BodyState::Streaming(stream) => {
                    format!("streaming, {} delivered", stream.delivered())
                }
            },
            Err(_) => "poisoned".to_owned(),
        };
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body", &body)
            .field("history", &self.history.len())
            .finish()
    }
}

impl crate::IOMedia for Response {
    crate::impl_default_iomedia!();
}

impl IOBase for Response {
    /// Read the body as sent at `offset`: from the held bytes, or forward
    /// through the stream - a position behind its cursor re-fetched by one
    /// ranged `GET` when the resource accepts ranges and refused by name
    /// otherwise.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let stream = {
            let state = self.state()?;
            match &*state {
                BodyState::Held { raw, .. } => {
                    let Ok(offset) = usize::try_from(offset) else {
                        return Ok(0);
                    };
                    if offset >= raw.len() {
                        return Ok(0);
                    }
                    let count = (raw.len() - offset).min(buffer.len());
                    buffer[..count].copy_from_slice(&raw[offset..offset + count]);
                    return Ok(count);
                }
                BodyState::Streaming(stream) => Arc::clone(stream),
            }
        };
        stream.pread(offset, buffer)
    }

    /// Stream the body as sent from `position`: the live stream for a
    /// streaming body, the held bytes otherwise.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        match self.stream()? {
            Some(_) => ByteStream::from_reader(reader_at(self, position), batch_size),
            None => ByteStream::from_handle(self, position, batch_size),
        }
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.raw_bytes()?.to_vec())
    }

    fn pwrite(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize> {
        Err(refuse_write())
    }

    /// The stated `Content-Length`, else the held length, else what the
    /// stream knows.
    fn size(&self) -> u64 {
        if let Some(length) = self.content_length() {
            return length;
        }
        match self.state() {
            Ok(state) => match &*state {
                BodyState::Held { raw, .. } => raw.len() as u64,
                BodyState::Streaming(stream) => stream.size(),
            },
            Err(_) => 0,
        }
    }

    fn capacity(&self) -> u64 {
        self.size()
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Err(refuse_write())
    }

    fn truncate(&mut self, _size: u64) -> Result<()> {
        Err(refuse_write())
    }

    fn uri(&self) -> Option<&Uri> {
        Some(self.url.as_ref())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn mtime(&self) -> Option<i64> {
        self.headers.last_modified().ok().flatten()
    }

    fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.media_type = media_type;
    }

    /// `Memory` for a held body, `File` for a streaming one.
    fn kind(&self) -> IOKind {
        match self.state() {
            Ok(state) => match &*state {
                BodyState::Held { .. } => IOKind::Memory,
                BodyState::Streaming(_) => IOKind::File,
            },
            Err(_) => IOKind::Unknown,
        }
    }

    fn is_container(&self) -> bool {
        false
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        Err(Error::unsupported(
            "resolving a child of a response body",
            format!("{}/{path}", self.url),
        ))
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        Listing::empty()
    }
}

/// The URL a response built by hand answers.
fn placeholder_url() -> Url {
    Url::from_str(PLACEHOLDER_URL)
        .or_else(|_| Url::from_str("http://localhost/"))
        .unwrap_or_else(|_| unreachable!("a constant URL parses"))
}

/// A response whose body state is not what its lock promised.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "an HTTP response lock was poisoned by a panicking reader",
    ))
}

impl Default for Response {
    fn default() -> Self {
        Self::new(Status::OK)
    }
}
