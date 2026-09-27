//! HTTP/3 on the UDP port beside the server's TCP one.
//!
//! QUIC carries TLS 1.3 and cannot be spoken in the clear, so a server that
//! answers HTTP/3 signs a certificate for itself when it binds - valid for
//! `localhost`, `127.0.0.1` and `::1` - and hands it out as PEM
//! ([`Server::certificate`](super::Server::certificate)) for a client to
//! trust. Its HTTP/1.1 and HTTP/2 answers advertise the port in `Alt-Svc`,
//! so a client that negotiates reaches it over QUIC from its next request.
//!
//! Each connection runs as a task on the private runtime and each stream as
//! one of its own, answered as an HTTP/2 stream is: collected within the
//! body bound, dispatched in place or with its worker stepped out of the
//! runtime when the answer may block, its body sent chunk by chunk;
//! a cut answer's stream is reset where it stops. A connection quiet for the
//! read timeout is closed by QUIC's idle timer; connections past
//! [`ServerOptions::max_connections`](super::ServerOptions::max_connections)
//! are refused before their handshake.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::{Buf, Bytes};
use h3::server::RequestStream;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use super::framed::{
    Chunks, head_of, is_truncated, prepared, traced_body, traced_head, traced_refusal,
    traced_request,
};
use super::trace::Exchange;
use super::{Incoming, Inner, Outcome};
use crate::http::runtime;
use crate::http::tls::ring;
use crate::http::{HttpVersion, Method, Status};
use crate::{Error, Result};

/// The receive window of one stream.
const STREAM_WINDOW: u32 = 4 << 20;
/// The receive window of the whole connection.
const CONNECTION_WINDOW: u32 = 16 << 20;
/// The most one write carries.
const SEND_CHUNK: usize = 64 << 10;

type Stream = RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>;

/// The QUIC side of a server: its endpoint, the certificate it answers
/// under, and the port it advertises.
pub(super) struct Listener {
    endpoint: quinn::Endpoint,
    pub(super) certificate: String,
    pub(super) port: u16,
    /// The same certificate for TLS on the TCP port, offering `h2`.
    pub(super) tcp: Arc<rustls::ServerConfig>,
    /// Wakes the accept loop when the server stops.
    stop: Arc<tokio::sync::Notify>,
}

impl Listener {
    /// Bind QUIC on `address`, the TCP listener's own, under a certificate
    /// signed for the loopback names; `idle` closes a quiet connection.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the port cannot be bound or the certificate made.
    pub(super) fn bind(address: SocketAddr, idle: std::time::Duration) -> Result<Self> {
        let failed = |what: &str, error: &dyn std::fmt::Display| {
            Error::Io(std::io::Error::other(format!("HTTP/3 {what}: {error}")))
        };
        let certified = rcgen::generate_simple_self_signed(vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
        ])
        .map_err(|error| failed("certificate", &error))?;
        let chain: Vec<CertificateDer<'static>> = vec![certified.cert.der().clone()];
        let key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()));
        let mut tcp = rustls::ServerConfig::builder_with_provider(ring())
            .with_safe_default_protocol_versions()
            .map_err(|error| failed("TLS", &error))?
            .with_no_client_auth()
            .with_single_cert(chain.clone(), key.clone_key())
            .map_err(|error| failed("TLS", &error))?;
        tcp.alpn_protocols = vec![b"h2".to_vec()];
        let mut tls = rustls::ServerConfig::builder_with_provider(ring())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| failed("TLS", &error))?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(|error| failed("TLS", &error))?;
        tls.alpn_protocols = vec![b"h3".to_vec()];
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
            .map_err(|error| failed("TLS", &error))?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        let mut transport = quinn::TransportConfig::default();
        transport
            .stream_receive_window(STREAM_WINDOW.into())
            .receive_window(CONNECTION_WINDOW.into())
            .max_idle_timeout(quinn::IdleTimeout::try_from(idle).ok());
        config.transport_config(Arc::new(transport));
        let endpoint = runtime::wait(async { quinn::Endpoint::server(config, address) })
            .map_err(Error::Io)?
            .map_err(Error::Io)?;
        Ok(Self {
            endpoint,
            certificate: certified.cert.pem(),
            port: address.port(),
            tcp: Arc::new(tcp),
            stop: Arc::new(tokio::sync::Notify::new()),
        })
    }

    /// Accept connections for `inner` until the server stops, each counted
    /// against the same `max_connections` as the TCP port's.
    pub(super) fn accept(&self, inner: &Arc<Inner>) -> Result<()> {
        let (endpoint, stop, inner) = (
            self.endpoint.clone(),
            Arc::clone(&self.stop),
            Arc::clone(inner),
        );
        runtime::handle().map_err(Error::Io)?.spawn(async move {
            loop {
                let mut accepting = std::pin::pin!(endpoint.accept());
                let mut stopping = std::pin::pin!(stop.notified());
                let incoming = std::future::poll_fn(|context| {
                    if stopping.as_mut().poll(context).is_ready() {
                        return std::task::Poll::Ready(None);
                    }
                    accepting.as_mut().poll(context)
                })
                .await;
                let Some(incoming) = incoming else {
                    return;
                };
                inner.connections.fetch_add(1, Ordering::Relaxed);
                let admitted = inner
                    .live
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                        (open < inner.options.max_connections).then_some(open + 1)
                    })
                    .is_ok();
                if !admitted {
                    incoming.refuse();
                    continue;
                }
                let inner = Arc::clone(&inner);
                tokio::spawn(async move {
                    serve(Arc::clone(&inner), incoming).await;
                    inner.live.fetch_sub(1, Ordering::AcqRel);
                });
            }
        });
        Ok(())
    }

    /// Stop accepting: new connections are refused, and the ones being
    /// served finish their requests, as the TCP port's do.
    pub(super) fn close(&self) {
        self.endpoint.set_server_config(None);
        self.stop.notify_one();
    }
}

/// Serve one connection until it closes.
async fn serve(inner: Arc<Inner>, incoming: quinn::Incoming) {
    let Ok(connection) = incoming.await else {
        return;
    };
    let Ok(mut session) =
        h3::server::Connection::<_, Bytes>::new(h3_quinn::Connection::new(connection)).await
    else {
        return;
    };
    while let Ok(Some(resolver)) = session.accept().await {
        let inner = Arc::clone(&inner);
        tokio::spawn(async move {
            if let Ok((request, stream)) = resolver.resolve_request().await {
                answer(&inner, request, stream).await;
            }
        });
    }
}

/// Answer one stream.
async fn answer(inner: &Arc<Inner>, request: ureq::http::Request<()>, mut stream: Stream) {
    let (parts, ()) = request.into_parts();
    let head = match head_of(&parts, HttpVersion::Http3) {
        Ok(head) => head,
        Err((status, text)) => {
            let exchange = traced_refusal(inner, &parts, HttpVersion::Http3);
            return refuse(&mut stream, inner, status, &text, exchange).await;
        }
    };
    let (max, read_timeout) = (inner.options.max_body_size, inner.options.read_timeout);
    // Each frame is waited for within the read timeout, as each read of an
    // HTTP/1.1 body is: a slow body that keeps moving is read whole.
    let collected = async {
        let mut body = Vec::new();
        loop {
            let next = tokio::time::timeout(read_timeout, stream.recv_data())
                .await
                .map_err(|_| None)?
                .map_err(|_| None)?;
            let Some(mut chunk) = next else {
                return Ok(body);
            };
            if body.len() as u64 + chunk.remaining() as u64 > max {
                return Err(Some(Status::new(413).unwrap_or(Status::BAD_REQUEST)));
            }
            while chunk.has_remaining() {
                let piece = chunk.chunk();
                body.extend_from_slice(piece);
                let taken = piece.len();
                chunk.advance(taken);
            }
        }
    }
    .await;
    let body = match collected {
        Ok(body) => body,
        Err(Some(status)) => {
            let exchange = traced_refusal(inner, &parts, HttpVersion::Http3);
            return refuse(
                &mut stream,
                inner,
                status,
                &format!("request body longer than {max} bytes"),
                exchange,
            )
            .await;
        }
        Err(None) => {
            return stream.stop_stream(h3::error::Code::H3_REQUEST_INCOMPLETE);
        }
    };
    let head_only = head.method == Method::Head;
    // A handler, a mounted leaf's reads or a pause may block, so the worker
    // steps out of the runtime for them rather than handing the request to
    // another thread and back; a fixed answer is made where it is.
    let incoming = Incoming { head, body };
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
        return stream.stop_stream(h3::error::Code::H3_INTERNAL_ERROR);
    };
    let truncated = is_truncated(&answer.body, cut);
    let (response, body) = prepared(answer, head_only, inner.options.server_header());
    if let Some(exchange) = exchange.as_mut() {
        traced_head(exchange, &response, HttpVersion::Http3);
    }
    // A peer that stops taking the answer is let go of at the write timeout,
    // as an HTTP/1.1 one is; the leaf's reader ends with the chunks it can
    // no longer hand over.
    let write_timeout = inner.options.write_timeout;
    let sent = tokio::time::timeout(write_timeout, stream.send_response(response)).await;
    if !matches!(sent, Ok(Ok(()))) {
        return stream.stop_stream(h3::error::Code::H3_REQUEST_CANCELLED);
    }
    if let Some(body) = body {
        let mut chunks = Chunks::of(body, cut, exchange.take());
        while let Some(chunk) = chunks.next(SEND_CHUNK).await {
            let Ok(chunk) = chunk else {
                return stream.stop_stream(h3::error::Code::H3_INTERNAL_ERROR);
            };
            let sent = tokio::time::timeout(write_timeout, stream.send_data(chunk)).await;
            if !matches!(sent, Ok(Ok(()))) {
                return stream.stop_stream(h3::error::Code::H3_REQUEST_CANCELLED);
            }
        }
    }
    if truncated {
        stream.stop_stream(h3::error::Code::H3_INTERNAL_ERROR);
    } else {
        let _ = tokio::time::timeout(write_timeout, stream.finish()).await;
    }
}

/// Answer `status` with `text` and end the stream.
async fn refuse(
    stream: &mut Stream,
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
        traced_head(exchange, &response, HttpVersion::Http3);
    }
    if stream.send_response(response).await.is_err() {
        return;
    }
    if let Some(super::AnswerBody::Bytes(body)) = body {
        traced_body(exchange, body.as_bytes());
        let _ = stream
            .send_data(Bytes::copy_from_slice(body.as_bytes()))
            .await;
    }
    let _ = stream.finish().await;
}
