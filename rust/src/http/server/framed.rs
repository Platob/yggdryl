//! What HTTP/2 and HTTP/3 share on the server: a request's head read off a
//! stream's fields, the answer's head as frames carry it, and its body as
//! chunks - a mounted leaf read, or a written body written, on a blocking
//! thread and handed over through a short channel, so the connection's task
//! never waits on storage or on a handler - and the exchange's trace, the
//! request and the answer rendered as the HTTP/1.1 messages they carried.
//!
//! A framed request reaches [`Inner::dispatch`](super::Inner::dispatch) as
//! an HTTP/1.1 one does - the same routes, mounts, faults and log - with its
//! `:authority` as `Host` and its version stated.

use std::io::{self, BufWriter, Write};

use bytes::Bytes;
use ureq::http;

use super::connection::{Cut, stream_source};
use super::trace::Exchange;
use super::{Answer, AnswerBody, Incoming, Inner};
use crate::DEFAULT_STREAM_BATCH_SIZE;
use crate::http::headers::render_http_date;
use crate::http::wire::{RequestHead, ResponseHead, render_request, render_response_head};
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
    let streamed = headers.transfer_encoding_chunked() || matches!(body, AnswerBody::Writer(_));
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
            AnswerBody::Bytes(bytes) => Some(bytes.len() as u64),
            AnswerBody::Stream { length, .. } => Some(*length),
            AnswerBody::Writer(_) => None,
        };
        if let Some(length) = length {
            let _ = headers.insert("content-length", &length.to_string());
        }
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
/// leaf read or a written body written on a blocking thread.
pub(super) enum Chunks {
    Held(Bytes),
    Streamed(tokio::sync::mpsc::Receiver<io::Result<Bytes>>),
}

impl Chunks {
    /// The first `limit` bytes of `body`: all of it when `limit` is `None`,
    /// each chunk copied into `exchange` as it is handed over.
    ///
    /// Must be called inside the runtime: a leaf's reader and a body's
    /// writer are spawned on it.
    pub(super) fn of(body: AnswerBody, limit: Option<u64>, exchange: Option<Exchange>) -> Self {
        match body {
            AnswerBody::Bytes(bytes) => {
                let bytes = bytes.as_bytes();
                let end = limit.map_or(bytes.len(), |at| {
                    usize::try_from(at).unwrap_or(usize::MAX).min(bytes.len())
                });
                if let Some(mut exchange) = exchange {
                    tokio::task::block_in_place(|| {
                        exchange.response(&bytes[..end]);
                        exchange.finish();
                    });
                }
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
                    let mut sink = Sink(sender.clone(), exchange);
                    if let Err(error) = stream_source(&mut sink, source, start, limit) {
                        let _ = sender.blocking_send(Err(error));
                    }
                    sink.finish();
                });
                Self::Streamed(receiver)
            }
            AnswerBody::Writer(writer) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(CHANNEL_DEPTH);
                tokio::task::spawn_blocking(move || {
                    let mut body = BufWriter::with_capacity(
                        DEFAULT_STREAM_BATCH_SIZE,
                        Cut::new(Sink(sender.clone(), exchange), limit),
                    );
                    let wrote = writer(&mut body);
                    let flushed = body.flush();
                    // A cut the body reached ends it there and the stream is
                    // reset for it, as is a writer that failed, so the peer
                    // reads a cut transfer, never a short body; a cut past
                    // the body's end cuts nothing.
                    let failed = if body.get_ref().reached {
                        Some(io::Error::other("the body was cut where the fault asked"))
                    } else {
                        match (wrote, flushed) {
                            (Err(error), _) => Some(io::Error::other(error.to_string())),
                            (Ok(()), Err(error)) => Some(error),
                            (Ok(()), Ok(())) => None,
                        }
                    };
                    if let Some(error) = failed {
                        let _ = sender.blocking_send(Err(error));
                    }
                    if let Ok(cut) = body.into_inner() {
                        cut.inner.finish();
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
/// A written body's length is not known ahead, so its writer reports a cut
/// as it reaches one, through the chunks ([`Chunks::of`]).
pub(super) fn is_truncated(body: &AnswerBody, cut: Option<u64>) -> bool {
    let length = match body {
        AnswerBody::Bytes(bytes) => bytes.len() as u64,
        AnswerBody::Stream { length, .. } => *length,
        AnswerBody::Writer(_) => return false,
    };
    cut.is_some_and(|at| at < length)
}

/// The exchange of a framed request, when the server traces: its request
/// written as the HTTP/1.1 message it carried, off the runtime.
pub(super) fn traced_request(inner: &Inner, incoming: &Incoming) -> Option<Exchange> {
    let trace = inner.trace.as_ref()?;
    let wire = render_request(&incoming.head, &incoming.body);
    Some(tokio::task::block_in_place(|| {
        let mut exchange = trace.begin();
        exchange.request(&wire);
        exchange
    }))
}

/// The exchange of a framed request refused before it was routed - a head
/// the grammar does not read, a body past the bound - when the server
/// traces: its request written as far as it was read, the request line and
/// the fields as the frames carried them, off the runtime.
pub(super) fn traced_refusal(
    inner: &Inner,
    parts: &http::request::Parts,
    version: HttpVersion,
) -> Option<Exchange> {
    let trace = inner.trace.as_ref()?;
    let target = parts
        .uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let mut wire = format!("{} {target} {}\r\n", parts.method, version.as_str()).into_bytes();
    for (name, value) in &parts.headers {
        wire.extend_from_slice(name.as_str().as_bytes());
        wire.extend_from_slice(b": ");
        wire.extend_from_slice(value.as_bytes());
        wire.extend_from_slice(b"\r\n");
    }
    wire.extend_from_slice(b"\r\n");
    Some(tokio::task::block_in_place(|| {
        let mut exchange = trace.begin();
        exchange.request(&wire);
        exchange
    }))
}

/// Write a refusal's body into its trace and close the exchange, off the
/// runtime.
pub(super) fn traced_body(exchange: Option<Exchange>, body: &[u8]) {
    if let Some(mut exchange) = exchange {
        tokio::task::block_in_place(|| {
            exchange.response(body);
            exchange.finish();
        });
    }
}

/// Write the answer's head into its trace as the frames carried it: the
/// status and the fields, under the version that carried them.
pub(super) fn traced_head(
    exchange: &mut Exchange,
    response: &http::Response<()>,
    version: HttpVersion,
) {
    let mut headers = Headers::new();
    for (name, value) in response.headers() {
        let _ = headers.append(
            name.as_str(),
            &crate::Charset::Utf8.transcribe(value.as_bytes()),
        );
    }
    let status = Status::new(response.status().as_u16()).unwrap_or(Status::INTERNAL_SERVER_ERROR);
    let head = ResponseHead {
        version,
        status,
        reason: status.reason().to_owned(),
        headers,
    };
    let head = render_response_head(&head);
    tokio::task::block_in_place(|| {
        exchange.response(&head);
        exchange.flush();
    });
}

/// A writer handing each batch it is given to a connection's task, and a
/// copy to the exchange's trace; on the blocking thread, where a trace
/// write may wait.
struct Sink(
    tokio::sync::mpsc::Sender<io::Result<Bytes>>,
    Option<Exchange>,
);

impl Sink {
    /// The body is done: its trace is appended and closed.
    fn finish(self) {
        if let Some(exchange) = self.1 {
            exchange.finish();
        }
    }
}

impl Write for Sink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .blocking_send(Ok(Bytes::copy_from_slice(buffer)))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        if let Some(exchange) = self.1.as_mut() {
            exchange.response(buffer);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
