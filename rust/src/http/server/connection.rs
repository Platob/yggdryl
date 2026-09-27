//! One connection: the loop that reads a request off the socket, frames its
//! body, hands it to the server and writes the answer, until the peer
//! closes, a request is malformed, or a fault severs it.
//!
//! The head is read up to the blank line within [`ServerOptions::max_head_size`]
//! and parsed by the wire grammar; the body is framed by `Content-Length`,
//! or by `Transfer-Encoding: chunked` through [`decode_chunked`], within
//! [`ServerOptions::max_body_size`]. What the head alone refuses a body on -
//! its framing, a stated length past the bound - is refused before a byte
//! of it is read, and only then does an HTTP/1.1 `Expect: 100-continue` earn
//! its `100 Continue` - never for a body of no bytes; a chunked body states
//! no length, so one past the bound is refused `413` as it is read, after
//! the `100`. Any other expectation is answered `417`. Every answer carries
//! `Server` and `Date`; a written body goes out chunked. While the server
//! traces, the bytes a request consumed and every byte written back are
//! copied into its exchange.

use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use super::trace::{Exchange, Trace};
use super::{ALLOW, Answer, AnswerBody, BodyWriter, Incoming, Inner, Mounted, Outcome, Source};
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

/// The connection's reader, keeping a copy of every byte a request consumes
/// while the server traces: what `consume` takes, never what was read ahead,
/// so a pipelined request's bytes are its own.
struct Wire {
    reader: BufReader<Deadline>,
    tracing: bool,
    /// The current request's bytes so far, while tracing.
    kept: Option<Vec<u8>>,
}

impl Wire {
    /// Start keeping a new request's bytes, when the server traces.
    fn keep(&mut self) {
        self.kept = self.tracing.then(Vec::new);
    }

    /// Whether a request began: a byte of it past the empty lines a peer
    /// may send before its request line.
    fn began(&self) -> bool {
        self.kept
            .as_ref()
            .is_some_and(|kept| kept.iter().any(|byte| !matches!(byte, b'\r' | b'\n')))
    }

    /// Hand the request's bytes, as far as they were read, to its trace.
    fn record(&mut self, exchange: Option<&mut Exchange>) {
        if let (Some(exchange), Some(kept)) = (exchange, self.kept.take()) {
            exchange.request(&kept);
        }
    }

    /// The socket, every byte written to it copied into `exchange`.
    fn tee<'a>(&'a mut self, exchange: Option<&'a mut Exchange>) -> Tee<'a> {
        Tee {
            stream: &mut self.reader.get_mut().stream,
            exchange,
        }
    }

    /// `100 Continue`, inviting the body the head announced.
    fn invite(&mut self, exchange: Option<&mut Exchange>) -> io::Result<()> {
        self.tee(exchange)
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
    }

    /// The connection ends with no answer - the peer gone or quiet, a fault
    /// closing it - and what the request sent is traced as far as it went.
    fn close(&mut self, mut exchange: Option<Exchange>) {
        self.record(exchange.as_mut());
        if let Some(exchange) = exchange {
            exchange.finish();
        }
    }

    /// Answer `status` with `text` and close, the request as far as it was
    /// read and the refusal both traced.
    fn refuse(&mut self, mut exchange: Option<Exchange>, status: Status, text: &str, server: &str) {
        self.record(exchange.as_mut());
        refuse(&mut self.tee(exchange.as_mut()), status, text, server);
        if let Some(exchange) = exchange {
            exchange.finish();
        }
    }
}

impl Read for Wire {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(buffer)?;
        if let Some(kept) = &mut self.kept {
            kept.extend_from_slice(&buffer[..read]);
        }
        Ok(read)
    }
}

impl BufRead for Wire {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.reader.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        if let Some(kept) = &mut self.kept {
            let buffered = self.reader.buffer();
            kept.extend_from_slice(&buffered[..amount.min(buffered.len())]);
        }
        self.reader.consume(amount);
    }
}

/// The socket, every byte written to it copied into the exchange's trace.
struct Tee<'a> {
    stream: &'a mut TcpStream,
    exchange: Option<&'a mut Exchange>,
}

impl Write for Tee<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.stream.write(buffer)?;
        if let Some(exchange) = self.exchange.as_deref_mut() {
            exchange.response(&buffer[..written]);
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// A writer taking at most `limit` bytes - a fault's cut - and refusing the
/// rest, so the writer feeding it stops where the cut is.
pub(super) struct Cut<W> {
    pub(super) inner: W,
    remaining: Option<u64>,
    /// Whether the cut was met.
    pub(super) reached: bool,
}

impl<W> Cut<W> {
    pub(super) fn new(inner: W, limit: Option<u64>) -> Self {
        Self {
            inner,
            remaining: limit,
            reached: false,
        }
    }
}

impl<W: Write> Write for Cut<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(remaining) = self.remaining else {
            return self.inner.write(buffer);
        };
        if remaining == 0 {
            self.reached = true;
            return Err(io::ErrorKind::WriteZero.into());
        }
        let take = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let written = self.inner.write(&buffer[..take])?;
        self.remaining = Some(remaining - written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Serve `stream` until it closes, is severed, or a request is malformed.
pub(super) fn serve(inner: &std::sync::Arc<Inner>, stream: TcpStream) {
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
    #[cfg(feature = "http2")]
    match super::h2::opening(&stream, options.read_timeout) {
        super::h2::Opening::Http2 => return super::h2::serve(inner, stream, None),
        super::h2::Opening::Tls => {
            #[cfg(feature = "http3")]
            if let Some(tls) = inner.tls() {
                return super::h2::serve(inner, stream, Some(tls));
            }
            // No certificate to answer TLS with: nothing this server reads.
            return;
        }
        super::h2::Opening::Http1 => {}
    }
    let mut wire = Wire {
        reader: BufReader::new(Deadline::new(stream, options.read_timeout)),
        tracing: inner.trace.is_some(),
        kept: None,
    };
    let server = options.server_header();
    loop {
        wire.keep();
        wire.reader.get_mut().until = Some(Instant::now() + options.read_timeout);
        let head = read_head(&mut wire, options.max_head_size);
        wire.reader.get_mut().until = None;
        // An exchange begins with the first byte of a request: a connection
        // that goes away before sending one asked nothing and wrote nothing.
        let mut exchange = if wire.began() {
            inner.trace.as_ref().map(Trace::begin)
        } else {
            None
        };
        let head = match head {
            Ok(Some(head)) => head,
            Ok(None) | Err(Refusal::Closed) => return wire.close(exchange),
            Err(Refusal::Malformed(status, text)) => {
                return wire.refuse(exchange, status, &text, server);
            }
        };
        let mut head = match parse_request_head(&head) {
            Ok(head) => head,
            Err(error) => {
                return wire.refuse(exchange, Status::BAD_REQUEST, &error.to_string(), server);
            }
        };
        if head.version.is_multiplexed() {
            // HTTP/2 opens with its preface, never a request line naming it,
            // and HTTP/3 is not spoken over TCP at all.
            let text = format!(
                "{} is not spoken in a request line; open with the HTTP/2 preface",
                head.version
            );
            let status = Status::new(505).unwrap_or(Status::BAD_REQUEST);
            return wire.refuse(exchange, status, &text, server);
        }
        if head.method == Method::Connect {
            return tunnel(inner, wire, head, exchange);
        }
        let keep_alive = options.keep_alive && keeps_alive(&head);
        let framing = match framing_of(head.version, &head.headers, options.max_body_size) {
            Ok(framing) => framing,
            Err(Refusal::Closed) => return wire.close(exchange),
            Err(Refusal::Malformed(status, text)) => {
                return wire.refuse(exchange, status, &text, server);
            }
        };
        match expectation(&head) {
            Expectation::Continue if framing.carries_body() => {
                if wire.invite(exchange.as_mut()).is_err() {
                    return wire.close(exchange);
                }
            }
            Expectation::Unmet(value) => {
                let text =
                    format!("this server meets no expectation but 100-continue; got {value:?}");
                let status = Status::new(417).unwrap_or(Status::BAD_REQUEST);
                return wire.refuse(exchange, status, &text, server);
            }
            Expectation::Continue | Expectation::None => {}
        }
        let body = match read_framed(&mut wire, framing, &mut head.headers, options.max_body_size) {
            Ok(body) => body,
            Err(Refusal::Closed) => return wire.close(exchange),
            Err(Refusal::Malformed(status, text)) => {
                return wire.refuse(exchange, status, &text, server);
            }
        };
        wire.record(exchange.as_mut());
        let head_only = head.method == Method::Head;
        match inner.dispatch(Incoming { head, body }) {
            Outcome::Close => return wire.close(exchange),
            Outcome::Answer { answer, cut } => {
                let close = !keep_alive || cut.is_some();
                let written = write_answer(
                    &mut wire.tee(exchange.as_mut()),
                    head_only,
                    answer,
                    close,
                    cut,
                    server,
                );
                // A trace that could not be written ends the connection
                // after its answer, rather than go on past the gap.
                let traced = exchange.is_none_or(Exchange::finish);
                if written.is_err() || close || !traced {
                    return;
                }
            }
        }
    }
}

/// Answer a `CONNECT`: `405` unless the options tunnel, else `502` when no
/// address the target resolves to can be reached, else `200` and the bytes
/// piped both ways - the connection is the tunnel's from then on.
///
/// A side that closes half-closes the other, so each direction ends on its
/// own; a tunnel quiet both ways for the read timeout, or failing either way,
/// is closed whole, so neither direction outlives it and its connection slot
/// is given back.
fn tunnel(inner: &Inner, mut wire: Wire, head: RequestHead, mut exchange: Option<Exchange>) {
    let options = &inner.options;
    wire.record(exchange.as_mut());
    let incoming = Incoming {
        head,
        body: Vec::new(),
    };
    let target = incoming.head.target.clone();
    if !options.tunnel {
        inner.record(&incoming, "", Vec::new(), Status::METHOD_NOT_ALLOWED);
        // A 405 names the methods that are allowed (RFC 9110 15.5.6).
        let answer = Answer::text(
            Status::METHOD_NOT_ALLOWED,
            "this server does not tunnel CONNECT",
        )
        .with_header("allow", ALLOW);
        let _ = write_answer(
            &mut wire.tee(exchange.as_mut()),
            false,
            answer,
            true,
            None,
            options.server_header(),
        );
        if let Some(exchange) = exchange {
            exchange.finish();
        }
        return;
    }
    // Every address the name resolves to, in turn, as a client dialing it
    // would: a dual-stack name whose first family is unreachable still
    // tunnels through its second.
    let upstream = std::net::ToSocketAddrs::to_socket_addrs(&target)
        .ok()
        .and_then(|mut addresses| {
            addresses
                .find_map(|address| TcpStream::connect_timeout(&address, options.read_timeout).ok())
        })
        .filter(|upstream| {
            upstream.set_nodelay(true).is_ok()
                && upstream
                    .set_read_timeout(Some(options.read_timeout))
                    .is_ok()
                && upstream
                    .set_write_timeout(Some(options.write_timeout))
                    .is_ok()
        });
    let Some(upstream) = upstream else {
        inner.record(&incoming, "", Vec::new(), Status::BAD_GATEWAY);
        refuse(
            &mut wire.tee(exchange.as_mut()),
            Status::BAD_GATEWAY,
            &format!("could not reach {target}"),
            options.server_header(),
        );
        if let Some(exchange) = exchange {
            exchange.finish();
        }
        return;
    };
    inner.record(&incoming, "", Vec::new(), Status::OK);
    let established = wire
        .tee(exchange.as_mut())
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
    // The bytes past the 200 are the tunnel's, not an exchange.
    if let Some(exchange) = exchange {
        exchange.finish();
    }
    if established.is_err() {
        return;
    }
    let mut reader = wire.reader;
    let client = &mut reader.get_mut().stream;
    let (Ok(upstream_reader), Ok(client_writer)) = (upstream.try_clone(), client.try_clone())
    else {
        return;
    };
    let quiet = Quiet::new(options.read_timeout);
    let back = {
        let quiet = quiet.clone();
        std::thread::Builder::new()
            .name("yggdryl-http-tunnel".to_owned())
            .spawn(move || pipe(upstream_reader, client_writer, &quiet))
    };
    let Ok(back) = back else {
        let _ = upstream.shutdown(Shutdown::Both);
        return;
    };
    // The reader still holds what the client sent past the head.
    let upstream_writer = upstream;
    pipe(reader, upstream_writer, &quiet);
    let _ = back.join();
}

/// When a tunnel last moved a byte either way, shared by its two directions
/// so a direction waiting on a quiet peer knows whether the other is busy.
#[derive(Clone)]
struct Quiet {
    since: Instant,
    /// Milliseconds after `since` of the last byte either way.
    last: std::sync::Arc<AtomicU64>,
    idle: Duration,
}

impl Quiet {
    fn new(idle: Duration) -> Self {
        Self {
            since: Instant::now(),
            last: std::sync::Arc::new(AtomicU64::new(0)),
            idle,
        }
    }

    fn elapsed(&self) -> u64 {
        u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn moved(&self) {
        self.last.store(self.elapsed(), Ordering::Relaxed);
    }

    fn is_idle(&self) -> bool {
        let quiet_for = self
            .elapsed()
            .saturating_sub(self.last.load(Ordering::Relaxed));
        u128::from(quiet_for) >= self.idle.as_millis()
    }
}

/// One direction of a tunnel: `from` copied into `to` until `from` closes,
/// then `to` half-closed; a failure either way, or both ways quiet for the
/// idle bound, closes `to` whole, which ends the other direction too.
fn pipe(mut from: impl Read, mut to: TcpStream, quiet: &Quiet) {
    let mut buffer = vec![0; DEFAULT_STREAM_BATCH_SIZE.min(64 * 1024)];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => {
                let _ = to.shutdown(Shutdown::Write);
                return;
            }
            Ok(read) => {
                if to.write_all(&buffer[..read]).is_err() {
                    break;
                }
                quiet.moved();
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) && !quiet.is_idle() => {}
            Err(_) => break,
        }
    }
    let _ = to.shutdown(Shutdown::Both);
}

/// The bytes of one request head up to and including the empty line, or
/// `None` at the end of the stream before any byte of one; empty lines
/// before the request line are skipped (RFC 9112 2.2).
fn read_head(
    reader: &mut Wire,
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
        let limit = max_head_size
            .saturating_add(1)
            .saturating_sub(before + skipped);
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
        HttpVersion::Http11 | HttpVersion::Http2 | HttpVersion::Http3 => !wants("close"),
    }
}

/// How a request's body is framed, read off its head alone.
#[derive(Clone, Copy)]
enum Framing {
    Chunked,
    Length(u64),
}

impl Framing {
    /// Whether the body may carry a byte: a chunked one may, whatever it
    /// turns out to hold.
    fn carries_body(self) -> bool {
        !matches!(self, Self::Length(0))
    }
}

/// What a request's `Expect` asks of the server.
enum Expectation {
    None,
    /// `100-continue` on HTTP/1.1.
    Continue,
    /// Anything else, answered `417`.
    Unmet(String),
}

/// The request's expectation: `100-continue` is met on HTTP/1.1 and
/// ignored on HTTP/1.0, which has no interim answers (RFC 9110 10.1.1); any
/// other is one this server does not meet.
fn expectation(head: &RequestHead) -> Expectation {
    let Some(value) = head.headers.get("expect") else {
        return Expectation::None;
    };
    if value.trim().eq_ignore_ascii_case("100-continue") {
        return if head.version == HttpVersion::Http11 {
            Expectation::Continue
        } else {
            Expectation::None
        };
    }
    Expectation::Unmet(value.to_owned())
}

/// The framing the head states: chunked, else `Content-Length`, else none.
///
/// Everything a body is refused on is refused here, from the head, before a
/// byte of the body is read or a `100 Continue` invites it. A
/// `Transfer-Encoding` on an HTTP/1.0 request, or one whose last coding is
/// not `chunked`, frames nothing this server can trust and is refused (RFC
/// 9112 6.1 and 6.3): reading it any other way is how a proxy in front and
/// this server come to disagree on where a request ends. A stated length
/// past `max_body_size` is refused as the body would be.
fn framing_of(
    version: HttpVersion,
    headers: &Headers,
    max_body_size: u64,
) -> std::result::Result<Framing, Refusal> {
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
        return Ok(Framing::Chunked);
    }
    let length = match headers.content_length() {
        Ok(length) => length.unwrap_or(0),
        Err(error) => return Err(Refusal::Malformed(Status::BAD_REQUEST, error.to_string())),
    };
    if length > max_body_size {
        return Err(too_large(max_body_size));
    }
    Ok(Framing::Length(length))
}

/// The body under `framing`, within `max_body_size`.
///
/// A chunked body leaves the headers as a decoded message carries them: the
/// trailers folded in, `content-length` the decoded length, no
/// `transfer-encoding`.
fn read_framed(
    reader: &mut Wire,
    framing: Framing,
    headers: &mut Headers,
    max_body_size: u64,
) -> std::result::Result<Vec<u8>, Refusal> {
    let length = match framing {
        Framing::Length(length) => length,
        Framing::Chunked => {
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
                return Err(too_large(max_body_size));
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
    };
    let mut body = vec![0; usize::try_from(length).map_err(|_| too_large(max_body_size))?];
    reader.read_exact(&mut body)?;
    Ok(body)
}

/// A body longer than `max_body_size`: `413`.
fn too_large(max_body_size: u64) -> Refusal {
    Refusal::Malformed(
        Status::new(413).unwrap_or(Status::BAD_REQUEST),
        format!("request body longer than {max_body_size} bytes"),
    )
}

/// Answer `status` with `text` and close; a failure to write is the peer's.
fn refuse(stream: &mut impl Write, status: Status, text: &str, server: &str) {
    let answer = Answer::text(status, text);
    let _ = write_answer(stream, false, answer, true, None, server);
}

/// UTC nanoseconds since the epoch.
pub(super) fn now_ns() -> i64 {
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
/// [`DEFAULT_STREAM_BATCH_SIZE`] chunks; a written body is always chunked,
/// its length unknown until it ends; otherwise, and for every streamed leaf,
/// `Content-Length` states the length. A `HEAD` declares the length its
/// `GET` would carry - none for a written body - and never a chunked
/// framing.
fn write_answer(
    stream: &mut impl Write,
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
        && match &body {
            AnswerBody::Bytes(_) => headers.transfer_encoding_chunked(),
            AnswerBody::Writer(_) => true,
            AnswerBody::Stream { .. } => false,
        };
    if !has_body {
        headers.remove("content-length");
        headers.remove("transfer-encoding");
    } else if chunked {
        // A sender states one framing, never both (RFC 9112 6.2).
        headers.remove("content-length");
        let _ = headers.insert("transfer-encoding", "chunked");
    } else {
        headers.remove("transfer-encoding");
        let length = match &body {
            AnswerBody::Bytes(bytes) => Some(bytes.len() as u64),
            AnswerBody::Stream { length, .. } => Some(*length),
            AnswerBody::Writer(_) => None,
        };
        match length {
            Some(length) => {
                let _ = headers.insert("content-length", &length.to_string());
            }
            None => {
                headers.remove("content-length");
            }
        }
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
                // A cut at or past the body's end cuts nothing: the body
                // ended whole, and says so.
                if limit == bytes.len() {
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
            stream_source(stream, source, start, limit)?;
        }
        AnswerBody::Writer(writer) => write_written(stream, writer, cut)?,
    }
    stream.flush()
}

/// Write a written body as chunks, [`DEFAULT_STREAM_BATCH_SIZE`] at a time,
/// and the last chunk once it ended whole - never when a cut was reached,
/// which leaves the transfer severed as the fault asked; a cut past the
/// body's end cuts nothing. A writer that fails severs it too: the
/// connection closes without the last chunk, so the peer reads a cut
/// transfer rather than a short body it takes for whole.
fn write_written(stream: &mut impl Write, writer: BodyWriter, cut: Option<u64>) -> io::Result<()> {
    let mut body = BufWriter::with_capacity(
        DEFAULT_STREAM_BATCH_SIZE,
        Cut::new(encode_chunked(&mut *stream), cut),
    );
    let wrote = writer(&mut body);
    let flushed = body.flush();
    if body.get_ref().reached {
        return Ok(());
    }
    if let Err(error) = wrote {
        return Err(io::Error::other(error.to_string()));
    }
    flushed?;
    let chunks = body.into_inner().map_err(io::IntoInnerError::into_error)?;
    chunks.inner.finish()?;
    Ok(())
}

/// Write `limit` bytes of `source` from `start`: the one door every version
/// streams a mounted leaf through.
pub(super) fn stream_source(
    stream: &mut impl Write,
    source: Source,
    start: u64,
    limit: u64,
) -> io::Result<()> {
    match source {
        Source::Owned(holder) => stream_leaf(stream, holder.as_ref(), start, limit),
        Source::Root(shared, version) => stream_root(stream, &shared, version, start, limit),
    }
}

/// Write `limit` bytes of the mounted holder itself from `start`, at the
/// version `version` the answer's head described, one
/// [`DEFAULT_STREAM_BATCH_SIZE`] range read per batch.
///
/// The mount's lock is taken for each read and never across a socket write:
/// a peer slow to take its answer then holds no `PUT` or `DELETE` on the
/// mount behind it. The price is one ranged read per batch where a child
/// streams from one. A write between two batches ends the body there, as a
/// severed transfer the peer resumes under `If-Range`, rather than finishing
/// it with the new version's bytes.
fn stream_root(
    stream: &mut impl Write,
    shared: &RwLock<Mounted>,
    version: u64,
    start: u64,
    limit: u64,
) -> io::Result<()> {
    let mut position = start;
    let mut remaining = limit;
    while remaining > 0 {
        let want = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(DEFAULT_STREAM_BATCH_SIZE);
        let batch = {
            let mounted = shared.read().unwrap_or_else(PoisonError::into_inner);
            if mounted.version != version {
                return Err(short_body(remaining));
            }
            mounted
                .holder
                .read_range_bytes(position, want)
                .map_err(io::Error::other)?
        };
        if batch.is_empty() {
            return Err(short_body(remaining));
        }
        let take = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(batch.len());
        stream.write_all(&batch[..take])?;
        position += take as u64;
        remaining -= take as u64;
    }
    Ok(())
}

/// The failure of a body that ends `remaining` bytes short of the length
/// its head stated: the connection closes on it, so the peer reads a
/// severed transfer rather than taking the next answer's bytes as the rest.
fn short_body(remaining: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("the body ended {remaining} bytes short of its stated length"),
    )
}

/// Write `limit` bytes of `holder` from `start`, streamed in
/// [`DEFAULT_STREAM_BATCH_SIZE`] batches; a leaf shorter than its stated
/// length ends the body short and closes the connection, which the peer
/// reads as a severed transfer.
fn stream_leaf(
    stream: &mut impl Write,
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
            return Ok(());
        }
    }
    Err(short_body(remaining))
}
