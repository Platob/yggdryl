//! `rust/src/http/request.rs`: the builders, the wire form, the body, and
//! every row of the cost table over the loopback server.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{
    Body, Headers, HttpOptions, Method, Pagination, Request, Session, parse_http_date,
};
use yggdryl::media::RecordOptions;
use yggdryl::{
    DigestAlgorithm, Error, IOBase, IOKind, IOMedia, MediaType, MimeType, Scalar, Serie, Url,
};

use crate::http_server::RecordedExt as _;
use crate::http_server::{HttpServer, LAST_MODIFIED, PageMode, Recorded};

const BODY: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// One recorded request's header, over whichever shape the fixture records.
fn header<'a>(recorded: &'a Recorded, name: &str) -> Option<&'a str> {
    recorded.header(name)
}

fn methods(server: &HttpServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|recorded| recorded.method.to_string())
        .collect()
}

/// A server holding [`BODY`] at `/data.bin`, and a fresh session to it.
fn fixture() -> (HttpServer, Session) {
    let server = HttpServer::start();
    server.put_resource("/data.bin", BODY, Some("application/octet-stream"));
    (server, Session::new())
}

fn leaf(server: &HttpServer, session: &Session, path: &str) -> Request {
    session.get(&server.url(path)).unwrap()
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_url_of_another_scheme_or_no_scheme_is_refused() {
    for text in [
        "ftp://files.example.com/x",
        "no-scheme/path",
        "s3://bucket/key",
    ] {
        let error = Request::get(text).expect_err(text);
        assert!(
            matches!(
                error,
                Error::Parse {
                    target: "http url",
                    ..
                }
            ),
            "{text}: {error:?}"
        );
    }
}

#[test]
fn a_header_that_will_not_validate_is_refused() {
    let error = Request::get("http://127.0.0.1:1/x")
        .unwrap()
        .with_header("Content Type", "text/plain")
        .expect_err("no token");
    assert!(
        matches!(
            error,
            Error::Parse {
                target: "http header",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_request_message_without_a_host_cannot_form_a_url() {
    let error =
        Request::from_bytes(b"GET /rows HTTP/1.1\r\nAccept: */*\r\n\r\n").expect_err("no Host");
    match error {
        Error::Parse { target, reason, .. } => {
            assert_eq!(target, "http message");
            assert!(reason.contains("Host"), "{reason}");
        }
        other => panic!("expected a parse refusal, got {other:?}"),
    }
    let malformed = Request::from_bytes(b"GET\r\n\r\n").expect_err("no request line");
    assert!(
        matches!(
            malformed,
            Error::Parse {
                target: "http message",
                ..
            }
        ),
        "{malformed:?}"
    );
}

#[test]
fn a_leaf_refuses_to_resolve_a_child() {
    let request = Request::get("http://127.0.0.1:1/data.bin").unwrap();
    let error = request
        .child_by_path("x")
        .expect_err("a leaf holds no children");
    match error {
        Error::Io(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory),
        other => panic!("expected an I/O refusal, got {other:?}"),
    }
    assert!(request.ls(true, true).next().is_none());
    assert!(!request.is_container());
    assert!(request.parent().is_none());
}

// --- the body ----------------------------------------------------------------

#[test]
fn a_body_is_held_whole_and_spelled_in_its_shape() {
    assert_eq!(Body::from("hello").as_bytes(), b"hello");
    assert_eq!(Body::from(b"hi".as_slice()).len(), 2);
    assert_eq!(Body::from(Vec::new()), Body::Empty);
    assert_eq!(Body::from(String::new()), Body::Empty);
    assert!(Body::default().is_empty());
    let json = Body::json(&Scalar::from_struct([("id", Scalar::from(1))]).unwrap()).unwrap();
    assert_eq!(json.as_bytes(), br#"{"id":1}"#);
    let form = Body::form([("q", "a b"), ("sym", "A&B=C+D"), ("empty", "")]);
    assert_eq!(form.as_bytes(), b"q=a+b&sym=A%26B%3DC%2BD&empty=");
    let none: [(&str, &str); 0] = [];
    assert_eq!(Body::form(none), Body::Empty);
}

// --- builders and the wire form ----------------------------------------------

#[test]
fn the_attempt_knobs_are_read_back_and_cost_nothing() {
    let session = Session::new();
    let plain = session.get("http://127.0.0.1:1/token").unwrap();
    assert_eq!(plain.idempotent(), None);
    assert_eq!(plain.max_attempts(), None);
    assert_eq!(plain.connect_timeout(), None);
    assert_eq!(plain.deadline(), None);
    assert!(!plain.is_direct());
    let plain_debug = format!("{plain:?}");
    assert!(!plain_debug.contains("<attempt headers>"), "{plain_debug}");
    assert!(!plain_debug.contains("<retry rule>"), "{plain_debug}");

    let request = session
        .post("http://127.0.0.1:1/token", "grant_type=refresh_token")
        .unwrap()
        .with_idempotent(true)
        .with_max_attempts(0)
        .with_connect_timeout(Duration::from_millis(250))
        .with_deadline(Duration::from_secs(2))
        .with_direct(true)
        .with_attempt_headers(|_| Ok(Headers::new()))
        .with_retry_on(|status, _, _| status.code() == 400);
    assert_eq!(request.idempotent(), Some(true));
    assert_eq!(request.max_attempts(), Some(1), "zero attempts is one");
    assert_eq!(request.connect_timeout(), Some(Duration::from_millis(250)));
    assert_eq!(request.deadline(), Some(Duration::from_secs(2)));
    assert!(request.is_direct());
    let debug = format!("{request:?}");
    assert!(debug.contains("<attempt headers>"), "{debug}");
    assert!(debug.contains("<retry rule>"), "{debug}");

    // A clone carries every knob, the hooks shared.
    let clone = request.clone().with_max_attempts(4);
    assert_eq!(clone.idempotent(), Some(true));
    assert_eq!(clone.max_attempts(), Some(4));
    assert_eq!(clone.deadline(), Some(Duration::from_secs(2)));
    assert!(clone.is_direct());
    assert!(format!("{clone:?}").contains("<attempt headers>"));
    assert_eq!(request.with_idempotent(false).idempotent(), Some(false));
    assert_eq!(session.stats().requests, 0);
}

#[test]
fn a_streamed_body_is_never_retried_however_the_request_is_declared() {
    let server = HttpServer::start();
    server.echo("/upload");
    server.fail_status("/upload", 503, Some("0"), 1);
    let session = Session::with_options(HttpOptions::default().with_max_attempts(3)).unwrap();
    let request = session
        .post(&server.url("/upload"), Body::Empty)
        .unwrap()
        .with_idempotent(true)
        .with_attempt_headers(|attempt| {
            // A body read from the caller's reader cannot be shown: the
            // attempt says it is streamed, and shows none.
            let mut headers = Headers::new();
            headers.insert("x-attempt", &attempt.number().to_string())?;
            headers.insert("x-streamed", &attempt.is_streamed().to_string())?;
            headers.insert("x-body", &format!("{:?}", attempt.body()))?;
            Ok(headers)
        });
    let mut reader = std::io::Cursor::new(b"payload".to_vec());

    let response = request.send_reader(&mut reader, 7).unwrap();

    assert_eq!(response.status().code(), 503);
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(header(&recorded[0], "x-attempt"), Some("1"));
    assert_eq!(header(&recorded[0], "x-streamed"), Some("true"));
    assert_eq!(header(&recorded[0], "x-body"), Some("None"));
    assert_eq!(session.stats().retries, 0);
}

#[test]
fn an_attempt_shows_its_hook_the_body_that_goes_out_and_its_debug_no_header_value() {
    let server = HttpServer::start();
    server.echo("/sign");
    let session = Session::new();
    let seen = Arc::new(Mutex::new(String::new()));
    let rendered = Arc::clone(&seen);
    let response = session
        .post(&server.url("/sign"), "payload")
        .unwrap()
        .with_header("Authorization", "Bearer never-rendered")
        .unwrap()
        .with_attempt_headers(move |attempt| {
            *rendered.lock().unwrap() = format!("{attempt:?}");
            let mut headers = Headers::new();
            // What a signature covers: the bytes that go out.
            let body = attempt.body().expect("the body in hand");
            headers.insert("x-body-len", &body.len().to_string())?;
            headers.insert("x-streamed", &attempt.is_streamed().to_string())?;
            Ok(headers)
        })
        .send()
        .unwrap();
    assert_eq!(response.status().code(), 200);
    let recorded = server.requests();
    assert_eq!(header(&recorded[0], "x-body-len"), Some("7"));
    assert_eq!(header(&recorded[0], "x-streamed"), Some("false"));
    let debug = seen.lock().unwrap().clone();
    assert!(
        debug.contains("Attempt") && debug.contains("body_len: Some(7)"),
        "{debug}"
    );
    assert!(!debug.contains("never-rendered"), "{debug}");
}

#[test]
fn building_a_request_costs_nothing_and_keeps_what_it_was_given() {
    let session = Session::new();
    let request = session
        .get("http://127.0.0.1:1/v1/items?a=1")
        .unwrap()
        .with_header("Accept", "application/json")
        .unwrap()
        .with_query([("b", "two words"), ("c", "3")])
        .unwrap()
        .with_media_type(MediaType::from(MimeType::JSON))
        .with_pagination(Pagination::None)
        .with_body("x");
    assert_eq!(request.method(), Method::Get);
    assert_eq!(
        request.url().to_string(),
        "http://127.0.0.1:1/v1/items?a=1&b=two%20words&c=3"
    );
    assert_eq!(request.headers().get("accept"), Some("application/json"));
    assert_eq!(request.body().as_bytes(), b"x");
    assert_eq!(request.media_type().base(), &MimeType::JSON);
    assert_eq!(request.session().stats().requests, 0);
    assert_eq!(request.stats().requests, 0);
    let debug = format!("{request:?}");
    assert!(debug.contains("Request"), "{debug}");
}

#[test]
fn with_json_and_with_form_set_the_body_and_its_content_type() {
    let value = Scalar::from_struct([("id", Scalar::from(7))]).unwrap();
    let json = Request::post("http://127.0.0.1:1/x", Body::Empty)
        .unwrap()
        .with_json(&value)
        .unwrap();
    assert_eq!(json.headers().get("content-type"), Some("application/json"));
    assert_eq!(json.body().as_bytes(), br#"{"id":7}"#);
    let form = Request::post("http://127.0.0.1:1/x", Body::Empty)
        .unwrap()
        .with_form([("a", "1")]);
    assert_eq!(
        form.headers().get("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(form.body().as_bytes(), b"a=1");
}

#[test]
fn a_request_message_round_trips_through_bytes() {
    // Rendered headers come out in lexical order, as the metadata map keeps them.
    let wire =
        b"POST /rows?limit=2 HTTP/1.1\r\naccept: */*\r\ncontent-length: 5\r\nhost: api.example.com:8443\r\n\r\nhello";
    let request = Request::from_bytes(wire).unwrap();
    assert_eq!(request.method(), Method::Post);
    assert_eq!(
        request.url().to_string(),
        "http://api.example.com:8443/rows?limit=2"
    );
    assert_eq!(request.headers().get("host"), None);
    assert_eq!(request.body().as_bytes(), b"hello");
    assert_eq!(request.into_bytes().unwrap(), wire.to_vec());

    // An absolute-form target is the URL, and a default port is not spelled.
    let absolute = Request::from_bytes(b"GET https://example.com/x HTTP/1.1\r\n\r\n")
        .unwrap()
        .with_session(Session::new());
    assert_eq!(absolute.url().to_string(), "https://example.com/x");
    assert_eq!(
        absolute.into_bytes().unwrap(),
        b"GET /x HTTP/1.1\r\nhost: example.com\r\n\r\n".to_vec()
    );
    // Parsing, rendering and binding cost no request.
    assert_eq!(absolute.stats().requests, 0);

    // An IPv6 literal keeps its brackets in the host it asks for.
    for (url, host) in [
        ("http://[::1]:4566/x", "[::1]:4566"),
        ("https://[2001:db8::7]/x", "[2001:db8::7]"),
    ] {
        let rendered = Request::get(url).unwrap().into_bytes().unwrap();
        let expected = format!("GET /x HTTP/1.1\r\nhost: {host}\r\n\r\n");
        assert_eq!(String::from_utf8_lossy(&rendered), expected, "{url}");
    }
}

#[test]
fn a_clone_shares_the_resource_and_not_the_staged_view() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");
    request.pwrite(0, b"XY").unwrap();
    let clone = request.clone();
    assert_eq!(clone.url(), request.url());
    // The clone reads the store; the original reads its stage.
    assert_eq!(&clone.read_range_bytes(0, 2).unwrap(), b"01");
    assert_eq!(&request.read_range_bytes(0, 2).unwrap(), b"XY");
    request.close().unwrap();
    assert_eq!(&clone.read_range_bytes(0, 2).unwrap(), b"XY");
}

// --- the cost table ----------------------------------------------------------

#[test]
fn pread_is_one_ranged_get() {
    let (server, session) = fixture();
    let request = leaf(&server, &session, "/data.bin");
    let mut buffer = [0_u8; 4];

    let read = request.pread(2, &mut buffer).unwrap();

    assert_eq!(read, 4);
    assert_eq!(&buffer, b"2345");
    assert_eq!(methods(&server), ["GET"]);
    assert_eq!(header(&server.requests()[0], "range"), Some("bytes=2-5"));
    assert_eq!(
        header(&server.requests()[0], "accept-encoding"),
        Some("identity")
    );
    assert_eq!(request.stats().gets, 1);
    assert_eq!(request.stats().requests, 1);

    // A read running off the end answers what is there.
    let mut tail = [0_u8; 8];
    assert_eq!(request.pread(BODY.len() as u64 - 3, &mut tail).unwrap(), 3);
    assert_eq!(&tail[..3], b"xyz");
    assert_eq!(request.pread(0, &mut []).unwrap(), 0);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn pread_skips_into_a_200_when_the_server_ignores_the_range() {
    let (server, session) = fixture();
    server.set_ranges("/data.bin", false);
    let request = leaf(&server, &session, "/data.bin");
    let mut buffer = [0_u8; 4];

    assert_eq!(request.pread(10, &mut buffer).unwrap(), 4);
    assert_eq!(&buffer, b"abcd");
    assert_eq!(server.requests()[0].status.code(), 200);
    assert_eq!(server.request_count(), 1);
    assert_eq!(request.pread(100, &mut buffer).unwrap(), 0);
}

#[test]
fn an_absent_resource_reads_as_empty_and_sizes_as_zero() {
    let (server, session) = fixture();
    let request = leaf(&server, &session, "/missing.bin");

    assert_eq!(request.pread(0, &mut [0; 4]).unwrap(), 0);
    assert_eq!(request.read_all_bytes().unwrap(), b"");
    assert_eq!(request.read_range_bytes(0, 4).unwrap(), b"");
    assert_eq!(
        request
            .pstream_bytes(0, 8)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        Vec::<Vec<u8>>::new()
    );
    assert_eq!(request.size(), 0);
    assert_eq!(request.kind(), IOKind::Unknown);
    assert_eq!(
        methods(&server),
        ["GET", "GET", "GET", "GET", "HEAD", "HEAD"]
    );
    assert_eq!(request.stats().heads, 2);
}

#[test]
fn a_range_past_the_end_reads_nothing_and_teaches_the_total_while_open() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");
    request.open().unwrap();
    assert_eq!(methods(&server), ["HEAD"]);
    server.clear_requests();

    let closed = Session::new();
    let unopened = leaf(&server, &closed, "/data.bin");
    assert_eq!(unopened.pread(100, &mut [0; 4]).unwrap(), 0);
    assert_eq!(server.requests()[0].status.code(), 416);
    server.clear_requests();

    // Open, the length already known bounds the read without a request.
    assert_eq!(request.pread(100, &mut [0; 4]).unwrap(), 0);
    assert_eq!(server.request_count(), 0);
    assert_eq!(request.size(), BODY.len() as u64);
    assert_eq!(server.request_count(), 0);
    request.close().unwrap();
}

#[test]
fn read_all_bytes_is_one_get_that_teaches_the_size_while_open() {
    let (server, session) = fixture();
    // A scripted path states the fixed `LAST_MODIFIED`; the bare mount would
    // state the write time.
    server.set_ranges("/data.bin", true);
    let request = leaf(&server, &session, "/data.bin");

    assert_eq!(request.read_all_bytes().unwrap(), BODY);
    assert_eq!(methods(&server), ["GET"]);
    assert_eq!(header(&server.requests()[0], "range"), None);

    let mut opened = leaf(&server, &session, "/data.bin");
    server.clear_requests();
    opened.open().unwrap();
    assert_eq!(opened.read_all_bytes().unwrap(), BODY);
    assert_eq!(opened.size(), BODY.len() as u64);
    assert_eq!(
        opened.mtime(),
        Some(parse_http_date(LAST_MODIFIED).unwrap())
    );
    assert_eq!(opened.kind(), IOKind::File);
    assert!(opened.opened());
    assert_eq!(methods(&server), ["HEAD", "GET"]);
    opened.close().unwrap();
    assert!(opened.closed());
}

#[test]
fn read_range_bytes_and_read_range_digest_are_one_ranged_get_each() {
    let (server, session) = fixture();
    let request = leaf(&server, &session, "/data.bin");

    assert_eq!(request.read_range_bytes(10, 5).unwrap(), b"abcde");
    assert_eq!(header(&server.requests()[0], "range"), Some("bytes=10-14"));
    // A length past the end answers what is there, and never allocates it.
    assert_eq!(request.read_range_bytes(30, 1 << 30).unwrap(), b"uvwxyz");
    assert_eq!(request.read_range_bytes(3, 0).unwrap(), b"");
    assert_eq!(methods(&server), ["GET", "GET"]);
    server.clear_requests();

    let expected = Holder::buffer(Buffer::from_bytes(BODY.to_vec()))
        .read_range_digest(4, 9, DigestAlgorithm::default())
        .unwrap();
    assert_eq!(
        request
            .read_range_digest(4, 9, DigestAlgorithm::default())
            .unwrap(),
        expected
    );
    assert_eq!(methods(&server), ["GET"]);
    assert_eq!(header(&server.requests()[0], "range"), Some("bytes=4-12"));
}

#[test]
fn a_full_pstream_drain_and_read_digest_are_one_get_each() {
    let (server, session) = fixture();
    let request = leaf(&server, &session, "/data.bin");

    let chunks: Vec<Vec<u8>> = request
        .pstream_bytes(3, 8)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(chunks.concat(), &BODY[3..]);
    assert!(chunks.iter().all(|chunk| chunk.len() <= 8));
    assert_eq!(methods(&server), ["GET"]);
    assert_eq!(header(&server.requests()[0], "range"), Some("bytes=3-"));
    server.clear_requests();

    let expected = Holder::buffer(Buffer::from_bytes(BODY.to_vec()))
        .read_digest(DigestAlgorithm::default())
        .unwrap();
    assert_eq!(
        request.read_digest(DigestAlgorithm::default()).unwrap(),
        expected
    );
    assert_eq!(methods(&server), ["GET"]);
    assert!(request.pstream_bytes(0, 0).is_err());
}

#[test]
fn size_mtime_and_kind_are_one_head_each_while_closed_and_none_while_open() {
    let (server, session) = fixture();
    // A scripted path states the fixed `LAST_MODIFIED`; the bare mount would
    // state the write time.
    server.set_ranges("/data.bin", true);
    let mut request = leaf(&server, &session, "/data.bin");

    assert_eq!(request.size(), BODY.len() as u64);
    assert_eq!(
        request.mtime(),
        Some(parse_http_date(LAST_MODIFIED).unwrap())
    );
    assert_eq!(request.kind(), IOKind::File);
    assert_eq!(methods(&server), ["HEAD", "HEAD", "HEAD"]);
    assert_eq!(request.stats().heads, 3);
    server.clear_requests();

    request.open().unwrap();
    request.open().unwrap();
    assert_eq!(request.size(), BODY.len() as u64);
    assert_eq!(
        request.mtime(),
        Some(parse_http_date(LAST_MODIFIED).unwrap())
    );
    assert_eq!(request.kind(), IOKind::File);
    assert!(request.is_atomic());
    assert!(!request.is_tabular());
    assert_eq!(methods(&server), ["HEAD"]);
    request.close().unwrap();
    // Closed again, the store is asked again.
    assert_eq!(request.size(), BODY.len() as u64);
    assert_eq!(methods(&server), ["HEAD", "HEAD"]);
}

#[test]
fn write_all_bytes_is_one_put_carrying_the_media_type() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/new/rows.json");

    request.write_all_bytes(br#"{"a":1}"#).unwrap();

    assert_eq!(methods(&server), ["PUT"]);
    let recorded = &server.requests()[0];
    assert_eq!(recorded.path, "/new/rows.json");
    assert_eq!(recorded.body_len, 7);
    assert_eq!(header(recorded, "content-type"), Some("application/json"));
    assert_eq!(recorded.status.code(), 201);
    assert_eq!(
        server.resource("/new/rows.json").unwrap().0,
        br#"{"a":1}"#.to_vec()
    );
    assert_eq!(request.stats().puts, 1);
    // A declared media type travels the same way.
    request.set_media_type(MediaType::from(MimeType::PLAIN_TEXT));
    request.write_all_bytes(b"plain").unwrap();
    assert_eq!(
        header(&server.requests()[1], "content-type"),
        Some("text/plain")
    );
    assert_eq!(server.requests()[1].status.code(), 204);
}

#[test]
fn clear_is_one_put_of_no_bytes() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");

    request.clear().unwrap();

    assert_eq!(methods(&server), ["PUT"]);
    assert_eq!(server.requests()[0].body_len, 0);
    assert_eq!(server.resource("/data.bin").unwrap().0, Vec::<u8>::new());
}

#[test]
fn pwrite_then_flush_is_one_get_and_one_put() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");

    request.pwrite(0, b"AB").unwrap();
    request.pwrite(2, b"CD").unwrap();
    assert_eq!(methods(&server), ["GET"]);
    // Staged, the write is what reads see and what the kind reports.
    assert_eq!(&request.read_range_bytes(0, 6).unwrap(), b"ABCD45");
    assert_eq!(request.size(), BODY.len() as u64);
    assert_eq!(request.kind(), IOKind::File);
    assert_eq!(server.request_count(), 1);

    request.flush().unwrap();
    assert_eq!(methods(&server), ["GET", "PUT"]);
    let mut stored = BODY.to_vec();
    stored[..4].copy_from_slice(b"ABCD");
    assert_eq!(server.resource("/data.bin").unwrap().0, stored);
    // Nothing dirty, nothing published.
    request.flush().unwrap();
    assert_eq!(server.request_count(), 2);
}

#[test]
fn append_bytes_is_one_get_and_one_put() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");

    let offset = request.append_bytes(b"!!").unwrap();

    assert_eq!(offset, BODY.len() as u64);
    assert_eq!(methods(&server), ["GET", "PUT"]);
    assert_eq!(
        server.resource("/data.bin").unwrap().0,
        [BODY, b"!!"].concat()
    );
}

#[test]
fn a_holder_appends_and_writes_through_the_request_plan() {
    let (server, session) = fixture();
    // A binding's handle is a `Holder`: an append through it is the
    // request's own, published before the call returns.
    let mut held = Holder::HttpRequest(leaf(&server, &session, "/data.bin"));

    assert_eq!(held.append_bytes(b"!!").unwrap(), BODY.len() as u64);
    assert_eq!(methods(&server), ["GET", "PUT"]);
    assert_eq!(
        server.resource("/data.bin").unwrap().0,
        [BODY, b"!!"].concat()
    );

    server.clear_requests();
    held.write_all_bytes(b"new").unwrap();
    assert_eq!(methods(&server), ["PUT"]);
    assert_eq!(server.resource("/data.bin").unwrap().0, b"new".to_vec());
}

#[test]
fn truncate_to_zero_loads_nothing_and_publishes_one_put() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");

    request.truncate(0).unwrap();
    request.reserve(16).unwrap();
    assert!(request.capacity() >= 16);
    request.pwrite(0, b"new").unwrap();
    request.close().unwrap();

    assert_eq!(methods(&server), ["PUT"]);
    assert_eq!(server.resource("/data.bin").unwrap().0, b"new".to_vec());
    server.clear_requests();
    request.truncate(2).unwrap();
    request.flush().unwrap();
    assert_eq!(methods(&server), ["GET", "PUT"]);
    assert_eq!(server.resource("/data.bin").unwrap().0, b"ne".to_vec());
}

#[test]
fn remove_is_one_delete_and_an_absent_resource_is_success() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/data.bin");

    request.remove(false).unwrap();
    assert_eq!(methods(&server), ["DELETE"]);
    assert_eq!(server.requests()[0].status.code(), 204);
    assert!(server.resource("/data.bin").is_none());

    request.remove(true).unwrap();
    assert_eq!(server.requests()[1].status.code(), 404);
    assert_eq!(request.stats().deletes, 2);
}

#[test]
fn a_refusing_status_is_a_remote_error_naming_the_method_and_the_url() {
    let (server, session) = fixture();
    server.set_status("/data.bin", 403, None);
    let mut request = leaf(&server, &session, "/data.bin");

    let error = request.read_all_bytes().expect_err("a 403 is a refusal");
    match &error {
        Error::Remote {
            service,
            operation,
            status,
            code,
            path,
            ..
        } => {
            assert_eq!(*service, "http");
            assert_eq!(*operation, "GET");
            assert_eq!(*status, 403);
            assert_eq!(code, "Forbidden");
            assert_eq!(path, &server.url("/data.bin"));
        }
        other => panic!("expected a remote refusal, got {other:?}"),
    }
    assert!(matches!(
        request.write_all_bytes(b"x"),
        Err(Error::Remote { status: 403, .. })
    ));
    assert!(matches!(
        request.remove(false),
        Err(Error::Remote { status: 403, .. })
    ));
    assert_eq!(request.size(), 0);
    assert!(!request.opened());
}

#[test]
fn a_dirty_stage_decides_the_kind_without_a_request() {
    let (server, session) = fixture();
    let mut request = leaf(&server, &session, "/fresh.bin");
    request.truncate(0).unwrap();
    assert_eq!(request.kind(), IOKind::File);
    assert_eq!(server.request_count(), 0);
    request.close().unwrap();
    assert_eq!(methods(&server), ["PUT"]);
}

// --- the media type ----------------------------------------------------------

#[test]
fn the_media_type_is_declared_else_learned_else_the_urls_own() {
    let server = HttpServer::start();
    server.put_resource("/blob", b"{}", Some("application/json"));
    server.put_resource("/report.csv", b"a,b", Some("text/plain"));
    let session = Session::new();

    let request = leaf(&server, &session, "/blob");
    // No extension, no response yet: the URL says nothing.
    assert_eq!(request.media_type().base(), &MimeType::FILE);
    request.read_all_bytes().unwrap();
    assert_eq!(request.media_type().base(), &MimeType::JSON);

    let mut named = leaf(&server, &session, "/report.csv");
    assert_eq!(named.media_type().base(), &MimeType::CSV);
    named.size();
    // The head taught text/plain, and what a response taught wins over the name.
    assert_eq!(named.media_type().base(), &MimeType::PLAIN_TEXT);
    named.set_media_type(MediaType::from(MimeType::CSV));
    assert_eq!(named.media_type().base(), &MimeType::CSV);
}

// --- sending -----------------------------------------------------------------

#[test]
fn send_reads_the_whole_body_and_stream_leaves_it_on_the_wire() {
    let (server, session) = fixture();
    let request = leaf(&server, &session, "/data.bin");

    let response = request.send().unwrap();
    assert_eq!(response.status().code(), 200);
    assert_eq!(&*response.bytes().unwrap(), BODY);
    assert_eq!(response.content_length(), Some(BODY.len() as u64));
    assert_eq!(response.url().to_string(), server.url("/data.bin"));
    assert_eq!(response.request().method(), Method::Get);

    let streamed = request.stream().unwrap();
    let mut stream = streamed.into_stream().unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut stream, &mut bytes).unwrap();
    assert_eq!(bytes, BODY);
    assert_eq!(stream.delivered(), BODY.len() as u64);
    assert_eq!(methods(&server), ["GET", "GET"]);
}

#[test]
fn send_reader_streams_the_upload_with_its_length() {
    let server = HttpServer::start();
    server.echo("/echo");
    let session = Session::new();
    // A length the caller stated too is the one the transport states, once.
    let request = session
        .post(&server.url("/echo"), Body::Empty)
        .unwrap()
        .with_header("Content-Length", "5")
        .unwrap();
    let mut source = std::io::Cursor::new(b"hello".to_vec());

    let response = request.send_reader(&mut source, 5).unwrap();

    assert_eq!(response.text().unwrap(), "hello");
    let recorded = &server.requests()[0];
    assert_eq!(recorded.body_len, 5);
    assert_eq!(header(recorded, "content-length"), Some("5"));
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_bounded_whole_read_refuses_a_body_past_the_bound() {
    let (server, _) = fixture();
    let session = Session::with_options(HttpOptions::default().with_max_body_size(8)).unwrap();
    let request = leaf(&server, &session, "/data.bin");

    let error = request.send().expect_err("36 bytes do not fit 8");
    assert!(matches!(error, Error::Io(_)), "{error:?}");
    let error = request.read_all_bytes().expect_err("36 bytes do not fit 8");
    assert!(matches!(error, Error::Io(_)), "{error:?}");
}

#[test]
fn a_query_pair_is_appended_and_sent() {
    let server = HttpServer::start();
    server.put_resource("/search", b"[]", Some("application/json"));
    let session = Session::new();

    session
        .get(&server.url("/search?q=1"))
        .unwrap()
        .with_query([("page", "2")])
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(
        server.requests()[0].query,
        [
            ("q".to_owned(), "1".to_owned()),
            ("page".to_owned(), "2".to_owned())
        ]
    );
}

// --- the record surface ------------------------------------------------------

#[test]
fn a_paginated_document_reads_one_batch_per_page_after_one_look() {
    let server = HttpServer::start();
    server.paginate(
        "/orders",
        vec![
            r#"[{"id":1},{"id":2}]"#.to_owned(),
            r#"[{"id":3},{"id":4}]"#.to_owned(),
            r#"[{"id":5}]"#.to_owned(),
        ],
        PageMode::Cursor,
    );
    let session = Session::new();
    let request =
        leaf(&server, &session, "/orders").with_media_type(MediaType::from(MimeType::JSON));

    let columns: Vec<Serie> = request
        .read_serie(None)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(columns.len(), 3);
    assert_eq!(columns.iter().map(Serie::len).sum::<usize>(), 5);
    assert_eq!(columns[0].field().unwrap().fields()[0].name(), "id");
    // The first page is read once to decide and handed to the walk: one GET
    // per page, no more.
    assert_eq!(methods(&server), ["GET", "GET", "GET"]);
    assert_eq!(server.requests()[0].query, Vec::<(String, String)>::new());
    assert_eq!(
        server.requests()[2].query,
        [("cursor".to_owned(), "c2".to_owned())]
    );

    // The batch door walks the same pages under any record options.
    server.clear_requests();
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).unwrap();
    let rows: usize = request
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 5);
    assert_eq!(server.request_count(), 3, "one GET per page");
}

#[test]
fn a_document_of_one_page_reads_through_its_bytes_with_one_get() {
    let server = HttpServer::start();
    server.put_resource(
        "/rows.json",
        br#"[{"id":1,"sym":"AAPL"},{"id":2,"sym":"MSFT"}]"#,
        Some("application/json"),
    );
    let session = Session::new();
    let request = leaf(&server, &session, "/rows.json");

    let columns: Vec<Serie> = request
        .read_serie(None)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns.iter().map(Serie::len).sum::<usize>(), 2);
    assert_eq!(methods(&server), ["GET"]);

    // Pagination off, the leaf reads as every other does: through its bytes.
    server.clear_requests();
    let plain = request.clone().with_pagination(Pagination::None);
    let columns: Vec<Serie> = plain
        .read_serie(None)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns.iter().map(Serie::len).sum::<usize>(), 2);
    assert_eq!(methods(&server), ["GET"]);

    // Held, the request answers the same through the holder.
    server.clear_requests();
    let held = Holder::HttpRequest(request.clone());
    let rows: usize = held
        .read_serie(None)
        .unwrap()
        .map(|column| column.unwrap().len())
        .sum();
    assert_eq!(rows, 2);
    assert_eq!(methods(&server), ["GET"]);
}

#[test]
fn a_record_encoding_reads_through_the_leaf_as_bytes() {
    let server = HttpServer::start();
    let mut stored =
        Buffer::new().with_media_type(Url::from_str("file:///rows.txt").unwrap().media_type());
    stored.write_all_bytes(b"alpha\nbeta\n").unwrap();
    server.put_resource(
        "/rows.txt",
        &stored.read_all_bytes().unwrap(),
        Some("text/plain"),
    );
    let session = Session::new();

    let held = Holder::HttpRequest(leaf(&server, &session, "/rows.txt")).into_declared_media();
    assert!(matches!(held, Holder::Text(_)), "{held:?}");
    let options = held.record_options().unwrap();
    let rows: usize = held
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();

    assert_eq!(rows, 2);
    // One GET for the rows; the text medium sizes nothing first.
    assert!(server.request_count() <= 2, "{:?}", methods(&server));
}
