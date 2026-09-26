//! One connection: the loop that reads a request off the socket, frames its
//! body, hands it to the server and writes the answer, until the peer
//! closes, a request is malformed, or a fault severs it.
//!
//! The head is read up to the blank line within [`ServerOptions::max_head_size`]
//! and parsed by the wire grammar; the body is framed by `Content-Length`,
//! or by `Transfer-Encoding: chunked` through [`decode_chunked`], within
//! [`ServerOptions::max_body_size`]. `Expect: 100-continue` is acknowledged
//! before the body is read. Every answer carries `Server` and `Date`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use super::{Answer, AnswerBody, Incoming, Inner, Mounted, Outcome, Source};
use crate::http::headers::render_http_date;
use crate::http::wire::{
    HttpVersion, RequestHead, ResponseHead, decode_chunked, encode_chunked, parse_request_head,
    render_response_head,
};
use crate::http::{Headers, Method, Status};
use crate::{DEFAULT_STREAM_BATCH_SIZE, IOBase};

/// Why a connection stops being read.
enum Refusal {
    /// The peer went away, or the request was cut short.
    Closed,
    /// The bytes were not a request this server reads: the status and the
    /// text to answer before closing.
    Malformed(Status, String),
}

impl From<io::Error> for Refusal {
    fn from(_: io::Error) -> Self {
        Self::Closed
    }
}

/// The socket under a deadline: every read waits no longer than the idle
/// bound, nor past the deadline while one is set.
///
/// The socket's own timeout restarts with every byte, so a peer trickling a
/// head one byte at a time would hold its connection for ever under it; a
/// deadline narrows each read to the time left, which a socket option
/// cannot. The option is re-stated only when the wait changes.
struct Deadline {
    stream: TcpStream,
    idle: Duration,
    until: Option<Instant>,
    stated: Option<Duration>,
}

impl Deadline {
    fn new(stream: TcpStream, idle: Duration) -> Self {
        Self {
            stream,
            idle,
            until: None,
            stated: None,
        }
    }
}

impl Read for Deadline {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let wait = match self.until {
            Some(until) => until
                .saturating_duration_since(Instant::now())
                .min(self.idle),
            None => self.idle,
        };
        if wait.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if self.stated != Some(wait) {
            self.stream.set_read_timeout(Some(wait))?;
            self.stated = Some(wait);
        }
        self.stream.read(buffer)
    }
}

/// Serve `stream` until it closes, is severed, or a request is malformed.
pub(super) fn serve(inner: &Inner, stream: TcpStream) {
    let options = &inner.options;
    // An answer is a head and a body written apart; with Nagle on, the body
    // waits for the peer's delayed acknowledgement of the head - forty
    // milliseconds a request on most stacks.
    if stream.set_nodelay(true).is_err()
        || stream
            .set_write_timeout(Some(options.write_timeout))
            .is_err()
    {
        return;
    }
    let mut reader = BufReader::new(Deadline::new(stream, options.read_timeout));
    loop {
        reader.get_mut().until = Some(Instant::now() + options.read_timeout);
        let head = read_head(&mut reader, options.max_head_size);
        reader.get_mut().until = None;
        let head = match head {
            Ok(Some(head)) => head,
            Ok(None) | Err(Refusal::Closed) => return,
            Err(Refusal::Malformed(status, text)) => {
                refuse(
                    &mut reader.get_mut().stream,
                    status,
                    &text,
                    options.server_header(),
                );
                return;
            }
        };
        let mut head = match parse_request_head(&head) {
            Ok(head) => head,
            Err(error) => {
                refuse(
                    &mut reader.get_mut().stream,
                    Status::BAD_REQUEST,
                    &error.to_string(),
                    options.server_header(),
                );
                return;
            }
        };
        if head.method == Method::Connect {
            tunnel(inner, reader, head);
            return;
        }
        let keep_alive = options.keep_alive && keeps_alive(&head);
        if head
            .headers
            .get("expect")
            .is_some_and(|value| value.eq_ignore_ascii_case("100-continue"))
            && reader
                .get_mut()
                .stream
                .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .is_err()
        {
            return;
        }
        let body = match read_body(
            &mut reader,
            head.version,
            &mut head.headers,
            options.max_body_size,
        ) {
            Ok(body) => body,
            Err(Refusal::Closed) => return,
            Err(Refusal::Malformed(status, text)) => {
                refuse(
                    &mut reader.get_mut().stream,
                    status,
                    &text,
                    options.server_header(),
                );
                return;
            }
        };
        let head_only = head.method == Method::Head;
        match inner.dispatch(Incoming { head, body }) {
            Outcome::Close => return,
            Outcome::Answer { answer, cut } => {
                let close = !keep_alive || cut.is_some();
                let written = write_answer(
                    &mut reader.get_mut().stream,
                    head_only,
                    answer,
                    close,
                    cut,
                    options.server_header(),
                );
                if written.is_err() || close {
                    return;
                }
            }
        }
    }
}

/// Answer a `CONNECT`: `405` unless the options tunnel, else `502` when the
/// address cannot be reached, else `200` and the bytes piped both ways until
/// either side closes - the connection is the tunnel's from then on.
fn tunnel(inner: &Inner, mut reader: BufReader<Deadline>, head: RequestHead) {
    let options = &inner.options;
    let incoming = Incoming {
        head,
        body: Vec::new(),
    };
    let target = incoming.head.target.clone();
    if !options.tunnel {
        inner.record(&incoming, "", Vec::new(), Status::METHOD_NOT_ALLOWED);
        refuse(
            &mut reader.get_mut().stream,
            Status::METHOD_NOT_ALLOWED,
            "this server does not tunnel CONNECT",
            options.server_header(),
        );
        return;
    }
    let upstream = std::net::ToSocketAddrs::to_socket_addrs(&target)
        .ok()
        .and_then(|mut addresses| addresses.next())
        .and_then(|address| TcpStream::connect_timeout(&address, options.read_timeout).ok());
    let Some(upstream) = upstream else {
        inner.record(&incoming, "", Vec::new(), Status::BAD_GATEWAY);
        refuse(
            &mut reader.get_mut().stream,
            Status::BAD_GATEWAY,
            &format!("could not reach {target}"),
            options.server_header(),
        );
        return;
    };
    inner.record(&incoming, "", Vec::new(), Status::OK);
    let client = &mut reader.get_mut().stream;
    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .is_err()
    {
        return;
    }
    let (Ok(mut upstream_reader), Ok(mut client_writer)) =
        (upstream.try_clone(), client.try_clone())
    else {
        return;
    };
    let back = std::thread::spawn(move || {
        let _ = io::copy(&mut upstream_reader, &mut client_writer);
        let _ = client_writer.shutdown(std::net::Shutdown::Write);
    });
    // The reader still holds what the client sent past the head.
    let mut upstream_writer = upstream;
    let _ = io::copy(&mut reader, &mut upstream_writer);
    let _ = upstream_writer.shutdown(std::net::Shutdown::Write);
    let _ = back.join();
}

/// The bytes of one request head up to and including the empty line, or
/// `None` at the end of the stream before any byte of one; empty lines
/// before the request line are skipped (RFC 9112 2.2).
fn read_head(
    reader: &mut BufReader<Deadline>,
    max_head_size: usize,
) -> std::result::Result<Option<Vec<u8>>, Refusal> {
    let mut head = Vec::new();
    let mut started = false;
    // Blank lines before the request line count against the bound too, so
    // a peer sending only those is refused like any other long head.
    let mut skipped = 0_usize;
    loop {
        let before = head.len();
        // One more byte than the bound tells a too-long head from an exact one.
        let limit = (max_head_size + 1).saturating_sub(before + skipped);
        let read = reader
            .by_ref()
            .take(limit as u64)
            .read_until(b'\n', &mut head)?;
        if read == 0 {
            return if started {
                Err(Refusal::Closed)
            } else {
                Ok(None)
            };
        }
        if head.len() + skipped > max_head_size {
            return Err(Refusal::Malformed(
                Status::new(431).unwrap_or(Status::BAD_REQUEST),
                format!("request head longer than {max_head_size} bytes"),
            ));
        }
        let line = &head[before..];
        let blank = line == b"\r\n" || line == b"\n";
        if !started {
            if blank {
                skipped += head.len();
                head.clear();
                continue;
            }
            started = true;
            continue;
        }
        if blank {
            return Ok(Some(head));
        }
        if line.last() != Some(&b'\n') {
            return Err(Refusal::Closed);
        }
    }
}

/// HTTP/1.1 keeps the connection unless `Connection: close`; HTTP/1.0
/// closes unless `Connection: keep-alive`.
fn keeps_alive(head: &RequestHead) -> bool {
    let wants = |token: &str| {
        head.headers.get("connection").is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        })
    };
    match head.version {
        HttpVersion::Http10 => wants("keep-alive"),
        HttpVersion::Http11 => !wants("close"),
    }
}

/// The body the headers frame: chunked, else `Content-Length`, else none.
///
/// A chunked body leaves the headers as a decoded message carries them: the
/// trailers folded in, `content-length` the decoded length, no
/// `transfer-encoding`. A `Transfer-Encoding` on an HTTP/1.0 request, or one
/// whose last coding is not `chunked`, frames nothing this server can trust
/// and is refused (RFC 9112 6.1 and 6.3): reading it any other way is how a
/// proxy in front and this server come to disagree on where a request ends.
fn read_body(
    reader: &mut BufReader<Deadline>,
    version: HttpVersion,
    headers: &mut Headers,
    max_body_size: u64,
) -> std::result::Result<Vec<u8>, Refusal> {
    let too_large = || {
        Refusal::Malformed(
            Status::new(413).unwrap_or(Status::BAD_REQUEST),
            format!("request body longer than {max_body_size} bytes"),
        )
    };
    let last = headers
        .get_all("transfer-encoding")
        .last()
        .map(str::to_owned);
    if let Some(last) = &last {
        if version == HttpVersion::Http10 {
            return Err(Refusal::Malformed(
                Status::BAD_REQUEST,
                "Transfer-Encoding on an HTTP/1.0 request".to_owned(),
            ));
        }
        if !last.eq_ignore_ascii_case("chunked") {
            return Err(Refusal::Malformed(
                Status::BAD_REQUEST,
                format!("Transfer-Encoding ending in {last:?} rather than chunked"),
            ));
        }
    }
    if last.is_some() {
        let mut decoder = decode_chunked(reader.by_ref());
        let mut body = Vec::new();
        match decoder
            .by_ref()
            .take(max_body_size.saturating_add(1))
            .read_to_end(&mut body)
        {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(Refusal::Closed);
            }
            Err(error) => {
                return Err(Refusal::Malformed(Status::BAD_REQUEST, error.to_string()));
            }
        }
        if body.len() as u64 > max_body_size {
            return Err(too_large());
        }
        if let Some(trailers) = decoder.trailers() {
            for (name, value) in trailers.iter() {
                if headers.append(name, value).is_err() {
                    return Err(Refusal::Malformed(
                        Status::BAD_REQUEST,
                        format!("trailer field {name:?} is not a field the grammar reads"),
                    ));
                }
            }
        }
        headers.remove("transfer-encoding");
        headers.remove("trailer");
        let _ = headers.insert("content-length", &body.len().to_string());
        return Ok(body);
    }
    let length = match headers.content_length() {
        Ok(length) => length.unwrap_or(0),
        Err(error) => return Err(Refusal::Malformed(Status::BAD_REQUEST, error.to_string())),
    };
    if length > max_body_size {
        return Err(too_large());
    }
    let mut body = vec![0; usize::try_from(length).map_err(|_| too_large())?];
    reader.read_exact(&mut body)?;
    Ok(body)
}

/// Answer `status` with `text` and close; a failure to write is the peer's.
fn refuse(stream: &mut TcpStream, status: Status, text: &str, server: &str) {
    let answer = Answer::text(status, text);
    let _ = write_answer(stream, false, answer, true, None, server);
}

/// UTC nanoseconds since the epoch.
fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
        .unwrap_or(0)
}

/// Write `answer`: the head with `Server`, `Date` and the framing, then the
/// body unless `head_only`; `cut` stops the body after that many bytes.
///
/// A `1xx`, `204` or `304` carries no body and no framing. A body in hand
/// keeps a `Transfer-Encoding: chunked` the handler stated and is written in
/// [`DEFAULT_STREAM_BATCH_SIZE`] chunks; otherwise, and for every streamed
/// leaf, `Content-Length` states the length. A `HEAD` declares the length
/// its `GET` would carry and never a chunked framing.
fn write_answer(
    stream: &mut TcpStream,
    head_only: bool,
    answer: Answer,
    close: bool,
    cut: Option<u64>,
    server: &str,
) -> io::Result<()> {
    let Answer {
        status,
        mut headers,
        body,
    } = answer;
    let _ = headers.insert("server", server);
    let _ = headers.insert("date", &render_http_date(now_ns()));
    if close {
        let _ = headers.insert("connection", "close");
    }
    let has_body = !(status.is_informational()
        || status == Status::NO_CONTENT
        || status == Status::NOT_MODIFIED);
    let chunked = has_body
        && !head_only
        && matches!(body, AnswerBody::Bytes(_))
        && headers.transfer_encoding_chunked();
    if !has_body {
        headers.remove("content-length");
        headers.remove("transfer-encoding");
    } else if !chunked {
        headers.remove("transfer-encoding");
        let length = match &body {
            AnswerBody::Bytes(bytes) => bytes.len() as u64,
            AnswerBody::Stream { length, .. } => *length,
        };
        let _ = headers.insert("content-length", &length.to_string());
    }
    let head = ResponseHead {
        version: HttpVersion::Http11,
        status,
        reason: status.reason().to_owned(),
        headers,
    };
    stream.write_all(&render_response_head(&head))?;
    if head_only || !has_body {
        return stream.flush();
    }
    match body {
        AnswerBody::Bytes(bytes) => {
            let bytes = bytes.as_bytes();
            let limit = cut.map_or(bytes.len(), |at| {
                usize::try_from(at).unwrap_or(usize::MAX).min(bytes.len())
            });
            if chunked {
                let mut encoder = encode_chunked(&mut *stream);
                for chunk in bytes[..limit].chunks(DEFAULT_STREAM_BATCH_SIZE) {
                    encoder.write_all(chunk)?;
                }
                if cut.is_none() {
                    encoder.finish()?;
                }
            } else {
                stream.write_all(&bytes[..limit])?;
            }
        }
        AnswerBody::Stream {
            source,
            start,
            length,
        } => {
            let limit = cut.map_or(length, |at| at.min(length));
            match source {
                Source::Owned(holder) => stream_leaf(stream, holder.as_ref(), start, limit)?,
                Source::Root(shared) => stream_root(stream, &shared, start, limit)?,
            }
        }
    }
    stream.flush()
}

/// Write `limit` bytes of the mounted holder itself from `start`, one
/// [`DEFAULT_STREAM_BATCH_SIZE`] range read per batch.
///
/// The mount's lock is taken for each read and never across a socket write:
/// a peer slow to take its answer then holds no `PUT` or `DELETE` on the
/// mount behind it. The price is one ranged read per batch where a child
/// streams from one.
fn stream_root(
    stream: &mut TcpStream,
    shared: &RwLock<Mounted>,
    start: u64,
    limit: u64,
) -> io::Result<()> {
    let mut position = start;
    let mut remaining = limit;
    while remaining > 0 {
        let want = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(DEFAULT_STREAM_BATCH_SIZE);
        let batch = shared
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .holder
            .read_range_bytes(position, want)
            .map_err(io::Error::other)?;
        if batch.is_empty() {
            break;
        }
        stream.write_all(&batch)?;
        position += batch.len() as u64;
        remaining -= batch.len() as u64;
    }
    Ok(())
}

/// Write `limit` bytes of `holder` from `start`, streamed in
/// [`DEFAULT_STREAM_BATCH_SIZE`] batches; a leaf shorter than its stated
/// length ends the body short, which the peer reads as a severed transfer.
fn stream_leaf(
    stream: &mut TcpStream,
    holder: &dyn IOBase,
    start: u64,
    limit: u64,
) -> io::Result<()> {
    let mut remaining = limit;
    if remaining == 0 {
        return Ok(());
    }
    let batches = holder
        .pstream_bytes(start, DEFAULT_STREAM_BATCH_SIZE)
        .map_err(io::Error::other)?;
    for batch in batches {
        let batch = batch.map_err(io::Error::other)?;
        let take = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(batch.len());
        stream.write_all(&batch[..take])?;
        remaining -= take as u64;
        if remaining == 0 {
            break;
        }
    }
    Ok(())
}
