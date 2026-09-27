//! HTTP/2 on the server's TCP port: by prior knowledge (`h2c`), and over TLS
//! when the server answers HTTP/3 and so holds a certificate.
//!
//! A connection whose first bytes are the HTTP/2 preface is handed to the
//! private runtime and served as HTTP/2; one that opens with a TLS record is
//! served HTTP/2 over TLS, `h2` being the one protocol its ALPN offers; any
//! other is HTTP/1.1, as before.
//! The connection's thread waits on it, so it counts against
//! [`ServerOptions::max_connections`](super::ServerOptions::max_connections)
//! as an HTTP/1.1 one does. Every stream is answered on a task of its own -
//! its request collected within the body bound, dispatched with its worker
//! stepped out of the runtime when the answer may block, its answer's body
//! sent under the stream's flow control - so one slow handler holds no
//! other stream. A connection with no stream open for
//! the read timeout is closed with a `GOAWAY`.

use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use h2::server::SendResponse;
use ureq::http;

use super::forwarded::scheme_of;
use super::framed::{
    Chunks, head_of, is_truncated, prepared, traced_body, traced_head, traced_refusal,
    traced_request,
};
use super::trace::Exchange;
use super::{Incoming, Inner, Outcome};
use crate::http::runtime;
use crate::http::{HttpVersion, Method, Status};

/// The HTTP/2 connection preface a client opens with.
const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
/// The receive window of one stream.
const STREAM_WINDOW: u32 = 4 << 20;
/// The receive window of the whole connection.
const CONNECTION_WINDOW: u32 = 16 << 20;
/// The largest frame a peer may send.
const MAX_FRAME: u32 = 64 << 10;
/// The most streams one connection holds open at once.
const MAX_STREAMS: u32 = 256;
/// The most one `DATA` frame carries.
const SEND_CHUNK: usize = 64 << 10;

/// The first byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 0x16;

/// What a connection opens with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Opening {
    /// Anything else: an HTTP/1.1 request line, or nothing at all.
    Http1,
    /// The HTTP/2 preface.
    Http2,
    /// A TLS record.
    Tls,
}

/// What `stream` opens with, read without taking a byte off it, within
/// `timeout`.
pub(super) fn opening(stream: &TcpStream, timeout: Duration) -> Opening {
    let deadline = Instant::now() + timeout;
    let mut seen = [0_u8; PREFACE.len()];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return Opening::Http1;
        }
        let Ok(peeked) = stream.peek(&mut seen) else {
            return Opening::Http1;
        };
        if peeked > 0 && seen[0] == TLS_HANDSHAKE {
            return Opening::Tls;
        }
        if peeked == 0 || seen[..peeked] != PREFACE[..peeked] {
            return Opening::Http1;
        }
        if peeked == PREFACE.len() {
            return Opening::Http2;
        }
        // Part of the preface is in: the rest is a packet behind it.
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Serve `stream`, which opened with the preface - or, given `tls`, with a
/// TLS record - as HTTP/2 until it closes.
pub(super) fn serve(inner: &Arc<Inner>, stream: TcpStream, tls: Option<Arc<rustls::ServerConfig>>) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let Ok(handle) = runtime::handle() else {
        return;
    };
    let inner = Arc::clone(inner);
    let read_timeout = inner.options.read_timeout;
    let peer = stream.peer_addr().ok();
    let secure = tls.is_some();
    // The connection is driven by a runtime task, where its socket's
    // readiness lands, so no frame crosses a thread to be read or written;
    // this thread waits on it, which is what counts it against the cap.
    let driven = handle.spawn(async move {
        let Ok(io) = tokio::net::TcpStream::from_std(stream) else {
            return;
        };
        match tls {
            None => drive(&inner, io, peer, secure).await,
            Some(config) => {
                let accepted = tokio::time::timeout(
                    read_timeout,
                    tokio_rustls::TlsAcceptor::from(config).accept(io),
                )
                .await;
                if let Ok(Ok(io)) = accepted {
                    drive(&inner, io, peer, secure).await;
                }
            }
        }
    });
    let _ = runtime::wait(driven);
}

/// Serve HTTP/2 over `io` - from `peer`, over TLS when `secure` - until the
/// connection closes.
async fn drive<T>(inner: &Arc<Inner>, io: T, peer: Option<std::net::SocketAddr>, secure: bool)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let options = &inner.options;
    let (read_timeout, max_head) = (options.read_timeout, options.max_head_size);
    let handshake = h2::server::Builder::new()
        .initial_window_size(STREAM_WINDOW)
        .initial_connection_window_size(CONNECTION_WINDOW)
        .max_frame_size(MAX_FRAME)
        .max_concurrent_streams(MAX_STREAMS)
        .max_header_list_size(u32::try_from(max_head).unwrap_or(u32::MAX))
        .handshake::<_, Bytes>(io);
    let Ok(Ok(mut connection)) = tokio::time::timeout(read_timeout, handshake).await else {
        return;
    };
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        let accepted = tokio::time::timeout(read_timeout, connection.accept()).await;
        match accepted {
            Ok(Some(Ok((request, respond)))) => {
                open.fetch_add(1, Ordering::AcqRel);
                let (inner, open) = (Arc::clone(inner), Arc::clone(&open));
                tokio::spawn(async move {
                    answer(&inner, request, respond, peer, secure).await;
                    open.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Ok(Some(Err(_)) | None) => return,
            // Quiet for the read timeout with nothing open: say so and
            // let the peer finish what it has in flight.
            Err(_) if open.load(Ordering::Acquire) == 0 => {
                connection.graceful_shutdown();
                // A peer that never acknowledges the shutdown is let go of at
                // the read timeout, as one that never sends is.
                while let Ok(Some(Ok((request, respond)))) =
                    tokio::time::timeout(read_timeout, connection.accept()).await
                {
                    let inner = Arc::clone(inner);
                    tokio::spawn(
                        async move { answer(&inner, request, respond, peer, secure).await },
                    );
                }
                return;
            }
            Err(_) => {}
        }
    }
}

/// Answer one stream of a connection from `peer`, over TLS when `secure`.
async fn answer(
    inner: &Arc<Inner>,
    request: http::Request<h2::RecvStream>,
    mut respond: SendResponse<Bytes>,
    peer: Option<std::net::SocketAddr>,
    secure: bool,
) {
    let (parts, mut recv) = request.into_parts();
    let head = match head_of(&parts, HttpVersion::Http2) {
        Ok(head) => head,
        Err((status, text)) => {
            let exchange = traced_refusal(inner, &parts, HttpVersion::Http2);
            return refuse(&mut respond, inner, status, &text, exchange);
        }
    };
    let (max, read_timeout) = (inner.options.max_body_size, inner.options.read_timeout);
    // Each frame is waited for within the read timeout, as each read of an
    // HTTP/1.1 body is: a slow body that keeps moving is read whole.
    let collected = async {
        let mut body = Vec::new();
        loop {
            let next = tokio::time::timeout(read_timeout, recv.data())
                .await
                .map_err(|_| None)?;
            let Some(chunk) = next else {
                return Ok(body);
            };
            let chunk = chunk.map_err(|_| None)?;
            let _ = recv.flow_control().release_capacity(chunk.len());
            if body.len() as u64 + chunk.len() as u64 > max {
                return Err(Some(Status::new(413).unwrap_or(Status::BAD_REQUEST)));
            }
            body.extend_from_slice(&chunk);
        }
    }
    .await;
    let body = match collected {
        Ok(body) => body,
        Err(Some(status)) => {
            let exchange = traced_refusal(inner, &parts, HttpVersion::Http2);
            return refuse(
                &mut respond,
                inner,
                status,
                &format!("request body longer than {max} bytes"),
                exchange,
            );
        }
        // The peer reset the stream, or went quiet part way.
        Err(None) => return respond.send_reset(h2::Reason::CANCEL),
    };
    let head_only = head.method == Method::Head;
    // A handler, a mounted leaf's reads or a pause may block, so the worker
    // steps out of the runtime for them rather than handing the request to
    // another thread and back; a fixed answer is made where it is.
    let incoming = Incoming {
        head,
        body,
        peer,
        secure,
        scheme: parts.uri.scheme_str().and_then(scheme_of),
    };
    let mut exchange = traced_request(inner, &incoming);
    let routed = inner.route_of(&incoming);
    let dispatched = if routed.may_block() {
        tokio::task::block_in_place(|| inner.answer_routed(routed, incoming))
    } else {
        inner.answer_routed(routed, incoming)
    };
    let Outcome::Answer { answer, cut } = dispatched else {
        // Closed without an answer, as a fault asked: the stream ends with
        // no response.
        return respond.send_reset(h2::Reason::INTERNAL_ERROR);
    };
    let truncated = is_truncated(&answer.body, cut);
    let (response, body) = prepared(answer, head_only, inner.options.server_header());
    if let Some(exchange) = exchange.as_mut() {
        traced_head(exchange, &response, HttpVersion::Http2);
    }
    let Ok(mut send) = respond.send_response(response, body.is_none()) else {
        return;
    };
    let Some(body) = body else {
        return;
    };
    let write_timeout = inner.options.write_timeout;
    let mut chunks = Chunks::of(body, cut, exchange.take());
    while let Some(chunk) = chunks.next(SEND_CHUNK).await {
        let Ok(mut chunk) = chunk else {
            // The leaf ended short of its announced length.
            return send.send_reset(h2::Reason::INTERNAL_ERROR);
        };
        while !chunk.is_empty() {
            send.reserve_capacity(chunk.len().min(SEND_CHUNK));
            // A peer that stops taking the answer is let go of at the write
            // timeout, as an HTTP/1.1 one is; the leaf's reader ends with
            // the chunks it can no longer hand over.
            let polled = tokio::time::timeout(
                write_timeout,
                std::future::poll_fn(|context| send.poll_capacity(context)),
            )
            .await;
            let granted = match polled {
                Ok(Some(Ok(0))) => continue,
                Ok(Some(Ok(granted))) => granted.min(chunk.len()),
                Err(_) => return send.send_reset(h2::Reason::CANCEL),
                Ok(Some(Err(_)) | None) => return,
            };
            if send.send_data(chunk.split_to(granted), false).is_err() {
                return;
            }
        }
    }
    if truncated {
        send.send_reset(h2::Reason::INTERNAL_ERROR);
    } else {
        let _ = send.send_data(Bytes::new(), true);
    }
}

/// Answer `status` with `text` and end the stream.
fn refuse(
    respond: &mut SendResponse<Bytes>,
    inner: &Inner,
    status: Status,
    text: &str,
    mut exchange: Option<Exchange>,
) {
    let (response, body) = prepared(
        super::Answer::text(status, text),
        false,
        inner.options.server_header(),
    );
    if let Some(exchange) = exchange.as_mut() {
        traced_head(exchange, &response, HttpVersion::Http2);
    }
    if let Ok(mut send) = respond.send_response(response, body.is_none()) {
        if let Some(super::AnswerBody::Bytes(body)) = body {
            traced_body(exchange, body.as_bytes());
            let _ = send.send_data(Bytes::copy_from_slice(body.as_bytes()), true);
        }
    }
}
