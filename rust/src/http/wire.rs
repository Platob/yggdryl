//! The HTTP/1.1 message grammar of RFC 9112: the request and response heads,
//! the field lines, and the two framings a body takes.
//!
//! A message is parsed once, whole, by [`parse_request`] and
//! [`parse_response`], and rendered by [`render_request`] and
//! [`render_response`]; a body still on the wire is decoded through
//! [`decode_chunked`] and written through [`encode_chunked`]. Every refusal is
//! [`Error::Parse`] with target `http message` naming the byte position it
//! stopped at.
//!
//! What is accepted, and what is not, is RFC 9112 as a recipient reads it: a
//! line ends in CRLF, a bare LF being tolerated as the terminator (section
//! 2.2), while a bare CR anywhere else is refused; a field value is HTAB, SP,
//! VCHAR and obs-text, with obs-text read through [`Charset::transcribe`];
//! obs-fold (a field line continued onto the next) is refused (5.2); a
//! request line or a field line above [`MAX_LINE_BYTES`] and a message with
//! more than [`MAX_FIELD_LINES`] field lines are refused before they are
//! read further; `Content-Length` is a decimal that every repetition agrees
//! on (6.2), and never stands beside `Transfer-Encoding` (6.1); the one
//! transfer coding read is `chunked`, its chunk extensions ignored and its
//! trailer fields folded into the headers (7.1), after which the message
//! carries `Content-Length` and no `Transfer-Encoding`, as 7.1.3 says a
//! decoded message does.

use std::fmt;
use std::io::{self, Read, Write};
use std::str::FromStr;

use memchr::memchr;
use smol_str::{SmolStr, format_smolstr};

use super::{Headers, Method, Status};
use crate::{Charset, Error, Result};

/// The longest request line, status line, field line or chunk-size line
/// read, in bytes, its terminator excluded.
pub const MAX_LINE_BYTES: usize = 8192;

/// The most field lines one message head, or one trailer section, holds.
pub const MAX_FIELD_LINES: usize = 256;

/// The [`Error::Parse`] target every refusal here names.
const TARGET: &str = "http message";

/// The HTTP version a message states.
///
/// ```
/// use yggdryl::http::HttpVersion;
///
/// assert_eq!(HttpVersion::from_str("HTTP/1.1").unwrap(), HttpVersion::Http11);
/// assert_eq!(HttpVersion::Http10.as_str(), "HTTP/1.0");
/// assert!(HttpVersion::from_str("HTTP/2").is_err());
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HttpVersion {
    /// `HTTP/1.0`: no persistent connections, no chunked transfer coding.
    Http10,
    /// `HTTP/1.1`: the grammar this module speaks.
    #[default]
    Http11,
}

impl HttpVersion {
    /// Both versions, oldest first.
    pub const ALL: [Self; 2] = [Self::Http10, Self::Http11];

    /// Parse `HTTP/1.0` or `HTTP/1.1`, case-insensitively.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `http version` for any other text.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// The wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http10 => "HTTP/1.0",
            Self::Http11 => "HTTP/1.1",
        }
    }
}

impl AsRef<str> for HttpVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for HttpVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for HttpVersion {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|version| normalized.eq_ignore_ascii_case(version.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "http version",
                position: 0,
                reason: format_smolstr!("expected HTTP/1.0 or HTTP/1.1, got {value:?}"),
            })
    }
}

impl serde::Serialize for HttpVersion {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for HttpVersion {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}

/// The request line and the field lines of one request message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestHead {
    /// The request method.
    pub method: Method,
    /// The request target as spelled on the wire: an origin form (`/path?q`),
    /// an absolute URL, an authority or `*`.
    pub target: String,
    /// The version the request states.
    pub version: HttpVersion,
    /// The field lines, trailer fields included once the body is decoded.
    pub headers: Headers,
}

/// The status line and the field lines of one response message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseHead {
    /// The version the response states.
    pub version: HttpVersion,
    /// The status code.
    pub status: Status,
    /// The reason phrase as sent, possibly empty; informational only.
    pub reason: String,
    /// The field lines, trailer fields included once the body is decoded.
    pub headers: Headers,
}

/// How the headers of a message frame its body (RFC 9112 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    /// `Transfer-Encoding: chunked`.
    Chunked,
    /// `Content-Length: n`.
    Length(u64),
    /// Neither: a request has no body, a response reads to the end.
    Unstated,
}

/// Parse one request message: the head, then the body it frames.
///
/// A request with neither `Content-Length` nor `Transfer-Encoding` has no
/// body, so any byte after its head is refused; the whole input must be one
/// message.
///
/// # Errors
///
/// Returns [`Error::Parse`] with target `http message` at the byte position
/// of the first thing the grammar refuses: a malformed request line, an
/// unknown method or version, a field line that is folded, over
/// [`MAX_LINE_BYTES`], past [`MAX_FIELD_LINES`], or holds a bare CR or a
/// control byte, a `Content-Length` that is not a decimal or disagrees with
/// a repetition or with `Transfer-Encoding`, a chunk size that is not
/// hexadecimal, a body shorter than its framing, or bytes after the message.
///
/// ```
/// use yggdryl::http::{Method, parse_request};
///
/// let wire = b"POST /rows HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\n\r\nhello";
/// let (head, body) = parse_request(wire).unwrap();
/// assert_eq!(head.method, Method::Post);
/// assert_eq!(head.target, "/rows");
/// assert_eq!(head.headers.get("host"), Some("example.com"));
/// assert_eq!(body, b"hello");
/// ```
pub fn parse_request(bytes: &[u8]) -> Result<(RequestHead, Vec<u8>)> {
    let (line, position) = line_at(bytes, 0)?;
    let (method, target, version) = parse_request_line(line, 0)?;
    let (mut headers, framing, end) = parse_field_lines(bytes, position)?;
    let framing = match framing {
        Framing::Unstated => Framing::Length(0),
        stated => stated,
    };
    let (body, end) = frame_body(bytes, end, framing, &mut headers)?;
    refuse_trailing(bytes, end)?;
    Ok((
        RequestHead {
            method,
            target,
            version,
            headers,
        },
        body,
    ))
}

/// Parse one request head - the request line and the field lines up to and
/// including the empty line - off `bytes`, which must hold exactly that.
///
/// This is the door a server reads a socket through: the head is read up to
/// the blank line, parsed here, and the body then framed off the socket by
/// what the headers state (`Content-Length`, or `Transfer-Encoding: chunked`
/// through [`decode_chunked`]), never held whole in one buffer first.
///
/// # Errors
///
/// Returns [`Error::Parse`] with target `http message` for what
/// [`parse_request`] refuses in a head, and for any byte after the empty
/// line.
pub(crate) fn parse_request_head(bytes: &[u8]) -> Result<RequestHead> {
    let (line, position) = line_at(bytes, 0)?;
    let (method, target, version) = parse_request_line(line, 0)?;
    let (headers, _, end) = parse_field_lines(bytes, position)?;
    refuse_trailing(bytes, end)?;
    Ok(RequestHead {
        method,
        target,
        version,
        headers,
    })
}

/// Render one response head - the status line, the field lines and the
/// empty line - exactly as `head` states it, adding no framing.
///
/// [`render_response`] frames a body it is handed; a server that streams a
/// body it does not hold whole states the framing in the headers itself and
/// writes the body after this head.
pub(crate) fn render_response_head(head: &ResponseHead) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(head.version.as_str().as_bytes());
    out.push(b' ');
    out.extend_from_slice(head.status.code().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(head.reason.as_bytes());
    out.extend_from_slice(b"\r\n");
    render_fields(&mut out, &head.headers);
    out.extend_from_slice(b"\r\n");
    out
}

/// Parse one response message: the head, then the body it frames.
///
/// A `1xx`, `204` or `304` response has no body whatever its headers state
/// (RFC 9112 6.3); a response stating neither `Content-Length` nor
/// `Transfer-Encoding` reads to the end of the input; the whole input must
/// be one message.
///
/// # Errors
///
/// Returns [`Error::Parse`] with target `http message` at the byte position
/// of the first thing the grammar refuses; see [`parse_request`].
///
/// ```
/// use yggdryl::http::{Status, parse_response};
///
/// let wire = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\nX-Sum: 6\r\n\r\n";
/// let (head, body) = parse_response(wire).unwrap();
/// assert_eq!(head.status, Status::OK);
/// assert_eq!(body, b"abc");
/// // Decoded: the trailer joins the headers, the length replaces the coding.
/// assert_eq!(head.headers.get("x-sum"), Some("6"));
/// assert_eq!(head.headers.get("content-length"), Some("3"));
/// assert_eq!(head.headers.get("transfer-encoding"), None);
/// ```
pub fn parse_response(bytes: &[u8]) -> Result<(ResponseHead, Vec<u8>)> {
    let (line, position) = line_at(bytes, 0)?;
    let (version, status, reason) = parse_status_line(line, 0)?;
    let (mut headers, framing, end) = parse_field_lines(bytes, position)?;
    let framing = if status_has_no_body(status) {
        Framing::Length(0)
    } else {
        framing
    };
    let (body, end) = frame_body(bytes, end, framing, &mut headers)?;
    refuse_trailing(bytes, end)?;
    Ok((
        ResponseHead {
            version,
            status,
            reason,
            headers,
        },
        body,
    ))
}

/// Render one request message: the request line, the field lines, an empty
/// line, then the body under the framing the headers state.
///
/// A `Transfer-Encoding` header writes the body as one chunk and the last
/// chunk; a `Content-Length` header writes the body as it is, the head being
/// the owner of what it states; neither writes `Content-Length` when the body
/// is not empty or the method carries one by convention.
///
/// ```
/// use yggdryl::http::{Headers, HttpVersion, Method, RequestHead, render_request};
///
/// let head = RequestHead {
///     method: Method::Put,
///     target: "/rows".to_owned(),
///     version: HttpVersion::Http11,
///     headers: Headers::from_entries([("Host", "example.com")]).unwrap(),
/// };
/// let wire = render_request(&head, b"hello");
/// assert_eq!(
///     wire,
///     b"PUT /rows HTTP/1.1\r\nhost: example.com\r\ncontent-length: 5\r\n\r\nhello"
/// );
/// ```
pub fn render_request(head: &RequestHead, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + body.len());
    out.extend_from_slice(head.method.as_str().as_bytes());
    out.push(b' ');
    out.extend_from_slice(head.target.as_bytes());
    out.push(b' ');
    out.extend_from_slice(head.version.as_str().as_bytes());
    out.extend_from_slice(b"\r\n");
    let framing = stated_framing(&head.headers);
    render_fields(&mut out, &head.headers);
    if framing == Framing::Unstated && (!body.is_empty() || head.method.has_request_body()) {
        render_content_length(&mut out, body.len());
    }
    out.extend_from_slice(b"\r\n");
    render_body(&mut out, framing, body);
    out
}

/// Render one response message: the status line, the field lines, an empty
/// line, then the body under the framing the headers state.
///
/// The framing rules are [`render_request`]'s; `Content-Length` is added
/// when neither framing is stated, except on a `1xx`, `204` or `304`, which
/// carry no body.
///
/// ```
/// use yggdryl::http::{Headers, HttpVersion, ResponseHead, Status, render_response};
///
/// let head = ResponseHead {
///     version: HttpVersion::Http11,
///     status: Status::OK,
///     reason: "OK".to_owned(),
///     headers: Headers::from_entries([("Content-Type", "text/plain")]).unwrap(),
/// };
/// let wire = render_response(&head, b"hi");
/// assert_eq!(
///     wire,
///     b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 2\r\n\r\nhi"
/// );
/// ```
pub fn render_response(head: &ResponseHead, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + body.len());
    out.extend_from_slice(head.version.as_str().as_bytes());
    out.push(b' ');
    out.extend_from_slice(head.status.code().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(head.reason.as_bytes());
    out.extend_from_slice(b"\r\n");
    let framing = stated_framing(&head.headers);
    render_fields(&mut out, &head.headers);
    if framing == Framing::Unstated && !status_has_no_body(head.status) {
        render_content_length(&mut out, body.len());
    }
    out.extend_from_slice(b"\r\n");
    render_body(&mut out, framing, body);
    out
}

/// Read a chunked body off `reader`, yielding the decoded bytes.
///
/// The reader's `trailers` answer the trailer section once the last chunk
/// has been read. A malformed body is an [`io::Error`] of kind
/// `InvalidData` (or `UnexpectedEof` when the input ends first) wrapping the
/// [`Error::Parse`] that names the byte position in the encoded stream.
///
/// ```
/// use std::io::{Cursor, Read};
///
/// use yggdryl::http::decode_chunked;
///
/// let wire = b"4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nExpires: 0\r\n\r\n";
/// let mut reader = decode_chunked(Cursor::new(&wire[..]));
/// let mut body = String::new();
/// reader.read_to_string(&mut body).unwrap();
/// assert_eq!(body, "Wikipedia");
/// assert_eq!(reader.trailers().unwrap().get("expires"), Some("0"));
/// ```
pub fn decode_chunked<R: Read>(reader: R) -> ChunkedReader<R> {
    ChunkedReader::new(reader)
}

/// Write a chunked body onto `writer`: every `write` is one chunk, and
/// [`ChunkedWriter::finish`] writes the last chunk.
///
/// ```
/// use std::io::Write;
///
/// use yggdryl::http::encode_chunked;
///
/// let mut writer = encode_chunked(Vec::new());
/// writer.write_all(b"Wiki").unwrap();
/// writer.write_all(b"pedia").unwrap();
/// let wire = writer.finish().unwrap();
/// assert_eq!(wire, b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n");
/// ```
pub fn encode_chunked<W: Write>(writer: W) -> ChunkedWriter<W> {
    ChunkedWriter::new(writer)
}

/// One refusal at `position`.
fn refuse(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Parse {
        target: TARGET,
        position,
        reason: reason.into(),
    }
}

/// `true` for a byte of the `tchar` production (RFC 9110 5.6.2).
const fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// `true` for a byte a field value may hold: HTAB, SP, VCHAR or obs-text.
const fn is_field_byte(byte: u8) -> bool {
    byte == b'\t' || (byte >= b' ' && byte != 0x7f)
}

/// `true` for a response status whose message carries no body (RFC 9112 6.3).
const fn status_has_no_body(status: Status) -> bool {
    status.is_informational() || status.code() == 204 || status.code() == 304
}

/// One line of `bytes` at `position`: the line without its terminator, and
/// the position after the terminator.
///
/// A LF ends the line, one CR before it is the terminator's, and a CR anywhere
/// else is refused where it lies.
fn line_at(bytes: &[u8], position: usize) -> Result<(&[u8], usize)> {
    let rest = &bytes[position..];
    let Some(lf) = memchr(b'\n', rest) else {
        if rest.len() > MAX_LINE_BYTES + 1 {
            return Err(refuse(
                position,
                format_smolstr!("line exceeds {MAX_LINE_BYTES} bytes"),
            ));
        }
        return Err(refuse(
            bytes.len(),
            "unexpected end of message: expected a line ending in CRLF",
        ));
    };
    let line = rest[..lf].strip_suffix(b"\r").unwrap_or(&rest[..lf]);
    if line.len() > MAX_LINE_BYTES {
        return Err(refuse(
            position,
            format_smolstr!("line exceeds {MAX_LINE_BYTES} bytes"),
        ));
    }
    if let Some(cr) = memchr(b'\r', line) {
        return Err(refuse(position + cr, "bare CR in a line"));
    }
    Ok((line, position + lf + 1))
}

/// The request line: `method SP request-target SP HTTP-version`.
fn parse_request_line(line: &[u8], position: usize) -> Result<(Method, String, HttpVersion)> {
    let Some(first) = memchr(b' ', line) else {
        return Err(refuse(
            position,
            "expected a request line `METHOD target HTTP/1.1`",
        ));
    };
    let Some(second) = memchr(b' ', &line[first + 1..]).map(|index| first + 1 + index) else {
        return Err(refuse(
            position + first + 1,
            "expected a request line `METHOD target HTTP/1.1`",
        ));
    };
    let method = std::str::from_utf8(&line[..first])
        .ok()
        .and_then(|text| Method::from_str(text).ok())
        .ok_or_else(|| {
            refuse(
                position,
                format_smolstr!(
                    "expected a request method, got {:?}",
                    String::from_utf8_lossy(&line[..first])
                ),
            )
        })?;
    let target = &line[first + 1..second];
    if target.is_empty() {
        return Err(refuse(position + first + 1, "expected a request target"));
    }
    if let Some(index) = target.iter().position(|byte| !(b'!'..=b'~').contains(byte)) {
        return Err(refuse(
            position + first + 1 + index,
            format_smolstr!("invalid byte {:#04x} in the request target", target[index]),
        ));
    }
    let version = parse_version(&line[second + 1..], position + second + 1)?;
    let target = std::str::from_utf8(target)
        .map_err(|_| refuse(position + first + 1, "request target is not ASCII"))?
        .to_owned();
    Ok((method, target, version))
}

/// The status line: `HTTP-version SP status-code [SP reason-phrase]`.
fn parse_status_line(line: &[u8], position: usize) -> Result<(HttpVersion, Status, String)> {
    let Some(first) = memchr(b' ', line) else {
        return Err(refuse(
            position,
            "expected a status line `HTTP/1.1 200 reason`",
        ));
    };
    let version = parse_version(&line[..first], position)?;
    let rest = &line[first + 1..];
    let code_position = position + first + 1;
    if rest.len() < 3 || !rest[..3].iter().all(u8::is_ascii_digit) {
        return Err(refuse(code_position, "expected a three-digit status code"));
    }
    let code = rest[..3]
        .iter()
        .fold(0_u16, |code, digit| code * 10 + u16::from(digit - b'0'));
    let status = Status::new(code).map_err(|_| {
        refuse(
            code_position,
            format_smolstr!("expected a status code in 100..=599, got {code}"),
        )
    })?;
    let reason = match rest.get(3) {
        None => &rest[3..],
        Some(b' ') => &rest[4..],
        Some(_) => {
            return Err(refuse(
                code_position + 3,
                "expected a space after the status code",
            ));
        }
    };
    if let Some(index) = reason.iter().position(|byte| !is_field_byte(*byte)) {
        return Err(refuse(
            code_position + 4 + index,
            format_smolstr!("control byte {:#04x} in the reason phrase", reason[index]),
        ));
    }
    Ok((
        version,
        status,
        Charset::Utf8.transcribe(reason).into_owned(),
    ))
}

/// The version token of a start line.
fn parse_version(token: &[u8], position: usize) -> Result<HttpVersion> {
    std::str::from_utf8(token)
        .ok()
        .and_then(|text| HttpVersion::from_str(text).ok())
        .ok_or_else(|| {
            refuse(
                position,
                format_smolstr!(
                    "expected HTTP/1.0 or HTTP/1.1, got {:?}",
                    String::from_utf8_lossy(token)
                ),
            )
        })
}

/// One field line: `field-name ":" OWS field-value OWS`.
///
/// The name is a token, so it is ASCII; the value is transcribed, so obs-text
/// reads as the windows-1252 character it is. A folded line is refused.
fn parse_field_line(line: &[u8], position: usize) -> Result<(&str, String)> {
    if matches!(line.first(), Some(b' ' | b'\t')) {
        return Err(refuse(
            position,
            "obsolete line folding is not accepted in a field line",
        ));
    }
    let Some(colon) = memchr(b':', line) else {
        return Err(refuse(position, "expected a field line `name: value`"));
    };
    let name = &line[..colon];
    if name.is_empty() {
        return Err(refuse(position, "expected a field name before `:`"));
    }
    if let Some(index) = name.iter().position(|byte| !is_tchar(*byte)) {
        return Err(refuse(
            position + index,
            format_smolstr!("invalid byte {:#04x} in a field name", name[index]),
        ));
    }
    let value = &line[colon + 1..];
    if let Some(index) = value.iter().position(|byte| !is_field_byte(*byte)) {
        return Err(refuse(
            position + colon + 1 + index,
            format_smolstr!("control byte {:#04x} in a field value", value[index]),
        ));
    }
    let value = trim_whitespace(value);
    // Every byte of `name` is a `tchar`, which is ASCII.
    let name =
        std::str::from_utf8(name).map_err(|_| refuse(position, "field name is not ASCII"))?;
    Ok((name, Charset::Utf8.transcribe(value).into_owned()))
}

/// `bytes` without leading and trailing SP and HTAB.
fn trim_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
        .map_or(start, |index| index + 1);
    &bytes[start..end.max(start)]
}

/// The field lines from `position` to the empty line, folded into `Headers`,
/// with the framing they state and the position after the empty line.
fn parse_field_lines(bytes: &[u8], mut position: usize) -> Result<(Headers, Framing, usize)> {
    let mut headers = Headers::new();
    let mut content_length: Option<u64> = None;
    let mut transfer_encoding: Option<usize> = None;
    let mut count = 0_usize;
    loop {
        let (line, next) = line_at(bytes, position)?;
        if line.is_empty() {
            position = next;
            break;
        }
        count += 1;
        if count > MAX_FIELD_LINES {
            return Err(refuse(
                position,
                format_smolstr!("more than {MAX_FIELD_LINES} field lines"),
            ));
        }
        let (name, value) = parse_field_line(line, position)?;
        if name.eq_ignore_ascii_case("content-length") {
            let length = parse_content_length(&value, content_length, position)?;
            content_length = Some(length);
            headers.insert("content-length", &length.to_string())?;
        } else {
            if name.eq_ignore_ascii_case("transfer-encoding") {
                transfer_encoding.get_or_insert(position);
            }
            headers.append(name, &value)?;
        }
        position = next;
    }
    let framing = match (transfer_encoding, content_length) {
        (Some(at), Some(_)) => {
            return Err(refuse(
                at,
                "Content-Length beside Transfer-Encoding: the framing is ambiguous",
            ));
        }
        (Some(at), None) => {
            check_transfer_encoding(headers.get("transfer-encoding").unwrap_or(""), at)?;
            Framing::Chunked
        }
        (None, Some(length)) => Framing::Length(length),
        (None, None) => Framing::Unstated,
    };
    Ok((headers, framing, position))
}

/// One `Content-Length` value: a list of decimals that agree with each other
/// and with what an earlier field line stated.
fn parse_content_length(value: &str, earlier: Option<u64>, position: usize) -> Result<u64> {
    let mut agreed = earlier;
    for member in value.split(',') {
        let member = member.trim();
        let length = if member.is_empty() || !member.bytes().all(|byte| byte.is_ascii_digit()) {
            None
        } else {
            member.parse::<u64>().ok()
        };
        let Some(length) = length else {
            return Err(refuse(
                position,
                format_smolstr!(
                    "Content-Length must be an unsigned 64-bit decimal, got {member:?}"
                ),
            ));
        };
        match agreed {
            Some(stated) if stated != length => {
                return Err(refuse(
                    position,
                    format_smolstr!(
                        "Content-Length disagrees with an earlier one: expected {stated}, got {length}"
                    ),
                ));
            }
            _ => agreed = Some(length),
        }
    }
    agreed.ok_or_else(|| {
        refuse(
            position,
            "Content-Length must be an unsigned 64-bit decimal",
        )
    })
}

/// `Transfer-Encoding` must name `chunked` alone: it is the one coding read.
fn check_transfer_encoding(value: &str, position: usize) -> Result<()> {
    let mut codings = value
        .split(',')
        .map(str::trim)
        .filter(|coding| !coding.is_empty());
    match (codings.next(), codings.next()) {
        (Some(coding), None) if coding.eq_ignore_ascii_case("chunked") => Ok(()),
        _ => Err(refuse(
            position,
            format_smolstr!("expected `Transfer-Encoding: chunked`, got {value:?}"),
        )),
    }
}

/// The body `framing` states from `position`, and the position after it.
///
/// A chunked body leaves the headers as a decoded message carries them: the
/// trailer fields folded in, `Content-Length` stating the decoded length, no
/// `Transfer-Encoding` and no `Trailer`.
fn frame_body(
    bytes: &[u8],
    position: usize,
    framing: Framing,
    headers: &mut Headers,
) -> Result<(Vec<u8>, usize)> {
    let rest = &bytes[position..];
    match framing {
        Framing::Length(length) => {
            let available = u64::try_from(rest.len()).unwrap_or(u64::MAX);
            if available < length {
                return Err(refuse(
                    bytes.len(),
                    format_smolstr!(
                        "incomplete body: Content-Length states {length} bytes, {available} follow the head"
                    ),
                ));
            }
            // `length <= available`, and `available` is a `usize`.
            let end = position + usize::try_from(length).unwrap_or(rest.len());
            Ok((bytes[position..end].to_vec(), end))
        }
        Framing::Unstated => Ok((rest.to_vec(), bytes.len())),
        Framing::Chunked => {
            let mut reader = ChunkedReader::new(rest).offset(position);
            let mut body = Vec::new();
            reader.read_to_end(&mut body).map_err(from_io)?;
            let end = position + reader.consumed();
            if let Some(trailers) = reader.trailers() {
                for (name, value) in trailers.iter() {
                    headers.append(name, value)?;
                }
            }
            headers.remove("transfer-encoding");
            headers.remove("trailer");
            headers.insert("content-length", &body.len().to_string())?;
            Ok((body, end))
        }
    }
}

/// Refuse any byte after the message ends at `end`.
fn refuse_trailing(bytes: &[u8], end: usize) -> Result<()> {
    if end < bytes.len() {
        return Err(refuse(
            end,
            format_smolstr!("{} bytes after the end of the message", bytes.len() - end),
        ));
    }
    Ok(())
}

/// The [`Error`] a [`ChunkedReader`] wrapped, or the I/O failure itself.
fn from_io(error: io::Error) -> Error {
    match error.downcast::<Error>() {
        Ok(error) => error,
        Err(error) => Error::Io(error),
    }
}

/// The framing the headers of a message state.
fn stated_framing(headers: &Headers) -> Framing {
    if headers.contains_key("transfer-encoding") {
        Framing::Chunked
    } else if let Some(length) = headers
        .get("content-length")
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        Framing::Length(length)
    } else {
        Framing::Unstated
    }
}

/// Every field line, one per name, a value holding LF (a joined `Set-Cookie`)
/// written as one line per member.
fn render_fields(out: &mut Vec<u8>, headers: &Headers) {
    for (name, value) in headers.iter() {
        for member in value.split('\n') {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(member.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
    }
}

/// The `Content-Length` field line for a body of `length` bytes.
fn render_content_length(out: &mut Vec<u8>, length: usize) {
    out.extend_from_slice(b"content-length: ");
    out.extend_from_slice(length.to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
}

/// The body under `framing`: chunked as one chunk and the last chunk, else
/// as it is.
fn render_body(out: &mut Vec<u8>, framing: Framing, body: &[u8]) {
    match framing {
        Framing::Chunked => {
            if !body.is_empty() {
                out.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
                out.extend_from_slice(body);
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"0\r\n\r\n");
        }
        Framing::Length(_) | Framing::Unstated => out.extend_from_slice(body),
    }
}

/// The chunk size of a chunk-size line, its extensions ignored.
fn parse_chunk_size(line: &[u8], position: usize) -> Result<u64> {
    let digits = line
        .iter()
        .position(|byte| !byte.is_ascii_hexdigit())
        .unwrap_or(line.len());
    if digits == 0 {
        return Err(refuse(
            position,
            format_smolstr!(
                "expected a hexadecimal chunk size, got {:?}",
                String::from_utf8_lossy(line)
            ),
        ));
    }
    if digits > 16 {
        return Err(refuse(position, "chunk size exceeds 64 bits"));
    }
    let rest = trim_whitespace(&line[digits..]);
    if !(rest.is_empty() || rest[0] == b';') {
        return Err(refuse(
            position + digits,
            format_smolstr!(
                "expected a chunk extension or the end of the chunk-size line, got {:?}",
                String::from_utf8_lossy(rest)
            ),
        ));
    }
    // Every byte of `digits` is an ASCII hex digit, and there are at most 16.
    let text = std::str::from_utf8(&line[..digits])
        .map_err(|_| refuse(position, "expected a hexadecimal chunk size"))?;
    u64::from_str_radix(text, 16).map_err(|_| refuse(position, "chunk size exceeds 64 bits"))
}

/// Where a [`ChunkedReader`] is in the chunked grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Before a chunk-size line.
    Size,
    /// Inside chunk data, this many bytes left.
    Data(u64),
    /// After chunk data, before its CRLF.
    DataEnd,
    /// Inside the trailer section.
    Trailers,
    /// After the empty line that ends the message.
    Done,
}

/// A `Read` over a chunked body, yielding the decoded bytes.
///
/// Built by [`decode_chunked`]. The reader holds one line's worth of
/// look-ahead ([`MAX_LINE_BYTES`] plus the terminator) and no more: chunk
/// data larger than a caller's buffer is read straight into it.
#[derive(Debug)]
pub struct ChunkedReader<R> {
    inner: R,
    buffer: Vec<u8>,
    start: usize,
    end: usize,
    consumed: usize,
    base: usize,
    state: State,
    trailers: Headers,
    trailer_count: usize,
}

impl<R: Read> ChunkedReader<R> {
    /// A decoder over `inner`, positioned at its first chunk-size line.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: vec![0; MAX_LINE_BYTES + 2],
            start: 0,
            end: 0,
            consumed: 0,
            base: 0,
            state: State::Size,
            trailers: Headers::new(),
            trailer_count: 0,
        }
    }

    /// Name positions from `base`, where the chunked body begins in a message.
    #[must_use]
    fn offset(mut self, base: usize) -> Self {
        self.base = base;
        self
    }

    /// The trailer fields, once the last chunk and the trailer section have
    /// been read; `None` while the body is still being decoded.
    pub fn trailers(&self) -> Option<&Headers> {
        (self.state == State::Done).then_some(&self.trailers)
    }

    /// Whether the last chunk and the trailer section have been read.
    pub fn is_finished(&self) -> bool {
        self.state == State::Done
    }

    /// How many bytes of the encoded stream have been read through the
    /// grammar so far; once finished, where the message ended.
    pub fn consumed(&self) -> usize {
        self.consumed
    }

    /// The underlying reader, positioned after whatever was buffered.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// The absolute position of the next unread encoded byte.
    fn position(&self) -> usize {
        self.base + self.consumed
    }

    /// Read more of `inner` into the buffer, compacting first; `Ok(false)`
    /// at its end.
    fn fill(&mut self) -> io::Result<bool> {
        if self.start > 0 {
            self.buffer.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        if self.end == self.buffer.len() {
            return Ok(false);
        }
        let read = self.inner.read(&mut self.buffer[self.end..])?;
        self.end += read;
        Ok(read > 0)
    }

    /// The range in the buffer of the next line, without its terminator,
    /// consuming the line and the terminator.
    fn line_range(&mut self) -> io::Result<std::ops::Range<usize>> {
        loop {
            let position = self.position();
            if let Some(lf) = memchr(b'\n', &self.buffer[self.start..self.end]) {
                let line_start = self.start;
                let mut line_end = line_start + lf;
                if line_end > line_start && self.buffer[line_end - 1] == b'\r' {
                    line_end -= 1;
                }
                if line_end - line_start > MAX_LINE_BYTES {
                    return Err(invalid(refuse(
                        position,
                        format_smolstr!("line exceeds {MAX_LINE_BYTES} bytes"),
                    )));
                }
                if let Some(cr) = memchr(b'\r', &self.buffer[line_start..line_end]) {
                    return Err(invalid(refuse(position + cr, "bare CR in a line")));
                }
                self.start += lf + 1;
                self.consumed += lf + 1;
                return Ok(line_start..line_end);
            }
            if self.end - self.start > MAX_LINE_BYTES + 1 {
                return Err(invalid(refuse(
                    position,
                    format_smolstr!("line exceeds {MAX_LINE_BYTES} bytes"),
                )));
            }
            if !self.fill()? {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    refuse(
                        self.base + self.consumed + (self.end - self.start),
                        "unexpected end of chunked body: expected a line ending in CRLF",
                    ),
                ));
            }
        }
    }

    /// Read a chunk-size line and move to its data, or to the trailers.
    fn read_size(&mut self) -> io::Result<()> {
        let position = self.position();
        let range = self.line_range()?;
        let size = parse_chunk_size(&self.buffer[range], position).map_err(invalid)?;
        self.state = if size == 0 {
            State::Trailers
        } else {
            State::Data(size)
        };
        Ok(())
    }

    /// Hand out up to `remaining` bytes of chunk data into `out`.
    fn read_data(&mut self, remaining: u64, out: &mut [u8]) -> io::Result<usize> {
        let wanted =
            usize::try_from(remaining).map_or(out.len(), |remaining| remaining.min(out.len()));
        let buffered = self.end - self.start;
        let read = if buffered > 0 {
            let count = wanted.min(buffered);
            out[..count].copy_from_slice(&self.buffer[self.start..self.start + count]);
            self.start += count;
            count
        } else if wanted >= self.buffer.len() {
            self.inner.read(&mut out[..wanted])?
        } else {
            if !self.fill()? {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    refuse(
                        self.position(),
                        format_smolstr!(
                            "unexpected end of chunked body: {remaining} bytes of chunk data missing"
                        ),
                    ),
                ));
            }
            let count = wanted.min(self.end - self.start);
            out[..count].copy_from_slice(&self.buffer[self.start..self.start + count]);
            self.start += count;
            count
        };
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                refuse(
                    self.position(),
                    format_smolstr!(
                        "unexpected end of chunked body: {remaining} bytes of chunk data missing"
                    ),
                ),
            ));
        }
        self.consumed += read;
        let left = remaining - read as u64;
        self.state = if left == 0 {
            State::DataEnd
        } else {
            State::Data(left)
        };
        Ok(read)
    }

    /// Read the CRLF that ends chunk data.
    fn read_data_end(&mut self) -> io::Result<()> {
        let position = self.position();
        let range = self.line_range()?;
        if !range.is_empty() {
            return Err(invalid(refuse(
                position,
                "expected CRLF after the chunk data",
            )));
        }
        self.state = State::Size;
        Ok(())
    }

    /// Read one trailer line, or the empty line that ends the message.
    fn read_trailer(&mut self) -> io::Result<()> {
        let position = self.position();
        let range = self.line_range()?;
        if range.is_empty() {
            self.state = State::Done;
            return Ok(());
        }
        self.trailer_count += 1;
        if self.trailer_count > MAX_FIELD_LINES {
            return Err(invalid(refuse(
                position,
                format_smolstr!("more than {MAX_FIELD_LINES} trailer fields"),
            )));
        }
        let (name, value) = parse_field_line(&self.buffer[range], position).map_err(invalid)?;
        if ["content-length", "transfer-encoding", "host", "trailer"]
            .iter()
            .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
        {
            return Err(invalid(refuse(
                position,
                format_smolstr!("{name} is not allowed as a trailer field"),
            )));
        }
        self.trailers.append(name, &value).map_err(invalid)?;
        Ok(())
    }
}

/// An [`Error`] as the `InvalidData` I/O failure a `Read` reports.
fn invalid(error: Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

impl<R: Read> Read for ChunkedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            match self.state {
                State::Size => self.read_size()?,
                State::Data(remaining) => return self.read_data(remaining, out),
                State::DataEnd => self.read_data_end()?,
                State::Trailers => self.read_trailer()?,
                State::Done => return Ok(0),
            }
        }
    }
}

/// A `Write` that frames every write as one chunk of a chunked body.
///
/// Built by [`encode_chunked`]. Nothing is written for an empty buffer,
/// since an empty chunk would end the body; [`finish`](Self::finish) writes
/// the last chunk and hands the writer back. A writer dropped without
/// `finish` leaves the body unterminated.
#[derive(Debug)]
pub struct ChunkedWriter<W> {
    inner: W,
}

impl<W: Write> ChunkedWriter<W> {
    /// An encoder onto `inner`.
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    /// Write the last chunk and the empty trailer section, flush, and hand
    /// the writer back.
    ///
    /// # Errors
    ///
    /// Returns the writer's own failure.
    pub fn finish(mut self) -> io::Result<W> {
        self.inner.write_all(b"0\r\n\r\n")?;
        self.inner.flush()?;
        Ok(self.inner)
    }

    /// The underlying writer, without a last chunk written.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for ChunkedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.inner
            .write_all(format!("{:x}\r\n", buf.len()).as_bytes())?;
        self.inner.write_all(buf)?;
        self.inner.write_all(b"\r\n")?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
