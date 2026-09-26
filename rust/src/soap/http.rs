//! The HTTP binding of SOAP 1.1: one request read off a connection, one
//! response written back, over any `std::io` stream.
//!
//! A SOAP endpoint needs exactly this much of HTTP/1.1: the request line and
//! headers, a body delimited by `Content-Length` or by chunked transfer
//! coding, and a response whose body is either a known length or a stream of
//! chunks. Everything is bounded - line length, header count, body size - so
//! a hostile peer costs a refusal rather than memory, and every refusal names
//! the status it is answered with.
//!
//! ```
//! use std::io::Cursor;
//!
//! use yggdryl::soap::http::{Request, Status};
//!
//! let wire = b"POST /xmla HTTP/1.1\r\nHost: localhost\r\nContent-Type: text/xml\r\n\
//!              SOAPAction: \"urn:schemas-microsoft-com:xml-analysis:Discover\"\r\n\
//!              Content-Length: 5\r\n\r\nhello";
//! let request = Request::read(&mut Cursor::new(&wire[..]), 1 << 20)?
//!     .expect("a request, not a closed connection");
//! assert_eq!(request.method(), "POST");
//! assert_eq!(request.target(), "/xmla");
//! assert_eq!(request.soap_action(), Some("urn:schemas-microsoft-com:xml-analysis:Discover"));
//! assert_eq!(request.body(), b"hello");
//! assert!(request.keep_alive());
//! assert_eq!(Status::Ok.code(), 200);
//! # Ok::<(), yggdryl::soap::http::HttpError>(())
//! ```

use std::fmt;
use std::io::{self, BufRead, Write};

/// The longest request or header line accepted; a longer one is refused as
/// malformed rather than buffered.
pub const MAX_LINE: usize = 8192;

/// The most header lines one request may carry.
pub const MAX_HEADERS: usize = 128;

/// The status a response answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The request was served.
    Ok,
    /// The request could not be read as HTTP, or its body as the message it
    /// claimed to be.
    BadRequest,
    /// The target names nothing here.
    NotFound,
    /// Only `POST` carries a SOAP message.
    MethodNotAllowed,
    /// A body with neither a length nor a transfer coding.
    LengthRequired,
    /// The body is larger than the endpoint accepts.
    PayloadTooLarge,
    /// The body is not an XML media type.
    UnsupportedMediaType,
    /// The message was read and processing it failed: what a SOAP fault is
    /// answered with, as the binding requires.
    InternalServerError,
}

impl Status {
    /// The numeric status code.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Ok => 200,
            Self::BadRequest => 400,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::LengthRequired => 411,
            Self::PayloadTooLarge => 413,
            Self::UnsupportedMediaType => 415,
            Self::InternalServerError => 500,
        }
    }

    /// The reason phrase HTTP/1.1 gives the code.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::BadRequest => "Bad Request",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::LengthRequired => "Length Required",
            Self::PayloadTooLarge => "Payload Too Large",
            Self::UnsupportedMediaType => "Unsupported Media Type",
            Self::InternalServerError => "Internal Server Error",
        }
    }
}

/// A request that could not be read, and the status that answers it.
#[derive(Debug)]
pub struct HttpError {
    status: Status,
    reason: String,
}

impl HttpError {
    /// A refusal answered with `status`, explained by `reason`.
    pub fn new(status: Status, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }

    /// The status the refusal is answered with.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// Why the request was refused.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {}: {}",
            self.status.code(),
            self.status.reason(),
            self.reason
        )
    }
}

impl std::error::Error for HttpError {}

impl From<io::Error> for HttpError {
    fn from(error: io::Error) -> Self {
        Self::new(Status::BadRequest, format!("reading the request failed: {error}"))
    }
}

/// One HTTP request: its line, its headers and its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    method: String,
    target: String,
    version: Version,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// The HTTP version a request line names, which decides whether the
/// connection outlives the exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// `HTTP/1.0`: one exchange per connection unless `keep-alive` asks.
    Http10,
    /// `HTTP/1.1`: the connection persists unless `close` asks.
    Http11,
}

impl Request {
    /// Read one request off `reader`, `None` when the connection closed
    /// before a request line.
    ///
    /// `max_body` bounds the body; a `Content-Length` past it, or a chunked
    /// body growing past it, is refused with `413`.
    ///
    /// # Errors
    ///
    /// Returns the status that answers a request that cannot be read: `400`
    /// for a malformed line, header or chunk, `411` for a body with no
    /// delimiter, `413` for one past `max_body`.
    pub fn read<R: BufRead>(reader: &mut R, max_body: usize) -> Result<Option<Self>, HttpError> {
        let Some(line) = read_line(reader)? else {
            return Ok(None);
        };
        // A stray empty line between requests is tolerated, as the
        // specification asks of a server.
        let line = if line.is_empty() {
            match read_line(reader)? {
                Some(line) => line,
                None => return Ok(None),
            }
        } else {
            line
        };
        let mut parts = line.split(' ');
        let (method, target, version) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(method), Some(target), Some(version), None)
                if !method.is_empty() && !target.is_empty() =>
            {
                (method, target, version)
            }
            _ => {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("expected `METHOD target HTTP/1.x` as the request line, got {line:?}"),
                ));
            }
        };
        let version = match version {
            "HTTP/1.1" => Version::Http11,
            "HTTP/1.0" => Version::Http10,
            other => {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("expected HTTP/1.0 or HTTP/1.1, got {other:?}"),
                ));
            }
        };
        let mut headers = Vec::new();
        loop {
            let Some(line) = read_line(reader)? else {
                return Err(HttpError::new(
                    Status::BadRequest,
                    "the connection closed inside the request headers",
                ));
            };
            if line.is_empty() {
                break;
            }
            if headers.len() >= MAX_HEADERS {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("more than {MAX_HEADERS} request headers"),
                ));
            }
            // Obsolete line folding is refused rather than joined (RFC 7230
            // section 3.2.4 allows either): a peer that joins it and one that
            // does not disagree about which header the fold belongs to.
            if line.starts_with([' ', '\t']) && !line.trim().is_empty() {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("a folded header line is not accepted, got {line:?}"),
                ));
            }
            let Some((name, value)) = line.split_once(':') else {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("expected `Name: value` as a header, got {line:?}"),
                ));
            };
            // The name runs up to the colon with nothing between (RFC 7230
            // section 3.2.4): peers that trim it differently frame the
            // message differently.
            if name.is_empty() || name.contains([' ', '\t']) {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!(
                        "expected a header name followed by its colon, got {:?} before the colon",
                        name.trim()
                    ),
                ));
            }
            headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let mut request = Self {
            method: method.to_owned(),
            target: target.to_owned(),
            version,
            headers,
            body: Vec::new(),
        };
        request.body = request.read_body(reader, max_body)?;
        Ok(Some(request))
    }

    fn read_body<R: BufRead>(&self, reader: &mut R, max_body: usize) -> Result<Vec<u8>, HttpError> {
        // Every Transfer-Encoding line is one list (RFC 7230 section 3.2.2),
        // and only a body whose one coding is chunked can be framed here.
        let mut chunked = false;
        for (_, codings) in self
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
        {
            if codings
                .split(',')
                .any(|coding| !coding.trim().eq_ignore_ascii_case("chunked"))
            {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("expected the chunked transfer coding, got {codings:?}"),
                ));
            }
            chunked = true;
        }
        if chunked {
            return read_chunked(reader, max_body);
        }
        let Some(length) = self.header("content-length") else {
            // A request without a body: `GET`, or a `POST` of nothing.
            if self.method == "GET" || self.method == "HEAD" {
                return Ok(Vec::new());
            }
            return Err(HttpError::new(
                Status::LengthRequired,
                "a request body needs a Content-Length or the chunked transfer coding",
            ));
        };
        // Two lengths that disagree frame two different bodies, and the bytes
        // after the shorter one would be read as the next request: refused,
        // never resolved by picking one (RFC 7230 section 3.3.2).
        if let Some((_, other)) = self
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .find(|(_, held)| held.trim() != length.trim())
        {
            return Err(HttpError::new(
                Status::BadRequest,
                format!("Content-Length is stated twice and disagrees: {length:?} and {other:?}"),
            ));
        }
        let length: usize = match length.trim().parse() {
            Ok(length) => length,
            // All digits and past `usize`: a byte count past any bound.
            Err(_) if is_count(length.trim(), 10) => {
                return Err(HttpError::new(
                    Status::PayloadTooLarge,
                    format!("the body is {length} bytes; at most {max_body} are accepted"),
                ));
            }
            Err(_) => {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("expected a byte count as Content-Length, got {length:?}"),
                ));
            }
        };
        if length > max_body {
            return Err(HttpError::new(
                Status::PayloadTooLarge,
                format!("the body is {length} bytes; at most {max_body} are accepted"),
            ));
        }
        let mut body = vec![0_u8; length];
        reader.read_exact(&mut body).map_err(|error| {
            HttpError::new(
                Status::BadRequest,
                format!("the connection closed inside a {length}-byte body: {error}"),
            )
        })?;
        Ok(body)
    }

    /// The method, as sent.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The request target: the path and query, as sent.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The HTTP version.
    #[must_use]
    pub const fn version(&self) -> Version {
        self.version
    }

    /// The first header named `name`, matched case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Every header, names lowercased, in wire order.
    #[must_use]
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The `Content-Type` header.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// The `SOAPAction` header with its quotes taken off, `None` when absent
    /// or empty.
    #[must_use]
    pub fn soap_action(&self) -> Option<&str> {
        let action = self.header(super::ACTION_HEADER)?.trim();
        let action = action
            .strip_prefix('"')
            .and_then(|action| action.strip_suffix('"'))
            .unwrap_or(action);
        (!action.is_empty()).then_some(action)
    }

    /// Whether the connection outlives this exchange: HTTP/1.1 unless
    /// `Connection: close`, HTTP/1.0 only with `Connection: keep-alive`.
    #[must_use]
    pub fn keep_alive(&self) -> bool {
        let connection = self.header("connection").map(str::to_ascii_lowercase);
        match self.version {
            Version::Http11 => connection.as_deref() != Some("close"),
            Version::Http10 => connection.as_deref() == Some("keep-alive"),
        }
    }
}

/// Read one CRLF-terminated line, `None` at a clean end of stream before any
/// byte; a bare LF terminator is accepted as well.
fn read_line<R: BufRead>(reader: &mut R) -> Result<Option<String>, HttpError> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(HttpError::new(
                Status::BadRequest,
                "the connection closed inside a line",
            ));
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(end) => {
                line.extend_from_slice(&available[..end]);
                reader.consume(end + 1);
                break;
            }
            None => {
                line.extend_from_slice(available);
                let taken = available.len();
                reader.consume(taken);
            }
        }
        // The bound counts the line's own bytes: the CR of a CRLF terminator
        // may still be in the buffer here, so it is the one byte allowed past.
        if line.len() > MAX_LINE + 1 {
            return Err(HttpError::new(
                Status::BadRequest,
                format!("a line longer than {MAX_LINE} bytes"),
            ));
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > MAX_LINE {
        return Err(HttpError::new(
            Status::BadRequest,
            format!("a line longer than {MAX_LINE} bytes"),
        ));
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| HttpError::new(Status::BadRequest, "a line that is not UTF-8"))
}

/// Whether `text` is a non-empty run of digits in `radix`: a count too large
/// to hold rather than text that is not a count.
fn is_count(text: &str, radix: u32) -> bool {
    !text.is_empty() && text.chars().all(|character| character.is_digit(radix))
}

/// Read a chunked body: hexadecimal sizes, the chunks they announce, and the
/// trailer up to its blank line.
fn read_chunked<R: BufRead>(reader: &mut R, max_body: usize) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    loop {
        let Some(line) = read_line(reader)? else {
            return Err(HttpError::new(
                Status::BadRequest,
                "the connection closed inside a chunked body",
            ));
        };
        let size = line.split(';').next().unwrap_or("").trim();
        let size = match usize::from_str_radix(size, 16) {
            Ok(size) => size,
            // Hexadecimal and past `usize`: a chunk past any bound.
            Err(_) if is_count(size, 16) => {
                return Err(HttpError::new(
                    Status::PayloadTooLarge,
                    format!("the chunked body grew past the {max_body} bytes accepted"),
                ));
            }
            Err(_) => {
                return Err(HttpError::new(
                    Status::BadRequest,
                    format!("expected a hexadecimal chunk size, got {size:?}"),
                ));
            }
        };
        if size == 0 {
            // Trailers, up to the blank line that ends the body.
            loop {
                match read_line(reader)? {
                    Some(line) if line.is_empty() => return Ok(body),
                    Some(_) => {}
                    None => {
                        return Err(HttpError::new(
                            Status::BadRequest,
                            "the connection closed inside the chunked trailer",
                        ));
                    }
                }
            }
        }
        if body.len().saturating_add(size) > max_body {
            return Err(HttpError::new(
                Status::PayloadTooLarge,
                format!("the chunked body grew past the {max_body} bytes accepted"),
            ));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..]).map_err(|error| {
            HttpError::new(
                Status::BadRequest,
                format!("the connection closed inside a {size}-byte chunk: {error}"),
            )
        })?;
        match read_line(reader)? {
            Some(line) if line.is_empty() => {}
            _ => {
                return Err(HttpError::new(
                    Status::BadRequest,
                    "expected the line break that ends a chunk",
                ));
            }
        }
    }
}

/// Write a response with a body of known length.
///
/// # Errors
///
/// Returns the sink's failure.
pub fn write_response<W: Write>(
    writer: &mut W,
    status: Status,
    content_type: &str,
    keep_alive: bool,
    body: &[u8],
) -> io::Result<()> {
    write!(
        writer,
        "HTTP/1.1 {} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: {}\r\n\r\n",
        status.code(),
        status.reason(),
        body.len(),
        if keep_alive { "keep-alive" } else { "close" },
    )?;
    writer.write_all(body)?;
    writer.flush()
}

/// Write the head of a response whose body follows as chunks, and hand back
/// the writer the chunks go through.
///
/// # Errors
///
/// Returns the sink's failure.
pub fn begin_chunked<W: Write>(
    mut writer: W,
    status: Status,
    content_type: &str,
    keep_alive: bool,
) -> io::Result<ChunkedWriter<W>> {
    write!(
        writer,
        "HTTP/1.1 {} {}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: {}\r\n\r\n",
        status.code(),
        status.reason(),
        if keep_alive { "keep-alive" } else { "close" },
    )?;
    Ok(ChunkedWriter {
        writer,
        buffer: Vec::with_capacity(CHUNK),
    })
}

/// The chunk size a streamed body is cut into: what one write hands the
/// socket, large enough that a row never costs its own chunk header.
const CHUNK: usize = 64 * 1024;

/// A body written as chunks: bytes gather up to one chunk, each full chunk
/// goes out with its size line, and [`finish`](Self::finish) writes the last
/// chunk and the terminator.
pub struct ChunkedWriter<W: Write> {
    writer: W,
    buffer: Vec<u8>,
}

impl<W: Write> ChunkedWriter<W> {
    fn write_chunk(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        write!(self.writer, "{:x}\r\n", self.buffer.len())?;
        self.writer.write_all(&self.buffer)?;
        self.writer.write_all(b"\r\n")?;
        self.buffer.clear();
        Ok(())
    }

    /// Write what is gathered, the terminating chunk and the empty trailer,
    /// and hand the sink back.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure.
    pub fn finish(mut self) -> io::Result<W> {
        self.write_chunk()?;
        self.writer.write_all(b"0\r\n\r\n")?;
        self.writer.flush()?;
        Ok(self.writer)
    }
}

impl<W: Write> Write for ChunkedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= CHUNK {
            self.write_chunk()?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_chunk()?;
        self.writer.flush()
    }
}
