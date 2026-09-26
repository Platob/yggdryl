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
use std::time::SystemTime;

use super::{Answer, AnswerBody, Incoming, Inner, Outcome, Source};
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

/// Serve `stream` until it closes, is severed, or a request is malformed.
pub(super) fn serve(inner: &Inner, stream: TcpStream) {
    let options = &inner.options;
    if stream.set_read_timeout(Some(options.read_timeout)).is_err() {
        return;
    }
    let mut reader = BufReader::new(stream);
    loop {
        let head = match read_head(&mut reader, options.max_head_size) {
            Ok(Some(head)) => head,
            Ok(None) | Err(Refusal::Closed) => return,
            Err(Refusal::Malformed(status, text)) => {
                refuse(reader.get_mut(), status, &text, options.server_header());
                return;
            }
        };
        let mut head = match parse_request_head(&head) {
            Ok(head) => head,
            Err(error) => {
                refuse(
                    reader.get_mut(),
                    Status::BAD_REQUEST,
                    &error.to_string(),
                    options.server_header(),
                );
                return;
            }
        };
        let keep_alive = options.keep_alive && keeps_alive(&head);
        if head
            .headers
            .get("expect")
            .is_some_and(|value| value.eq_ignore_ascii_case("100-continue"))
            && reader
                .get_mut()
                .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .is_err()
        {
            return;
        }
        let body = match read_body(&mut reader, &mut head.headers, options.max_body_size) {
            Ok(body) => body,
            Err(Refusal::Closed) => return,
            Err(Refusal::Malformed(status, text)) => {
                refuse(reader.get_mut(), status, &text, options.server_header());
                return;
            }
        };
        let head_only = head.method == Method::Head;
        match inner.dispatch(Incoming { head, body }) {
            Outcome::Close => return,
            Outcome::Answer { answer, cut } => {
                let close = !keep_alive || cut.is_some();
                let written = write_answer(
                    reader.get_mut(),
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

/// The bytes of one request head up to and including the empty line, or
/// `None` at the end of the stream before any byte of one; empty lines
/// before the request line are skipped (RFC 9112 2.2).
fn read_head(
    reader: &mut BufReader<TcpStream>,
    max_head_size: usize,
) -> std::result::Result<Option<Vec<u8>>, Refusal> {
    let mut head = Vec::new();
    let mut started = false;
    loop {
        let before = head.len();
        // One more byte than the bound tells a too-long head from an exact one.
        let limit = (max_head_size + 1).saturating_sub(before);
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
        if head.len() > max_head_size {
            return Err(Refusal::Malformed(
                Status::new(431).unwrap_or(Status::BAD_REQUEST),
                format!("request head longer than {max_head_size} bytes"),
            ));
        }
        let line = &head[before..];
        let blank = line == b"\r\n" || line == b"\n";
        if !started {
            if blank {
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
/// `transfer-encoding`.
fn read_body(
    reader: &mut BufReader<TcpStream>,
    headers: &mut Headers,
    max_body_size: u64,
) -> std::result::Result<Vec<u8>, Refusal> {
    let too_large = || {
        Refusal::Malformed(
            Status::new(413).unwrap_or(Status::BAD_REQUEST),
            format!("request body longer than {max_body_size} bytes"),
        )
    };
    if headers.transfer_encoding_chunked() {
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
                Source::Root(shared) => {
                    let mounted = shared
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    stream_leaf(stream, &mounted.holder, start, limit)?;
                }
            }
        }
    }
    stream.flush()
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
