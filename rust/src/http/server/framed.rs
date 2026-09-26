//! What HTTP/2 and HTTP/3 share on the server: a request's head read off a
//! stream's fields, the answer's head as frames carry it, and its body as
//! chunks - a mounted leaf read on a blocking thread and handed over through
//! a short channel, so the connection's task never waits on storage.
//!
//! A framed request reaches [`Inner::dispatch`](super::Inner::dispatch) as
//! an HTTP/1.1 one does - the same routes, mounts, faults and log - with its
//! `:authority` as `Host` and its version stated.

use std::io::{self, Write};

use bytes::Bytes;
use ureq::http;

use super::connection::stream_source;
use super::{Answer, AnswerBody};
use crate::http::headers::render_http_date;
use crate::http::wire::RequestHead;
use crate::http::{Headers, HttpVersion, Method, Status};

/// How many chunks of a streamed leaf wait between its reader and the
/// connection.
const CHANNEL_DEPTH: usize = 4;

/// The head of a framed request as the server's grammar holds it; the
/// status and text to answer when it is not one the server reads.
pub(super) fn head_of(
    parts: &http::request::Parts,
    version: HttpVersion,
) -> std::result::Result<RequestHead, (Status, String)> {
    let method = Method::from_str(parts.method.as_str()).map_err(|error| {
        (
            Status::new(501).unwrap_or(Status::BAD_REQUEST),
            error.to_string(),
        )
    })?;
    let target = parts
        .uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str)
        .to_owned();
    let mut headers = Headers::new();
    if let Some(authority) = parts.uri.authority() {
        headers
            .insert("host", authority.as_str())
            .map_err(|error| (Status::BAD_REQUEST, error.to_string()))?;
    }
    for (name, value) in &parts.headers {
        let value = crate::Charset::Utf8.transcribe(value.as_bytes());
        headers
            .append(name.as_str(), &value)
            .map_err(|error| (Status::BAD_REQUEST, error.to_string()))?;
    }
    Ok(RequestHead {
        method,
        target,
        version,
        headers,
    })
}

/// The answer's head as frames carry it - `Server` and `Date` stated, no
/// field of one HTTP/1.1 connection, `Content-Length` wherever the length is
/// known - and the body to send, `None` for a `HEAD` or a status that has
/// none.
pub(super) fn prepared(
    answer: Answer,
    head_only: bool,
    server: &str,
) -> (http::Response<()>, Option<AnswerBody>) {
    let Answer {
        status,
        mut headers,
        body,
    } = answer;
    let _ = headers.insert("server", server);
    let _ = headers.insert("date", &render_http_date(super::connection::now_ns()));
    let streamed = headers.transfer_encoding_chunked();
    for name in [
        "connection",
        "keep-alive",
        "proxy-connection",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
    let has_body = !(status.is_informational()
        || status == Status::NO_CONTENT
        || status == Status::NOT_MODIFIED);
    if !has_body {
        headers.remove("content-length");
    } else if !streamed {
        let length = match &body {
            AnswerBody::Bytes(bytes) => bytes.len() as u64,
            AnswerBody::Stream { length, .. } => *length,
        };
        let _ = headers.insert("content-length", &length.to_string());
    }
    let mut response = http::Response::builder().status(status.code());
    for (name, value) in &headers {
        response = response.header(name, value);
    }
    let response = response.body(()).unwrap_or_else(|_| {
        // Every field the server holds passed the field grammar, which the
        // framing's own is a subset of; a status the builder refuses is
        // answered as the server's own failure.
        let mut fallback = http::Response::new(());
        *fallback.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
        fallback
    });
    let body = (has_body && !head_only).then_some(body);
    (response, body)
}

/// A body as the chunks a framed connection sends: bytes in hand sliced, a
/// leaf read on a blocking thread.
pub(super) enum Chunks {
    Held(Bytes),
    Streamed(tokio::sync::mpsc::Receiver<io::Result<Bytes>>),
}

impl Chunks {
    /// The first `limit` bytes of `body`: all of it when `limit` is `None`.
    ///
    /// Must be called inside the runtime: a leaf's reader is spawned on it.
    pub(super) fn of(body: AnswerBody, limit: Option<u64>) -> Self {
        match body {
            AnswerBody::Bytes(bytes) => {
                let bytes = bytes.as_bytes();
                let end = limit.map_or(bytes.len(), |at| {
                    usize::try_from(at).unwrap_or(usize::MAX).min(bytes.len())
                });
                Self::Held(Bytes::copy_from_slice(&bytes[..end]))
            }
            AnswerBody::Stream {
                source,
                start,
                length,
            } => {
                let limit = limit.map_or(length, |at| at.min(length));
                let (sender, receiver) = tokio::sync::mpsc::channel(CHANNEL_DEPTH);
                tokio::task::spawn_blocking(move || {
                    let mut sink = Sink(sender.clone());
                    if let Err(error) = stream_source(&mut sink, source, start, limit) {
                        let _ = sender.blocking_send(Err(error));
                    }
                });
                Self::Streamed(receiver)
            }
        }
    }

    /// The next chunk, at most `most` bytes of held ones; `None` at the end.
    pub(super) async fn next(&mut self, most: usize) -> Option<io::Result<Bytes>> {
        match self {
            Self::Held(bytes) if bytes.is_empty() => None,
            Self::Held(bytes) => Some(Ok(bytes.split_to(most.min(bytes.len())))),
            Self::Streamed(receiver) => receiver.recv().await,
        }
    }
}

/// Whether an answer cut at `cut` ends before the body it announced: the
/// stream is then reset where it stops, as an HTTP/1.1 connection is closed.
pub(super) fn is_truncated(body: &AnswerBody, cut: Option<u64>) -> bool {
    let length = match body {
        AnswerBody::Bytes(bytes) => bytes.len() as u64,
        AnswerBody::Stream { length, .. } => *length,
    };
    cut.is_some_and(|at| at < length)
}

/// A writer handing each batch it is given to a connection's task.
struct Sink(tokio::sync::mpsc::Sender<io::Result<Bytes>>);

impl Write for Sink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .blocking_send(Ok(Bytes::copy_from_slice(buffer)))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
