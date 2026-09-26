//! `rust/src/soap/http.rs`: the SOAP 1.1 HTTP binding - the status table
//! and the refusal that names one, one request read off a connection (its
//! line, its headers, a body framed by `Content-Length` or by chunks, every
//! bound and every refusal), the connection's persistence, and the two
//! response framings: a body of known length and a stream of chunks.

use std::cell::{Cell, RefCell};
use std::error::Error as _;
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::rc::Rc;

use yggdryl::soap::http::{
    HttpError, MAX_HEADERS, MAX_LINE, Request, Status, Version, begin_chunked, write_response,
};
use yggdryl::soap::{ACTION_HEADER, CONTENT_TYPE};

/// The body bound every test reads under unless it pins the bound itself.
const LIMIT: usize = 1 << 20;

/// The head of a chunked response, keeping the connection alive.
const CHUNKED_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/xml; charset=utf-8\r\n\
                            Transfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n";

fn read_within(wire: &[u8], max_body: usize) -> Request {
    Request::read(&mut Cursor::new(wire), max_body)
        .unwrap_or_else(|error| panic!("{}: {error}", String::from_utf8_lossy(wire)))
        .expect("a request, not a closed connection")
}

fn read(wire: &[u8]) -> Request {
    read_within(wire, LIMIT)
}

fn refused_within(wire: &[u8], max_body: usize) -> HttpError {
    Request::read(&mut Cursor::new(wire), max_body).expect_err("a refusal")
}

fn refused(wire: &[u8]) -> HttpError {
    refused_within(wire, LIMIT)
}

/// Assert that `wire` is refused with `status` for exactly `reason`, and that
/// the refusal displays as the status line's code and phrase before it.
fn assert_refused(wire: &[u8], max_body: usize, status: Status, reason: &str) {
    let error = refused_within(wire, max_body);
    assert_eq!(
        (error.status(), error.reason()),
        (status, reason),
        "{}",
        String::from_utf8_lossy(wire)
    );
    assert_eq!(
        error.to_string(),
        format!("{} {}: {reason}", status.code(), status.reason())
    );
}

/// A head with `headers` under `POST /xmla HTTP/1.1`, followed by `body`.
fn post(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = format!("POST /xmla HTTP/1.1\r\n{headers}\r\n").into_bytes();
    wire.extend_from_slice(body);
    wire
}

/// A chunked `POST` whose body is the literal chunk framing `chunks`.
fn chunked_post(chunks: &str) -> Vec<u8> {
    post("Transfer-Encoding: chunked\r\n", chunks.as_bytes())
}

/// A connection that hands over at most `step` bytes per read, so every line
/// and every body is framed across many reads.
struct Trickle {
    bytes: Vec<u8>,
    position: usize,
    step: usize,
    reads: usize,
}

impl Trickle {
    fn new(bytes: &[u8], step: usize) -> Self {
        Self {
            bytes: bytes.to_vec(),
            position: 0,
            step,
            reads: 0,
        }
    }
}

impl Read for Trickle {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let taken = available.len().min(target.len());
        target[..taken].copy_from_slice(&available[..taken]);
        self.consume(taken);
        Ok(taken)
    }
}

impl BufRead for Trickle {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.reads += 1;
        let end = (self.position + self.step).min(self.bytes.len());
        Ok(&self.bytes[self.position..end])
    }

    fn consume(&mut self, amount: usize) {
        self.position += amount;
    }
}

/// A connection whose every read fails.
struct Reset;

impl Read for Reset {
    fn read(&mut self, _target: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("the peer reset the connection"))
    }
}

impl BufRead for Reset {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("the peer reset the connection"))
    }

    fn consume(&mut self, _amount: usize) {}
}

/// A sink the test keeps a handle on, so what has gone out is visible while
/// a `ChunkedWriter` still owns it.
#[derive(Clone, Default)]
struct Sink {
    bytes: Rc<RefCell<Vec<u8>>>,
    flushes: Rc<Cell<usize>>,
    /// How many bytes had gone out when the sink was last flushed.
    flushed_len: Rc<Cell<usize>>,
}

impl Sink {
    fn bytes(&self) -> Vec<u8> {
        self.bytes.borrow().clone()
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes.set(self.flushes.get() + 1);
        self.flushed_len.set(self.bytes.borrow().len());
        Ok(())
    }
}

/// A sink that accepts `allowed` bytes and fails every write past them.
#[derive(Debug)]
struct Budget {
    allowed: usize,
    bytes: Vec<u8>,
}

impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len() + bytes.len() > self.allowed {
            return Err(io::Error::other("the socket is gone"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A response split at the blank line: the head's lines and the body.
fn split_head(response: &[u8]) -> (Vec<&str>, &[u8]) {
    let end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a blank line ends the head");
    let head = std::str::from_utf8(&response[..end]).expect("an ASCII head");
    (head.split("\r\n").collect(), &response[end + 4..])
}

/// The chunked framing decoded by hand: the size of every data chunk, and the
/// payload they carry. The body must end with the terminating chunk and the
/// empty trailer, and hold nothing past them.
fn decode_chunked(mut body: &[u8]) -> (Vec<usize>, Vec<u8>) {
    let mut sizes = Vec::new();
    let mut payload = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|pair| pair == b"\r\n")
            .expect("a size line");
        let line = std::str::from_utf8(&body[..end]).expect("an ASCII size line");
        let size = usize::from_str_radix(line, 16).expect("a hexadecimal size");
        assert_eq!(line, format!("{size:x}"), "sizes are bare lowercase hex");
        body = &body[end + 2..];
        if size == 0 {
            assert_eq!(
                body, b"\r\n",
                "the last chunk, the empty trailer, nothing more"
            );
            return (sizes, payload);
        }
        sizes.push(size);
        payload.extend_from_slice(&body[..size]);
        assert_eq!(
            &body[size..size + 2],
            b"\r\n",
            "a chunk ends with a line break"
        );
        body = &body[size + 2..];
    }
}

/// The payload of the whole chunks in `body`, which must hold whole data
/// chunks and nothing else: what a chunked response has sent before
/// `finish`.
fn sent_payload(mut body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    while !body.is_empty() {
        let end = body
            .windows(2)
            .position(|pair| pair == b"\r\n")
            .expect("a size line");
        let line = std::str::from_utf8(&body[..end]).expect("an ASCII size line");
        let size = usize::from_str_radix(line, 16).expect("a hexadecimal size");
        assert_ne!(size, 0, "no terminating chunk before finish");
        body = &body[end + 2..];
        payload.extend_from_slice(&body[..size]);
        assert_eq!(
            &body[size..size + 2],
            b"\r\n",
            "a chunk ends with a line break"
        );
        body = &body[size + 2..];
    }
    payload
}

/// The most bytes a chunked response gathers before one goes out.
const FULL_CHUNK: usize = 64 * 1024;

// --- Status and HttpError --------------------------------------------------

#[test]
fn every_status_answers_its_code_and_the_reason_phrase_http_gives_it() {
    let table = [
        (Status::Ok, 200, "OK"),
        (Status::BadRequest, 400, "Bad Request"),
        (Status::NotFound, 404, "Not Found"),
        (Status::MethodNotAllowed, 405, "Method Not Allowed"),
        (Status::LengthRequired, 411, "Length Required"),
        (Status::PayloadTooLarge, 413, "Payload Too Large"),
        (Status::UnsupportedMediaType, 415, "Unsupported Media Type"),
        (Status::InternalServerError, 500, "Internal Server Error"),
    ];
    for (status, code, reason) in table {
        assert_eq!(status.code(), code, "{status:?}");
        assert_eq!(status.reason(), reason, "{status:?}");
    }
}

#[test]
fn a_status_code_and_reason_are_constants() {
    const CODE: u16 = Status::LengthRequired.code();
    const REASON: &str = Status::LengthRequired.reason();
    assert_eq!((CODE, REASON), (411, "Length Required"));
}

#[test]
fn an_http_error_carries_its_status_and_displays_code_phrase_and_reason() {
    let error = HttpError::new(Status::LengthRequired, "no delimiter");
    assert_eq!(error.status(), Status::LengthRequired);
    assert_eq!(error.reason(), "no delimiter");
    assert_eq!(error.to_string(), "411 Length Required: no delimiter");
    assert!(error.source().is_none());

    let boxed: Box<dyn std::error::Error> = Box::new(HttpError::new(Status::Ok, String::new()));
    assert_eq!(boxed.to_string(), "200 OK: ");
}

#[test]
fn the_status_the_version_and_a_refusal_debug_as_their_names() {
    assert_eq!(format!("{:?}", Status::PayloadTooLarge), "PayloadTooLarge");
    assert_eq!(
        (
            format!("{:?}", Version::Http10),
            format!("{:?}", Version::Http11)
        ),
        ("Http10".to_owned(), "Http11".to_owned())
    );
    let debug = format!(
        "{:?}",
        HttpError::new(Status::LengthRequired, "no delimiter")
    );
    assert!(
        debug.contains("LengthRequired") && debug.contains("\"no delimiter\""),
        "{debug}"
    );
}

#[test]
fn an_io_failure_becomes_a_400_naming_the_failure() {
    let error = HttpError::from(io::Error::other("the peer reset the connection"));
    assert_eq!(error.status(), Status::BadRequest);
    assert_eq!(
        error.reason(),
        "reading the request failed: the peer reset the connection"
    );
    assert_eq!(
        error.to_string(),
        "400 Bad Request: reading the request failed: the peer reset the connection"
    );
}

// --- Request refusals -------------------------------------------------------

#[test]
fn a_connection_that_fails_to_read_is_refused_with_400_naming_the_failure() {
    let error = Request::read(&mut Reset, LIMIT).expect_err("a refusal");
    assert_eq!(error.status(), Status::BadRequest);
    assert_eq!(
        error.reason(),
        "reading the request failed: the peer reset the connection"
    );
}

#[test]
fn a_connection_that_fails_after_its_first_bytes_is_refused_with_400_naming_the_failure() {
    let chunked = "POST /xmla HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
    for head in [
        "\r\n".to_owned(),
        "POST /x".to_owned(),
        "POST /xmla HTTP/1.1\r\n".to_owned(),
        "POST /xmla HTTP/1.1\r\nHost: loc".to_owned(),
        chunked.to_owned(),
        format!("{chunked}5;ext"),
        format!("{chunked}5\r\nhello"),
        format!("{chunked}0\r\n"),
        format!("{chunked}0\r\nX-Trailer: t\r\n"),
    ] {
        let mut connection = Cursor::new(head.as_bytes()).chain(Reset);
        let error = Request::read(&mut connection, LIMIT).expect_err("a refusal");
        assert_eq!(
            (error.status(), error.reason()),
            (
                Status::BadRequest,
                "reading the request failed: the peer reset the connection"
            ),
            "{head:?}"
        );
    }
}

#[test]
fn a_request_line_that_is_not_three_space_separated_parts_is_refused_with_400() {
    for line in [
        "POST /xmla",
        "POST /xmla HTTP/1.1 extra",
        "POST  /xmla HTTP/1.1",
        "POST /xmla  HTTP/1.1",
        "POST  HTTP/1.1",
        " /xmla HTTP/1.1",
        "POST\t/xmla\tHTTP/1.1",
        "POST /xmla HTTP/1.1 ",
        "garbage",
    ] {
        let wire = format!("{line}\r\nContent-Length: 0\r\n\r\n");
        assert_refused(
            wire.as_bytes(),
            LIMIT,
            Status::BadRequest,
            &format!("expected `METHOD target HTTP/1.x` as the request line, got {line:?}"),
        );
    }
}

#[test]
fn two_blank_lines_before_the_request_line_are_refused_with_400() {
    assert_refused(
        b"\r\n\r\nGET /xmla HTTP/1.1\r\n\r\n",
        LIMIT,
        Status::BadRequest,
        "expected `METHOD target HTTP/1.x` as the request line, got \"\"",
    );
}

#[test]
fn a_version_other_than_http_1_0_or_1_1_is_refused_with_400_naming_it() {
    for version in [
        "HTTP/2.0",
        "HTTP/1.2",
        "HTTP/1",
        "http/1.1",
        "SOAP/1.1",
        "",
        "HTTP/1.1\r",
    ] {
        let wire = format!("POST /xmla {version}\r\nContent-Length: 0\r\n\r\n");
        assert_refused(
            wire.as_bytes(),
            LIMIT,
            Status::BadRequest,
            &format!("expected HTTP/1.0 or HTTP/1.1, got {version:?}"),
        );
    }
}

#[test]
fn a_line_longer_than_max_line_is_refused_with_400() {
    let reason = format!("a line longer than {MAX_LINE} bytes");
    let long_target = format!("/{}", "a".repeat(MAX_LINE));
    let long_value = "a".repeat(MAX_LINE - "X-Long: ".len() + 1);
    for terminator in ["\r\n", "\n"] {
        let request_line = format!("GET {long_target} HTTP/1.1{terminator}{terminator}");
        assert_refused(request_line.as_bytes(), LIMIT, Status::BadRequest, &reason);

        let header =
            format!("GET /xmla HTTP/1.1{terminator}X-Long: {long_value}{terminator}{terminator}");
        assert_refused(header.as_bytes(), LIMIT, Status::BadRequest, &reason);
    }
}

#[test]
fn an_endless_line_is_refused_without_reading_the_whole_stream() {
    let reason = format!("a line longer than {MAX_LINE} bytes");
    let error = Request::read(&mut BufReader::new(io::repeat(b'a')), LIMIT)
        .expect_err("an endless request line is refused");
    assert_eq!(
        (error.status(), error.reason()),
        (Status::BadRequest, reason.as_str())
    );

    let mut header = Cursor::new(&b"GET /xmla HTTP/1.1\r\nX-Endless: "[..])
        .chain(BufReader::new(io::repeat(b'a')));
    let error = Request::read(&mut header, LIMIT).expect_err("an endless header is refused");
    assert_eq!(
        (error.status(), error.reason()),
        (Status::BadRequest, reason.as_str())
    );
}

#[test]
fn a_line_one_byte_past_max_line_is_refused_however_the_reads_split_it() {
    let reason = format!("a line longer than {MAX_LINE} bytes");
    let value = "a".repeat(MAX_LINE - "X-Long: ".len() + 1);
    for terminator in ["\r\n", "\n"] {
        let wire = format!("GET /xmla HTTP/1.1{terminator}X-Long: {value}{terminator}{terminator}");
        for step in [
            1,
            2,
            3,
            7,
            4096,
            MAX_LINE,
            MAX_LINE + 1,
            MAX_LINE + 2,
            3 * MAX_LINE,
        ] {
            let error = Request::read(&mut Trickle::new(wire.as_bytes(), step), LIMIT)
                .expect_err("a refusal");
            assert_eq!(
                (error.status(), error.reason()),
                (Status::BadRequest, reason.as_str()),
                "step {step}, {terminator:?}"
            );
        }
    }
}

#[test]
fn an_endless_line_is_refused_at_most_one_read_past_the_bound() {
    let wire = vec![b'a'; 4 * MAX_LINE];
    for step in [1, 7, 4096] {
        let mut trickle = Trickle::new(&wire, step);
        let error = Request::read(&mut trickle, LIMIT).expect_err("a refusal");
        assert_eq!(
            error.reason(),
            format!("a line longer than {MAX_LINE} bytes")
        );
        assert!(
            trickle.position <= MAX_LINE + 1 + step,
            "step {step}: {} bytes read",
            trickle.position
        );
    }
}

#[test]
fn a_chunk_size_line_or_a_trailer_line_longer_than_max_line_is_refused_with_400() {
    let long = "x".repeat(MAX_LINE);
    for chunks in [
        format!("5;{long}\r\nhello\r\n0\r\n\r\n"),
        format!("0\r\nX-Trailer: {long}\r\n\r\n"),
    ] {
        assert_refused(
            &chunked_post(&chunks),
            LIMIT,
            Status::BadRequest,
            &format!("a line longer than {MAX_LINE} bytes"),
        );
    }
}

#[test]
fn a_line_that_is_not_utf8_is_refused_with_400() {
    for wire in [
        &b"POST /caf\xe9 HTTP/1.1\r\nContent-Length: 0\r\n\r\n"[..],
        &b"GET /xmla HTTP/1.1\r\nX-Name: \xff\xfe\r\n\r\n"[..],
    ] {
        assert_refused(wire, LIMIT, Status::BadRequest, "a line that is not UTF-8");
    }
}

#[test]
fn a_connection_that_closes_inside_a_line_is_refused_with_400() {
    for wire in [
        &b"POST /xmla HTT"[..],
        &b"POST /xmla HTTP/1.1\r\nHost: loc"[..],
        &b"POST /xmla HTTP/1.1\r\nHost: localhost\r"[..],
    ] {
        assert_refused(
            wire,
            LIMIT,
            Status::BadRequest,
            "the connection closed inside a line",
        );
    }
}

#[test]
fn a_connection_that_closes_inside_the_headers_is_refused_with_400() {
    for wire in [
        &b"POST /xmla HTTP/1.1\r\n"[..],
        &b"POST /xmla HTTP/1.1\r\nHost: localhost\r\n"[..],
        &b"\r\nGET /xmla HTTP/1.0\n"[..],
    ] {
        assert_refused(
            wire,
            LIMIT,
            Status::BadRequest,
            "the connection closed inside the request headers",
        );
    }
}

#[test]
fn a_header_without_a_colon_is_refused_with_400_naming_the_line() {
    for line in ["Host localhost", "Content-Length 5", "  "] {
        let wire = format!("POST /xmla HTTP/1.1\r\n{line}\r\nContent-Length: 0\r\n\r\n");
        assert_refused(
            wire.as_bytes(),
            LIMIT,
            Status::BadRequest,
            &format!("expected `Name: value` as a header, got {line:?}"),
        );
    }
}

#[test]
fn max_headers_are_accepted_and_one_more_is_refused_with_400() {
    let head = |count: usize| {
        let mut wire = String::from("GET /xmla HTTP/1.1\r\n");
        for index in 0..count {
            wire.push_str(&format!("X-Header-{index}: {index}\r\n"));
        }
        wire.push_str("\r\n");
        wire
    };
    let request = read(head(MAX_HEADERS).as_bytes());
    assert_eq!(request.headers().len(), MAX_HEADERS);
    assert_eq!(request.header("x-header-127"), Some("127"));

    assert_refused(
        head(MAX_HEADERS + 1).as_bytes(),
        LIMIT,
        Status::BadRequest,
        &format!("more than {MAX_HEADERS} request headers"),
    );
}

#[test]
fn a_body_with_neither_a_length_nor_a_coding_is_refused_with_411() {
    for method in ["POST", "PUT", "DELETE", "post", "get"] {
        let wire = format!("{method} /xmla HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert_refused(
            wire.as_bytes(),
            LIMIT,
            Status::LengthRequired,
            "a request body needs a Content-Length or the chunked transfer coding",
        );
    }
}

#[test]
fn a_content_length_that_is_not_a_byte_count_is_refused_with_400_naming_it() {
    for value in [
        "five", "-1", "", "5 5", "0x10", "5.0", "3, 5", "\u{665}", "5\u{665}",
    ] {
        let wire = post(&format!("Content-Length: {value}\r\n"), b"hello");
        assert_refused(
            &wire,
            LIMIT,
            Status::BadRequest,
            &format!("expected a byte count as Content-Length, got {value:?}"),
        );
    }
}

#[test]
fn a_content_length_past_max_body_is_refused_with_413_before_the_body_is_read() {
    // No body bytes follow the head: reading one would be refused as a
    // connection closed inside the body, not as a body too large.
    assert_refused(
        &post("Content-Length: 11\r\n", b""),
        10,
        Status::PayloadTooLarge,
        "the body is 11 bytes; at most 10 are accepted",
    );
    assert_refused(
        &post("Content-Length: 1\r\n", b"x"),
        0,
        Status::PayloadTooLarge,
        "the body is 1 bytes; at most 0 are accepted",
    );
}

#[test]
fn a_body_shorter_than_its_content_length_is_refused_with_400() {
    let error = refused(&post("Content-Length: 5\r\n", b"hel"));
    assert_eq!(error.status(), Status::BadRequest);
    assert!(
        error
            .reason()
            .starts_with("the connection closed inside a 5-byte body: "),
        "{error}"
    );
}

#[test]
fn a_failure_inside_the_body_is_refused_with_400_naming_the_failure() {
    let head = post("Content-Length: 5\r\n", b"");
    let mut connection = Cursor::new(head).chain(Reset);
    let error = Request::read(&mut connection, LIMIT).expect_err("a refusal");
    assert_eq!(error.status(), Status::BadRequest);
    assert_eq!(
        error.reason(),
        "the connection closed inside a 5-byte body: the peer reset the connection"
    );
}

#[test]
fn a_failure_inside_a_chunk_is_refused_with_400_naming_the_failure() {
    let mut connection = Cursor::new(chunked_post("5\r\nhe")).chain(Reset);
    let error = Request::read(&mut connection, LIMIT).expect_err("a refusal");
    assert_eq!(
        (error.status(), error.reason()),
        (
            Status::BadRequest,
            "the connection closed inside a 5-byte chunk: the peer reset the connection"
        )
    );
}

#[test]
fn a_transfer_coding_other_than_chunked_on_the_first_of_two_lines_is_refused_with_400() {
    let error = refused(&post(
        "Transfer-Encoding: gzip\r\nTransfer-Encoding: chunked\r\n",
        b"5\r\nhello\r\n0\r\n\r\n",
    ));
    assert_eq!(error.status(), Status::BadRequest, "{error}");
    assert!(error.reason().contains("gzip"), "{error}");
}

#[test]
fn a_transfer_coding_other_than_chunked_is_refused_with_400_naming_it() {
    for coding in ["gzip", "identity", "gzip, chunked", "chunked, gzip", ""] {
        let wire = post(
            &format!("Transfer-Encoding: {coding}\r\n"),
            b"5\r\nhello\r\n0\r\n\r\n",
        );
        assert_refused(
            &wire,
            LIMIT,
            Status::BadRequest,
            &format!("expected the chunked transfer coding, got {coding:?}"),
        );
    }
}

#[test]
fn a_chunk_size_that_is_not_hexadecimal_is_refused_with_400_naming_it() {
    for (line, size) in [
        ("zz", "zz"),
        ("", ""),
        ("-5", "-5"),
        ("0x5", "0x5"),
        ("5 5", "5 5"),
        ("zz;name=value", "zz"),
        (";name=value", ""),
        ("\u{665}", "\u{665}"),
        ("g0000000000000000000000", "g0000000000000000000000"),
    ] {
        let wire = chunked_post(&format!("{line}\r\nhello\r\n0\r\n\r\n"));
        assert_refused(
            &wire,
            LIMIT,
            Status::BadRequest,
            &format!("expected a hexadecimal chunk size, got {size:?}"),
        );
    }
}

#[test]
fn a_chunked_body_growing_past_max_body_is_refused_with_413_before_the_chunk_is_read() {
    // The announced chunk's bytes never arrive: the size line alone refuses.
    assert_refused(
        &chunked_post("10\r\n"),
        8,
        Status::PayloadTooLarge,
        "the chunked body grew past the 8 bytes accepted",
    );
    // Each chunk fits; together they do not.
    assert_refused(
        &chunked_post("6\r\nabcdef\r\n6\r\nghijkl\r\n0\r\n\r\n"),
        10,
        Status::PayloadTooLarge,
        "the chunked body grew past the 10 bytes accepted",
    );
}

#[test]
fn a_chunk_without_its_closing_line_break_is_refused_with_400() {
    for chunks in [
        "5\r\nhelloX\r\n0\r\n\r\n",
        "5\r\nhello world\r\n0\r\n\r\n",
        "5\r\nhello",
    ] {
        assert_refused(
            &chunked_post(chunks),
            LIMIT,
            Status::BadRequest,
            "expected the line break that ends a chunk",
        );
    }
}

#[test]
fn a_connection_that_closes_inside_a_chunked_body_is_refused_with_400() {
    for chunks in ["", "5\r\nhello\r\n"] {
        assert_refused(
            &chunked_post(chunks),
            LIMIT,
            Status::BadRequest,
            "the connection closed inside a chunked body",
        );
    }
    for chunks in ["0\r\n", "0\r\nX-Trailer: yes\r\n"] {
        assert_refused(
            &chunked_post(chunks),
            LIMIT,
            Status::BadRequest,
            "the connection closed inside the chunked trailer",
        );
    }
    let error = refused(&chunked_post("5\r\nhel"));
    assert_eq!(error.status(), Status::BadRequest);
    assert!(
        error
            .reason()
            .starts_with("the connection closed inside a 5-byte chunk: "),
        "{error}"
    );
    assert_refused(
        &chunked_post("5\r\nhello\r\n3"),
        LIMIT,
        Status::BadRequest,
        "the connection closed inside a line",
    );
}

#[test]
fn disagreeing_content_lengths_are_refused_with_400_naming_both() {
    // RFC 7230 section 3.3.2: differing Content-Length values are an
    // unrecoverable framing error; reading either one frames a different
    // message than the peer (or an intermediary) framed.
    for (headers, first, other) in [
        ("Content-Length: 3\r\nContent-Length: 5\r\n", "3", "5"),
        (
            "Content-Length: 5\r\ncontent-length: 5\r\nCONTENT-LENGTH: 3\r\n",
            "5",
            "3",
        ),
        (
            "Content-Length: 5\r\nHost: localhost\r\nContent-Length: five\r\n",
            "5",
            "five",
        ),
        ("Content-Length: 5\r\nContent-Length:\r\n", "5", ""),
    ] {
        assert_refused(
            &post(headers, b"hello"),
            LIMIT,
            Status::BadRequest,
            &format!("Content-Length is stated twice and disagrees: {first:?} and {other:?}"),
        );
    }
}

#[test]
fn a_content_length_repeated_with_one_value_frames_the_body_by_it() {
    let request = read(&post(
        "Content-Length: 5\r\ncontent-length:   5  \r\n",
        b"hello",
    ));
    assert_eq!(request.body(), b"hello");
}

#[test]
fn a_crlf_line_of_exactly_max_line_bytes_is_accepted() {
    // `MAX_LINE` is the longest line accepted; the CR is the terminator's,
    // not the line's.
    let value = "a".repeat(MAX_LINE - "X-Long: ".len());
    let wire = format!("GET /xmla HTTP/1.1\r\nX-Long: {value}\r\n\r\n");
    let request = Request::read(&mut Cursor::new(wire.as_bytes()), LIMIT)
        .unwrap_or_else(|error| panic!("{error}"))
        .expect("a request");
    assert_eq!(request.header("x-long"), Some(value.as_str()));
}

#[test]
fn a_crlf_line_of_exactly_max_line_bytes_is_accepted_however_the_reads_split_it() {
    // A read that ends between the CR and the LF holds one byte past the
    // bound that is still the terminator's.
    let value = "a".repeat(MAX_LINE - "X-Long: ".len());
    let wire = format!("GET /xmla HTTP/1.1\r\nX-Long: {value}\r\n\r\n");
    for step in [
        1,
        2,
        3,
        7,
        4096,
        MAX_LINE,
        MAX_LINE + 1,
        MAX_LINE + 2,
        3 * MAX_LINE,
    ] {
        let request = Request::read(&mut Trickle::new(wire.as_bytes(), step), LIMIT)
            .unwrap_or_else(|error| panic!("step {step}: {error}"))
            .expect("a request");
        assert_eq!(
            request.header("x-long"),
            Some(value.as_str()),
            "step {step}"
        );
    }
}

#[test]
fn a_content_length_past_the_integer_range_is_refused_with_413() {
    // A byte count past `usize` is still a byte count, and it is past any
    // `max_body`: the rustdoc answers it with 413.
    let huge = "99999999999999999999999";
    assert_refused(
        &post(&format!("Content-Length: {huge}\r\n"), b""),
        LIMIT,
        Status::PayloadTooLarge,
        &format!("the body is {huge} bytes; at most {LIMIT} are accepted"),
    );
}

#[test]
fn a_chunk_size_past_the_integer_range_is_refused_with_413() {
    for line in [
        "fffffffffffffffffffffff",
        "FFFFFFFFFFFFFFFFFFFFFFFF",
        "10000000000000000000;ext=1",
        " 123456789abcdef0123 ",
    ] {
        assert_refused(
            &chunked_post(&format!("{line}\r\n")),
            LIMIT,
            Status::PayloadTooLarge,
            &format!("the chunked body grew past the {LIMIT} bytes accepted"),
        );
    }
}

#[test]
fn a_count_with_leading_zeros_is_its_decimal_or_hexadecimal_value() {
    let zeros = "0".repeat(40);
    let request = read(&post(&format!("Content-Length: {zeros}5\r\n"), b"hello"));
    assert_eq!(request.body(), b"hello");
    let decimal = read(&post("Content-Length: 010\r\n", b"0123456789"));
    assert_eq!(decimal.body(), b"0123456789", "decimal, never octal");
    let chunked = read(&chunked_post(&format!(
        "{zeros}5\r\nhello\r\n{zeros}\r\n\r\n"
    )));
    assert_eq!(chunked.body(), b"hello");
}

// --- Request refusals the source does not make (defects) --------------------

#[test]
fn a_transfer_coding_other_than_chunked_on_a_later_line_is_refused_with_400() {
    // RFC 7230 section 3.2.2: two lines of a list header are one list, so
    // this is `chunked, gzip`, which the one-line spelling already refuses;
    // section 3.3.3: a request whose final coding is not chunked cannot be
    // framed, and a server MUST answer it with 400. Reading the first line
    // alone frames by chunks what an intermediary frames otherwise.
    let wire = post(
        "Transfer-Encoding: chunked\r\nTransfer-Encoding: gzip\r\n",
        b"5\r\nhello\r\n0\r\n\r\n",
    );
    match Request::read(&mut Cursor::new(&wire[..]), LIMIT) {
        Err(error) => {
            assert_eq!(error.status(), Status::BadRequest, "{error}");
            assert!(error.reason().contains("gzip"), "{error}");
        }
        Ok(answer) => panic!("framed by the first coding alone: {answer:?}"),
    }
}

#[test]
fn whitespace_between_a_header_name_and_its_colon_is_refused_with_400() {
    // RFC 7230 section 3.2.4: a server MUST refuse such a request with 400,
    // because peers that disagree about the name disagree about the framing.
    for (header, body, name) in [
        (
            "Transfer-Encoding : chunked\r\nContent-Length: 5\r\n",
            &b"0\r\n\r\n"[..],
            "Transfer-Encoding",
        ),
        ("Content-Length\t: 5\r\n", b"hello", "Content-Length"),
        ("Host :localhost\r\nContent-Length: 0\r\n", b"", "Host"),
    ] {
        match Request::read(&mut Cursor::new(&post(header, body)[..]), LIMIT) {
            Err(error) => {
                assert_eq!(error.status(), Status::BadRequest, "{error}");
                assert!(error.reason().contains(name), "{error}");
            }
            Ok(answer) => panic!("{header:?} was read as a header: {answer:?}"),
        }
    }
}

#[test]
fn a_folded_header_line_is_refused_with_400_or_joined_to_the_header_it_continues() {
    // RFC 7230 section 3.2.4: a server MUST either refuse obsolete line
    // folding with 400 or read each fold as a space in the value it
    // continues - never as a header of its own.
    for fold in [" ", "\t"] {
        let wire = post(
            &format!("X-A: 1\r\n{fold}Transfer-Encoding: chunked\r\nContent-Length: 5\r\n"),
            b"0\r\n\r\n",
        );
        match Request::read(&mut Cursor::new(&wire[..]), LIMIT) {
            Err(error) => assert_eq!(error.status(), Status::BadRequest, "{error}"),
            Ok(Some(request)) => {
                assert_eq!(request.header("transfer-encoding"), None, "{fold:?}");
                assert_eq!(request.body(), b"0\r\n\r\n", "{fold:?}");
            }
            Ok(None) => panic!("a request was sent"),
        }
    }
}

// --- Request reading --------------------------------------------------------

#[test]
fn a_request_reads_its_method_target_version_headers_and_body() {
    let wire = format!(
        "POST /xmla HTTP/1.1\r\nHost: localhost:8080\r\nContent-Type: {CONTENT_TYPE}\r\n\
         {ACTION_HEADER}: \"urn:schemas-microsoft-com:xml-analysis:Discover\"\r\n\
         Content-Length: 5\r\n\r\nhello"
    );
    let request = read(wire.as_bytes());
    assert_eq!(request.method(), "POST");
    assert_eq!(request.target(), "/xmla");
    assert_eq!(request.version(), Version::Http11);
    assert_eq!(request.header("host"), Some("localhost:8080"));
    assert_eq!(request.content_type(), Some(CONTENT_TYPE));
    assert_eq!(
        request.soap_action(),
        Some("urn:schemas-microsoft-com:xml-analysis:Discover")
    );
    assert_eq!(request.body(), b"hello");
    assert!(request.keep_alive());
}

#[test]
fn the_method_and_target_are_kept_as_sent_query_included() {
    for (method, target) in [
        ("POST", "/xmla?catalog=market&format=tabular"),
        ("GET", "/xmla?q=%20a%20"),
        ("GET", "http://localhost:8080/xmla?x=1"),
        ("OPTIONS", "*"),
        ("get", "/caf\u{e9}"),
    ] {
        let wire = format!("{method} {target} HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
        let request = read(wire.as_bytes());
        assert_eq!((request.method(), request.target()), (method, target));
    }
}

#[test]
fn header_lookup_ignores_case_and_answers_the_first_of_a_repeated_name() {
    let request =
        read(b"GET /xmla HTTP/1.1\r\nX-Tag: first\r\nx-tag: second\r\nACCEPT: text/xml\r\n\r\n");
    for name in ["X-Tag", "x-tag", "X-TAG"] {
        assert_eq!(request.header(name), Some("first"), "{name}");
    }
    assert_eq!(request.header("accept"), Some("text/xml"));
    assert_eq!(request.header("Accept"), Some("text/xml"));
    assert_eq!(request.header("x-missing"), None);
    assert_eq!(request.content_type(), None);
}

#[test]
fn headers_are_lowercased_trimmed_and_kept_in_wire_order() {
    let request = read(
        b"GET /xmla HTTP/1.1\r\nZeta:   last-named   \r\nALPHA:\tfirst-named\t\r\n\
          Mixed-Case: a:b:c\r\nX-Empty:\r\nX-Unicode: caf\xc3\xa9 \xe2\x9c\x93\r\n\r\n",
    );
    let headers: Vec<(&str, &str)> = request
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        headers,
        [
            ("zeta", "last-named"),
            ("alpha", "first-named"),
            ("mixed-case", "a:b:c"),
            ("x-empty", ""),
            ("x-unicode", "caf\u{e9} \u{2713}"),
        ]
    );
}

#[test]
fn an_empty_connection_reads_as_no_request() {
    for wire in [&b""[..], b"\r\n", b"\n"] {
        let answer = Request::read(&mut Cursor::new(wire), LIMIT).expect("no refusal");
        assert_eq!(answer, None, "{wire:?}");
    }
}

#[test]
fn one_blank_line_before_the_request_line_is_tolerated() {
    for wire in [
        &b"\r\nPOST /xmla HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi"[..],
        &b"\nPOST /xmla HTTP/1.1\nContent-Length: 2\n\nhi"[..],
    ] {
        let request = read(wire);
        assert_eq!((request.target(), request.body()), ("/xmla", &b"hi"[..]));
    }
}

#[test]
fn lf_only_line_endings_are_accepted() {
    let request = read(b"POST /xmla HTTP/1.1\nHost: localhost\nContent-Length: 2\n\nhi");
    assert_eq!(request.header("host"), Some("localhost"));
    assert_eq!(request.body(), b"hi");

    let chunked = read(b"POST /xmla HTTP/1.1\nTransfer-Encoding: chunked\n\n2\nhi\n0\n\n");
    assert_eq!(chunked.body(), b"hi");
}

#[test]
fn a_line_of_exactly_max_line_bytes_ending_in_lf_is_accepted() {
    let value = "a".repeat(MAX_LINE - "X-Long: ".len());
    let wire = format!("GET /xmla HTTP/1.1\nX-Long: {value}\n\n");
    assert_eq!(read(wire.as_bytes()).header("x-long"), Some(value.as_str()));
}

#[test]
fn the_body_is_exactly_content_length_bytes_and_the_next_request_follows_it() {
    let mut connection = Cursor::new(
        &b"POST /first HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello\
           POST /second HTTP/1.1\r\nContent-Length: 3\r\n\r\nbye"[..],
    );
    let first = Request::read(&mut connection, LIMIT)
        .expect("read")
        .expect("a request");
    assert_eq!((first.target(), first.body()), ("/first", &b"hello"[..]));
    let second = Request::read(&mut connection, LIMIT)
        .expect("read")
        .expect("a request");
    assert_eq!((second.target(), second.body()), ("/second", &b"bye"[..]));
    assert_eq!(Request::read(&mut connection, LIMIT).expect("read"), None);
}

#[test]
fn a_body_is_kept_byte_for_byte() {
    let body = "caf\u{e9} \u{2713}\r\n\0\n".as_bytes();
    let request = read(&post(&format!("Content-Length: {}\r\n", body.len()), body));
    assert_eq!(request.body(), body);
}

#[test]
fn a_body_of_exactly_max_body_bytes_is_accepted() {
    let request = read_within(&post("Content-Length: 10\r\n", b"0123456789"), 10);
    assert_eq!(request.body(), b"0123456789");
    let empty = read_within(&post("Content-Length: 0\r\n", b""), 0);
    assert_eq!(empty.body(), b"");
}

#[test]
fn get_and_head_without_a_length_read_an_empty_body() {
    for method in ["GET", "HEAD"] {
        let wire = format!("{method} /xmla HTTP/1.1\r\nHost: localhost\r\n\r\n");
        let request = read(wire.as_bytes());
        assert_eq!((request.method(), request.body()), (method, &b""[..]));
    }
    let with_body = read(b"GET /xmla HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc");
    assert_eq!(with_body.body(), b"abc");
}

#[test]
fn a_chunked_body_is_the_concatenation_of_its_chunks() {
    let request = read(&chunked_post("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"));
    assert_eq!(request.body(), b"hello world");
    assert_eq!(request.header("transfer-encoding"), Some("chunked"));

    let empty = read(&chunked_post("0\r\n\r\n"));
    assert_eq!(empty.body(), b"");
}

#[test]
fn chunk_sizes_read_in_either_case_with_extensions_and_whitespace() {
    let request = read(&chunked_post(
        "A;name=value\r\n0123456789\r\na ; flag\r\nabcdefghij\r\n 2 \r\n\r\n\r\n00\r\n\r\n",
    ));
    assert_eq!(request.body(), b"0123456789abcdefghij\r\n");
}

#[test]
fn the_chunked_coding_is_matched_case_insensitively() {
    for coding in ["chunked", "Chunked", "CHUNKED"] {
        let wire = post(
            &format!("transfer-encoding: {coding}\r\n"),
            b"2\r\nhi\r\n0\r\n\r\n",
        );
        assert_eq!(read(&wire).body(), b"hi", "{coding}");
    }
}

#[test]
fn the_chunked_trailer_is_read_to_its_blank_line_and_discarded() {
    let mut wire = chunked_post("2\r\nhi\r\n0\r\nX-Checksum: abc\r\nX-Other: d\r\n\r\n");
    wire.extend_from_slice(b"GET /next HTTP/1.1\r\n\r\n");
    let mut connection = Cursor::new(&wire[..]);
    let first = Request::read(&mut connection, LIMIT)
        .expect("read")
        .expect("a request");
    assert_eq!(first.body(), b"hi");
    assert_eq!(
        first.header("x-checksum"),
        None,
        "a trailer is not a header"
    );
    let next = Request::read(&mut connection, LIMIT)
        .expect("read")
        .expect("a request");
    assert_eq!(next.target(), "/next");
}

#[test]
fn the_chunked_coding_frames_the_body_over_a_content_length() {
    // RFC 7230 section 3.3.3: Transfer-Encoding overrides Content-Length.
    let wire = post(
        "Content-Length: 3\r\nTransfer-Encoding: chunked\r\n",
        b"5\r\nhello\r\n0\r\n\r\n",
    );
    assert_eq!(read(&wire).body(), b"hello");
}

#[test]
fn a_chunked_body_of_exactly_max_body_bytes_is_accepted() {
    let request = read_within(&chunked_post("4\r\nabcd\r\n6\r\nefghij\r\n0\r\n\r\n"), 10);
    assert_eq!(request.body(), b"abcdefghij");
}

#[test]
fn a_request_framed_across_many_small_reads_reads_as_one_read_does() {
    let wires = [
        post(
            "Host: localhost\r\nSOAPAction: \"urn:x\"\r\nContent-Length: 11\r\n",
            b"hello world",
        ),
        chunked_post("5;ext=1\r\nhello\r\n6\r\n world\r\n0\r\nX-Trailer: t\r\n\r\n"),
        b"\r\nPOST /xmla HTTP/1.0\nContent-Length: 3\n\nabc".to_vec(),
    ];
    for wire in &wires {
        let whole = read(wire);
        for step in [1, 2, 3, 7] {
            let mut trickle = Trickle::new(wire, step);
            let framed = Request::read(&mut trickle, LIMIT)
                .expect("read")
                .expect("a request");
            assert_eq!(framed, whole, "step {step}");
            assert_eq!(
                trickle.position,
                wire.len(),
                "nothing past the request is read"
            );
            assert!(trickle.reads >= wire.len() / step, "step {step}");
        }
    }
}

#[test]
fn a_request_reads_again_to_an_equal_value() {
    let wire = post("Content-Length: 2\r\n", b"hi");
    let request = read(&wire);
    assert_eq!(request.clone(), read(&wire));
    assert_ne!(request, read(&post("Content-Length: 2\r\n", b"ho")));
}

#[test]
fn the_content_type_is_the_content_type_header() {
    let request = read(&post(
        "content-type: application/soap+xml; charset=utf-8\r\nContent-Length: 0\r\n",
        b"",
    ));
    assert_eq!(
        request.content_type(),
        Some("application/soap+xml; charset=utf-8")
    );
}

#[test]
fn the_soap_action_is_the_header_with_its_quotes_taken_off() {
    for (value, action) in [
        ("\"urn:x#Discover\"", Some("urn:x#Discover")),
        ("urn:x#Discover", Some("urn:x#Discover")),
        ("  \"urn:x\"  ", Some("urn:x")),
        ("\"\"", None),
        ("", None),
        ("\"urn:x", Some("\"urn:x")),
        ("urn:x\"", Some("urn:x\"")),
    ] {
        for name in [ACTION_HEADER, "soapaction", "SOAPACTION"] {
            let request = read(&post(
                &format!("{name}: {value}\r\nContent-Length: 0\r\n"),
                b"",
            ));
            assert_eq!(request.soap_action(), action, "{name}: {value}");
        }
    }
    assert_eq!(
        read(&post("Content-Length: 0\r\n", b"")).soap_action(),
        None
    );
}

#[test]
fn http_1_1_keeps_the_connection_alive_unless_it_asks_to_close() {
    for (connection, alive) in [
        (None, true),
        (Some("close"), false),
        (Some("Close"), false),
        (Some("CLOSE"), false),
        (Some("keep-alive"), true),
        (Some("upgrade"), true),
    ] {
        let header = connection.map_or(String::new(), |value| format!("Connection: {value}\r\n"));
        let request = read(format!("GET /xmla HTTP/1.1\r\n{header}\r\n").as_bytes());
        assert_eq!(request.version(), Version::Http11);
        assert_eq!(request.keep_alive(), alive, "{connection:?}");
    }
}

#[test]
fn http_1_0_closes_the_connection_unless_it_asks_to_keep_it_alive() {
    for (connection, alive) in [
        (None, false),
        (Some("keep-alive"), true),
        (Some("Keep-Alive"), true),
        (Some("close"), false),
        (Some("upgrade"), false),
    ] {
        let header = connection.map_or(String::new(), |value| format!("Connection: {value}\r\n"));
        let request = read(format!("GET /xmla HTTP/1.0\r\n{header}\r\n").as_bytes());
        assert_eq!(request.version(), Version::Http10);
        assert_eq!(request.keep_alive(), alive, "{connection:?}");
    }
}

// --- Responses of known length ----------------------------------------------

#[test]
fn a_response_writes_its_status_line_headers_blank_line_and_body() {
    let mut out = Vec::new();
    write_response(&mut out, Status::Ok, CONTENT_TYPE, true, b"hello").expect("written");
    assert_eq!(
        out,
        b"HTTP/1.1 200 OK\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: 5\r\n\
          Connection: keep-alive\r\n\r\nhello"
    );
}

#[test]
fn a_response_that_closes_the_connection_says_so() {
    let mut out = Vec::new();
    write_response(&mut out, Status::NotFound, "text/plain", false, b"none").expect("written");
    let (head, body) = split_head(&out);
    assert_eq!(
        head,
        [
            "HTTP/1.1 404 Not Found",
            "Content-Type: text/plain",
            "Content-Length: 4",
            "Connection: close",
        ]
    );
    assert_eq!(body, b"none");
}

#[test]
fn a_zero_length_response_states_a_zero_length_and_writes_nothing_after_the_head() {
    let mut out = Vec::new();
    write_response(&mut out, Status::Ok, CONTENT_TYPE, true, b"").expect("written");
    let (head, body) = split_head(&out);
    assert!(head.contains(&"Content-Length: 0"), "{head:?}");
    assert_eq!(body, b"");
    assert!(out.ends_with(b"\r\n\r\n"));
}

#[test]
fn a_response_states_the_byte_length_of_its_body_not_its_character_count() {
    let body = "caf\u{e9} \u{2713}".as_bytes();
    let mut out = Vec::new();
    write_response(&mut out, Status::Ok, CONTENT_TYPE, true, body).expect("written");
    let (head, written) = split_head(&out);
    assert!(head.contains(&"Content-Length: 9"), "{head:?}");
    assert_eq!(written, body);
}

#[test]
fn every_status_writes_its_code_and_reason_on_the_status_line() {
    for status in [
        Status::Ok,
        Status::BadRequest,
        Status::NotFound,
        Status::MethodNotAllowed,
        Status::LengthRequired,
        Status::PayloadTooLarge,
        Status::UnsupportedMediaType,
        Status::InternalServerError,
    ] {
        let mut out = Vec::new();
        write_response(&mut out, status, CONTENT_TYPE, false, b"").expect("written");
        let (head, _) = split_head(&out);
        assert_eq!(
            head[0],
            format!("HTTP/1.1 {} {}", status.code(), status.reason())
        );
    }
}

#[test]
fn a_response_is_flushed_once_written() {
    let sink = Sink::default();
    write_response(&mut sink.clone(), Status::Ok, CONTENT_TYPE, true, b"hi").expect("written");
    assert!(sink.flushes.get() >= 1);
    assert!(sink.bytes().ends_with(b"\r\n\r\nhi"));
    assert_eq!(
        sink.flushed_len.get(),
        sink.bytes().len(),
        "the flush follows the last byte"
    );
}

#[test]
fn a_failing_sink_fails_the_response() {
    let mut sink = Budget {
        allowed: 0,
        bytes: Vec::new(),
    };
    let error = write_response(&mut sink, Status::Ok, CONTENT_TYPE, true, b"hi")
        .expect_err("the sink refuses");
    assert_eq!(error.to_string(), "the socket is gone");

    // The head fits, the body does not.
    let mut whole = Vec::new();
    write_response(&mut whole, Status::Ok, CONTENT_TYPE, true, b"hi").expect("written");
    let mut sink = Budget {
        allowed: whole.len() - 2,
        bytes: Vec::new(),
    };
    let error = write_response(&mut sink, Status::Ok, CONTENT_TYPE, true, b"hi")
        .expect_err("the body refuses");
    assert_eq!(error.to_string(), "the socket is gone");
    assert_eq!(
        sink.bytes,
        whole[..whole.len() - 2],
        "the head went out whole"
    );
}

// --- Chunked responses ------------------------------------------------------

#[test]
fn a_chunked_response_opens_with_a_head_that_states_the_chunked_coding() {
    let sink = Sink::default();
    let chunked = begin_chunked(sink.clone(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    assert_eq!(sink.bytes(), CHUNKED_HEAD.as_bytes());
    drop(chunked);

    let closing = begin_chunked(Vec::new(), Status::InternalServerError, "text/xml", false)
        .expect("the head")
        .finish()
        .expect("finished");
    let (head, _) = split_head(&closing);
    assert_eq!(
        head,
        [
            "HTTP/1.1 500 Internal Server Error",
            "Content-Type: text/xml",
            "Transfer-Encoding: chunked",
            "Connection: close",
        ]
    );
    assert!(!head.iter().any(|line| line.starts_with("Content-Length")));
}

#[test]
fn a_chunked_response_with_no_body_is_the_terminating_chunk_alone() {
    let out = begin_chunked(Vec::new(), Status::Ok, CONTENT_TYPE, true)
        .expect("the head")
        .finish()
        .expect("finished");
    assert_eq!(out, format!("{CHUNKED_HEAD}0\r\n\r\n").as_bytes());
}

#[test]
fn a_chunked_response_frames_what_is_written_and_ends_with_the_terminating_chunk() {
    let mut chunked = begin_chunked(Vec::new(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    chunked.write_all(b"<row>1</row>").expect("written");
    write!(chunked, "<row>{}</row>", 2).expect("written");
    let out = chunked.finish().expect("finished");
    assert_eq!(
        out,
        format!("{CHUNKED_HEAD}18\r\n<row>1</row><row>2</row>\r\n0\r\n\r\n").as_bytes()
    );
    let (_, body) = split_head(&out);
    let (sizes, payload) = decode_chunked(body);
    assert_eq!(sizes, [24]);
    assert_eq!(payload, b"<row>1</row><row>2</row>");
}

#[test]
fn small_writes_gather_until_a_flush_sends_them_as_one_chunk() {
    let sink = Sink::default();
    let mut chunked =
        begin_chunked(sink.clone(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    for row in [&b"ab"[..], b"cd", b"ef"] {
        assert_eq!(chunked.write(row).expect("gathered"), row.len());
    }
    assert_eq!(
        sink.bytes(),
        CHUNKED_HEAD.as_bytes(),
        "nothing has gone out yet"
    );

    let flushes = sink.flushes.get();
    chunked.flush().expect("flushed");
    assert_eq!(
        sink.bytes(),
        format!("{CHUNKED_HEAD}6\r\nabcdef\r\n").as_bytes()
    );
    assert_eq!(
        sink.flushes.get(),
        flushes + 1,
        "the flush reaches the sink"
    );
    assert_eq!(
        sink.flushed_len.get(),
        sink.bytes().len(),
        "the sink is flushed after the chunk"
    );

    chunked.write_all(b"g").expect("gathered");
    let returned = chunked.finish().expect("finished");
    assert_eq!(
        sink.bytes(),
        format!("{CHUNKED_HEAD}6\r\nabcdef\r\n1\r\ng\r\n0\r\n\r\n").as_bytes()
    );
    assert!(
        Rc::ptr_eq(&returned.bytes, &sink.bytes),
        "finish hands the sink back"
    );
    assert_eq!(sink.flushes.get(), flushes + 2, "finish flushes the sink");
    assert_eq!(
        sink.flushed_len.get(),
        sink.bytes().len(),
        "finish flushes after the terminator"
    );
}

#[test]
fn an_empty_write_or_flush_emits_no_chunk_that_would_end_the_body() {
    let sink = Sink::default();
    let mut chunked =
        begin_chunked(sink.clone(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    assert_eq!(chunked.write(b"").expect("nothing to gather"), 0);
    chunked.flush().expect("flushed");
    chunked.flush().expect("flushed again");
    assert_eq!(sink.bytes(), CHUNKED_HEAD.as_bytes());

    chunked.write_all(b"row").expect("gathered");
    chunked.flush().expect("flushed");
    chunked.write_all(b"").expect("nothing to gather");
    chunked.flush().expect("flushed again");
    chunked.finish().expect("finished");
    assert_eq!(
        sink.bytes(),
        format!("{CHUNKED_HEAD}3\r\nrow\r\n0\r\n\r\n").as_bytes()
    );
    let bytes = sink.bytes();
    let (_, body) = split_head(&bytes);
    assert_eq!(decode_chunked(body), (vec![3], b"row".to_vec()));
}

#[test]
fn a_full_chunk_goes_out_without_a_flush_and_the_rest_waits() {
    let sink = Sink::default();
    let mut chunked =
        begin_chunked(sink.clone(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    chunked
        .write_all(&vec![b'a'; FULL_CHUNK - 1])
        .expect("gathered");
    assert_eq!(
        sink.bytes(),
        CHUNKED_HEAD.as_bytes(),
        "a byte short of a chunk waits"
    );

    chunked.write_all(b"b").expect("sent");
    let mut expected = CHUNKED_HEAD.as_bytes().to_vec();
    expected.extend_from_slice(b"10000\r\n");
    expected.extend_from_slice(&vec![b'a'; FULL_CHUNK - 1]);
    expected.extend_from_slice(b"b\r\n");
    assert_eq!(sink.bytes(), expected, "a full chunk goes out unasked");
    assert_eq!(sink.flushes.get(), 0, "sent, not flushed");

    chunked.write_all(b"tail").expect("gathered");
    assert_eq!(sink.bytes(), expected, "the rest waits");
    chunked.finish().expect("finished");
    expected.extend_from_slice(b"4\r\ntail\r\n0\r\n\r\n");
    assert_eq!(sink.bytes(), expected);
}

#[test]
fn after_every_write_less_than_one_chunk_waits_unsent() {
    let sink = Sink::default();
    let mut chunked =
        begin_chunked(sink.clone(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    let mut written = Vec::new();
    let sizes = [
        1,
        100,
        FULL_CHUNK - 102,
        1,
        1,
        5000,
        FULL_CHUNK,
        3 * FULL_CHUNK + 17,
        0,
        7,
    ];
    for (index, size) in sizes.into_iter().enumerate() {
        let row: Vec<u8> = (0..=250_u8).cycle().skip(index).take(size).collect();
        chunked.write_all(&row).expect("written");
        written.extend_from_slice(&row);
        let bytes = sink.bytes();
        let sent = sent_payload(&bytes[CHUNKED_HEAD.len()..]);
        assert_eq!(
            sent,
            written[..sent.len()],
            "what went out is what was written"
        );
        assert!(
            written.len() - sent.len() < FULL_CHUNK,
            "after write {index}: {} bytes wait",
            written.len() - sent.len()
        );
    }
    let out = chunked.finish().expect("finished");
    assert_eq!(out.bytes(), sink.bytes());
    let bytes = sink.bytes();
    let (_, body) = split_head(&bytes);
    assert_eq!(decode_chunked(body).1, written);
}

#[test]
fn many_small_writes_share_chunks_and_decode_to_what_was_written() {
    let payload: Vec<u8> = (0..=250_u8).cycle().take(150_000).collect();
    let rows = payload.chunks(100);
    let writes = rows.len();
    let mut chunked = begin_chunked(Vec::new(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    for row in rows {
        chunked.write_all(row).expect("written");
    }
    let out = chunked.finish().expect("finished");
    let (_, body) = split_head(&out);
    let (sizes, decoded) = decode_chunked(body);
    assert_eq!(decoded, payload);
    // A chunk goes out on the write that fills 64 KiB: 656 writes of 100.
    assert_eq!(sizes, [65_600, 65_600, 18_800], "{writes} writes");
}

#[test]
fn one_large_write_decodes_to_the_same_bytes() {
    let payload: Vec<u8> = (0..=250_u8).cycle().take(200_000).collect();
    let mut chunked = begin_chunked(Vec::new(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    chunked.write_all(&payload).expect("written");
    let out = chunked.finish().expect("finished");
    let (_, body) = split_head(&out);
    assert_eq!(decode_chunked(body).1, payload);
}

#[test]
fn a_chunked_response_body_reads_back_as_a_chunked_request_body() {
    let payload: Vec<u8> = "caf\u{e9} \u{2713} <row/>\r\n"
        .bytes()
        .cycle()
        .take(150_000)
        .collect();
    let mut chunked = begin_chunked(Vec::new(), Status::Ok, CONTENT_TYPE, true).expect("the head");
    for row in payload.chunks(1000) {
        chunked.write_all(row).expect("written");
    }
    chunked.flush().expect("flushed");
    let out = chunked.finish().expect("finished");
    let (_, body) = split_head(&out);

    let wire = post("Transfer-Encoding: chunked\r\n", body);
    let request = read_within(&wire, payload.len());
    assert_eq!(request.body(), payload);
    assert_eq!(
        refused_within(&wire, payload.len() - 1).status(),
        Status::PayloadTooLarge
    );
}

#[test]
fn a_failing_sink_fails_the_chunked_head_and_the_chunks() {
    let error = begin_chunked(
        Budget {
            allowed: 0,
            bytes: Vec::new(),
        },
        Status::Ok,
        CONTENT_TYPE,
        true,
    )
    .err()
    .expect("the head refuses");
    assert_eq!(error.to_string(), "the socket is gone");

    let budget = || Budget {
        allowed: CHUNKED_HEAD.len(),
        bytes: Vec::new(),
    };
    let mut chunked =
        begin_chunked(budget(), Status::Ok, CONTENT_TYPE, true).expect("the head fits");
    chunked.write_all(b"row").expect("gathered, not yet sent");
    let error = chunked.flush().expect_err("the chunk refuses");
    assert_eq!(error.to_string(), "the socket is gone");

    let mut chunked =
        begin_chunked(budget(), Status::Ok, CONTENT_TYPE, true).expect("the head fits");
    chunked.write_all(b"row").expect("gathered, not yet sent");
    let error = chunked.finish().expect_err("the last chunk refuses");
    assert_eq!(error.to_string(), "the socket is gone");

    let chunked = begin_chunked(budget(), Status::Ok, CONTENT_TYPE, true).expect("the head fits");
    let error = chunked.finish().expect_err("the terminator refuses");
    assert_eq!(error.to_string(), "the socket is gone");
}
