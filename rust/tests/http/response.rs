//! `rust/src/http/response.rs`: one HTTP answer - status, headers, final URL
//! - and its body, held whole or left on the wire.
//!
//! Every network test runs against the loopback fixture and counts what went
//! on the wire: a `send` is one request, a streaming body materialized from
//! its first byte after the cursor moved is one ranged `GET` more, and
//! nothing else costs one.

use std::io::Read;

use yggdryl::http::{HttpOptions, Request, Response, Session, Status, parse_http_date};
use yggdryl::{Charset, Codec, Error, IOBase, IOKind, Scalar, Url};

use crate::http_server::RecordedExt as _;
use crate::http_server::{HttpServer, LAST_MODIFIED, PageMode};

/// A body of `len` bytes with no repeating window shorter than 251.
fn body(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn get(server: &HttpServer, path: &str) -> Response {
    Request::get(&server.url(path))
        .expect("a URL")
        .send()
        .expect("a response")
}

fn stream(server: &HttpServer, path: &str) -> Response {
    Request::get(&server.url(path))
        .expect("a URL")
        .stream()
        .expect("a response")
}

const JSON: &[u8] = b"{\"a\":1,\"b\":[1,2],\"name\":\"caf\xc3\xa9\"}";

// --- refusals ----------------------------------------------------------------

#[test]
fn a_body_over_max_body_size_is_refused_naming_the_bound() {
    let server = HttpServer::start();
    // Sixty-four identical bytes: over the bound as sent, and under it once
    // gzip has coded them, so the decoded form is what the bound refuses.
    server.put_resource("/big", &[b'x'; 64], Some("text/plain"));
    let session =
        Session::with_options(HttpOptions::default().with_max_body_size(48)).expect("a session");
    let error = session
        .get(&server.url("/big"))
        .expect("a URL")
        .send()
        .expect_err("a refusal");
    match error {
        Error::Io(error) => assert!(error.to_string().contains("max_body_size 48"), "{error}"),
        other => panic!("expected an I/O refusal, got {other:?}"),
    }
    assert_eq!(server.request_count(), 1);
    // A streaming body meets the same bound when it is read whole.
    let response = session
        .get(&server.url("/big"))
        .expect("a URL")
        .stream()
        .expect("a response");
    assert!(matches!(response.bytes(), Err(Error::Io(_))));
    // A decoded body is bounded too: the coded bytes fit, the decoded do not.
    server.set_encoding("/big", "gzip");
    let response = session
        .get(&server.url("/big"))
        .expect("a URL")
        .send()
        .expect("a response");
    assert!(response.size() < 48, "the coded body fits under the bound");
    assert!(matches!(response.bytes(), Err(Error::Io(_))));
}

#[test]
fn a_coding_the_crate_cannot_decode_is_refused_by_name() {
    let wire = b"HTTP/1.1 200 OK\r\nContent-Encoding: br\r\nContent-Length: 2\r\n\r\nxx";
    match Response::from_bytes(wire).expect_err("a refusal") {
        Error::Parse { target, reason, .. } => {
            assert_eq!(target, "http header");
            assert!(reason.contains("br"), "{reason}");
        }
        other => panic!("expected a header refusal, got {other:?}"),
    }
    assert!(
        Response::new(Status::OK)
            .with_header("Content-Encoding", "compress")
            .is_err()
    );
}

#[test]
fn bytes_that_are_not_their_declared_coding_are_refused() {
    let response = Response::new(Status::OK)
        .with_header("Content-Encoding", "gzip")
        .expect("a coding")
        .with_body("this is not gzip");
    assert!(response.bytes().is_err());
    assert!(response.text().is_err());
    // The bytes as sent are still what the handle reads.
    assert_eq!(
        response.read_all_bytes().expect("raw bytes"),
        b"this is not gzip"
    );
}

#[test]
fn reading_behind_the_cursor_of_a_streaming_body_without_ranges_is_refused() {
    let server = HttpServer::start();
    server.put_resource("/flat", &body(8192), Some("text/plain"));
    server.set_ranges("/flat", false);
    let response = stream(&server, "/flat");
    let mut buffer = [0_u8; 16];
    assert_eq!(
        response.pread(100, &mut buffer).expect("a forward read"),
        16
    );
    assert_eq!(buffer, body(8192)[100..116]);
    match response.pread(0, &mut buffer).expect_err("a refusal") {
        Error::Unsupported { operation, .. } => {
            assert!(
                operation.contains("behind the delivered position"),
                "{operation}"
            );
        }
        other => panic!("expected an unsupported read, got {other:?}"),
    }
    // Materializing reads from the first byte, which is behind the cursor.
    assert!(matches!(response.bytes(), Err(Error::Unsupported { .. })));
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_body_that_is_no_structured_text_refuses_scalar_and_a_status_raises() {
    let server = HttpServer::start();
    server.put_resource("/plain", b"just text", Some("text/plain"));
    let response = get(&server, "/plain");
    assert!(response.scalar().is_err());
    assert!(response.raise_for_status().is_ok());

    let missing = get(&server, "/missing");
    assert_eq!(missing.status(), Status::NOT_FOUND);
    assert!(!missing.is_ok());
    match missing.raise_for_status().expect_err("a refusal") {
        Error::Remote {
            service,
            operation,
            status,
            code,
            path,
            ..
        } => {
            assert_eq!(service, "http");
            assert_eq!(operation, "GET");
            assert_eq!(status, 404);
            assert_eq!(code, "Not Found");
            assert!(path.contains("/missing"), "{path}");
        }
        other => panic!("expected a remote refusal, got {other:?}"),
    }
    assert_eq!(server.request_count(), 2);
}

#[test]
fn writes_are_refused() {
    let mut response = Response::new(Status::OK).with_text("held");
    assert!(matches!(
        response.pwrite(0, b"x"),
        Err(Error::Unsupported {
            operation: "writing a response body",
            ..
        })
    ));
    assert!(response.truncate(0).is_err());
    assert!(response.reserve(16).is_err());
    assert!(response.write_all_bytes(b"x").is_err());
    assert_eq!(response.text().expect("text"), "held");
}

// --- the body ----------------------------------------------------------------

#[test]
fn text_is_read_in_the_declared_charset() {
    let server = HttpServer::start();
    server.put_resource(
        "/latin",
        b"caf\xe9 \x80",
        Some("text/plain; charset=windows-1252"),
    );
    server.put_resource("/utf8", "café".as_bytes(), Some("text/plain"));
    let latin = get(&server, "/latin");
    assert_eq!(latin.encoding(), Some(Charset::Cp1252));
    assert_eq!(latin.text().expect("text"), "café €");
    assert_eq!(latin.media_type().charset(), Some(Charset::Cp1252));
    let utf8 = get(&server, "/utf8");
    assert_eq!(utf8.encoding(), None);
    assert_eq!(utf8.text().expect("text"), "café");
}

#[test]
fn scalar_parses_json_json_lines_xml_and_yaml_under_the_content_type() {
    let server = HttpServer::start();
    server.put_resource("/doc.json", JSON, Some("application/json"));
    server.put_resource(
        "/rows.jsonl",
        b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n",
        Some("application/x-ndjson"),
    );
    server.put_resource(
        "/doc.xml",
        b"<root><a>1</a><b>two</b></root>",
        Some("application/xml"),
    );
    server.put_resource("/doc.yaml", b"a: 1\nb: two\n", Some("application/yaml"));

    let json = get(&server, "/doc.json").scalar().expect("json");
    assert_eq!(json.get_key_str("a"), Some(&Scalar::from(1_i64)));
    assert_eq!(json.path("b").map(|b| b.len()), Some(2));
    assert_eq!(
        json.get_key_str("name").and_then(Scalar::as_str),
        Some("café")
    );

    let lines = get(&server, "/rows.jsonl").scalar().expect("json lines");
    let rows = lines.sequence_rows().expect("a sequence of documents");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].get_key_str("a"), Some(&Scalar::from(3_i64)));

    let xml = get(&server, "/doc.xml").scalar().expect("xml");
    assert_eq!(
        xml.path("root.b").as_deref().and_then(Scalar::as_str),
        Some("two")
    );

    let yaml = get(&server, "/doc.yaml").scalar().expect("yaml");
    assert_eq!(yaml.get_key_str("b").and_then(Scalar::as_str), Some("two"));
    assert_eq!(server.request_count(), 4);
}

#[test]
fn scalar_with_field_reads_the_document_under_the_field() {
    let server = HttpServer::start();
    server.put_resource(
        "/doc.json",
        br#"{"id":"7","ok":"true"}"#,
        Some("application/json"),
    );
    let typed = yggdryl::from_json_scalar(br#"[{"id":7,"ok":true}]"#)
        .expect("rows")
        .inferred_struct_field()
        .expect("a root");
    let response = get(&server, "/doc.json");
    let document = response
        .scalar_with_field(&typed)
        .expect("a typed document");
    assert_eq!(document.get(0).as_deref(), Some(&Scalar::from(7_i64)));
    assert_eq!(document.get(1).as_deref(), Some(&Scalar::from(true)));
}

#[test]
fn gzip_and_zstd_bodies_decode_through_the_codec() {
    let server = HttpServer::start();
    for (path, coding, codec) in [
        ("/gz.json", "gzip", Codec::Gzip),
        ("/zst.json", "zstd", Codec::Zstd),
        ("/zz.json", "deflate", Codec::Zlib),
    ] {
        server.put_resource(path, JSON, Some("application/json"));
        server.set_encoding(path, coding);
        let response = get(&server, path);
        assert_eq!(&*response.bytes().expect("decoded"), JSON, "{coding}");
        assert_eq!(
            response.scalar().expect("json").get_key_str("a"),
            Some(&Scalar::from(1_i64))
        );
        assert!(response.media_type().is_encoded(), "{coding}");
        // The handle is the body as sent: coded bytes under the stated length.
        let raw = response.read_all_bytes().expect("raw bytes");
        assert_eq!(codec.load(&raw).expect("decodes"), JSON, "{coding}");
        assert_eq!(response.size(), raw.len() as u64);
        assert_eq!(response.content_length(), Some(raw.len() as u64));
        assert_eq!(response.kind(), IOKind::Memory);
        // Decoded once and cached: the second ask is the same bytes.
        assert_eq!(&*response.bytes().expect("decoded"), JSON);
    }
    assert_eq!(server.request_count(), 3);
}

#[test]
fn into_holder_into_declared_media_composes_the_coding_and_the_encoding() {
    let server = HttpServer::start();
    server.put_resource("/plain.json", JSON, Some("application/json"));
    server.put_resource("/coded.json", JSON, Some("application/json"));
    server.set_encoding("/coded.json", "gzip");
    let expected = yggdryl::from_json_scalar(JSON).expect("json");

    let plain = get(&server, "/plain.json")
        .into_holder()
        .into_declared_media();
    assert_eq!(plain.read_scalar(None).expect("a document"), expected);
    assert_eq!(plain.read_all_bytes().expect("bytes"), JSON);

    let coded = get(&server, "/coded.json")
        .into_holder()
        .into_declared_media();
    assert_eq!(coded.read_all_bytes().expect("decoded bytes"), JSON);
    assert_eq!(coded.read_scalar(None).expect("a document"), expected);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_held_body_cut_short_resumes_from_the_byte_it_reached() {
    let server = HttpServer::start();
    let bytes = body(8192);
    server.put_resource("/cut", &bytes, Some("application/octet-stream"));
    server.cut_body_at("/cut", 3000);
    let session = Session::with_options(HttpOptions::default()).expect("a session");

    let response = session.get(&server.url("/cut")).unwrap().send().unwrap();

    assert_eq!(&*response.bytes().expect("the body"), &bytes[..]);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), Some("bytes=3000-"));
    assert_eq!(session.stats().resumes, 1);
}

// --- standing alone ----------------------------------------------------------

#[test]
fn a_closed_response_reopens_at_its_cursor_on_its_own() {
    let server = HttpServer::start();
    let body: Vec<u8> = (0..10_000_u32).map(|index| (index % 251) as u8).collect();
    server.put_resource("/big", &body, Some("application/octet-stream"));
    // The session goes out of scope: the response carries what it needs.
    let mut response = {
        let session = Session::with_options(HttpOptions::default()).expect("a session");
        session.get(&server.url("/big")).unwrap().stream().unwrap()
    };
    assert!(response.opened());
    let mut head = vec![0_u8; 100];
    response.pread_exact(0, &mut head).expect("the first bytes");

    response.close().expect("let go of the transfer");
    assert!(!response.opened());
    assert_eq!(server.request_count(), 1);

    let mut rest = vec![0_u8; 9_900];
    response
        .pread_exact(100, &mut rest)
        .expect("the rest, re-opened");
    head.extend_from_slice(&rest);
    assert_eq!(head, body);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), Some("bytes=100-"));
    assert!(requests[1].header("if-range").is_some());
    // Everything delivered: closing and opening again asks for nothing.
    response.close().unwrap();
    response.open().unwrap();
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_closed_response_whose_resource_changed_is_refused_on_reopen() {
    let server = HttpServer::start();
    server.put_resource("/doc", &[1_u8; 4096], Some("application/octet-stream"));
    // A session of its own: the default one's counters are other tests'.
    let session = Session::with_options(HttpOptions::default()).expect("a session");
    let mut response = session.get(&server.url("/doc")).unwrap().stream().unwrap();
    let mut head = [0_u8; 10];
    response.pread_exact(0, &mut head).expect("the first bytes");
    response.close().expect("closed");
    server.put_resource("/doc", &[2_u8; 4096], Some("application/octet-stream"));

    match response.open().expect_err("the resource changed") {
        Error::Conflict { actual, .. } => assert_eq!(actual, "changed resource"),
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[test]
fn a_post_answer_is_neither_resumed_nor_reopened() {
    let server = HttpServer::start();
    server.echo("/echo");
    let payload = body(8192);
    let session = Session::with_options(HttpOptions::default()).expect("a session");

    // A cut answer to a POST is not asked for again: that would post twice.
    server.cut_body_at("/echo", 4096);
    let cut = session
        .post(&server.url("/echo"), payload.clone())
        .unwrap()
        .stream()
        .unwrap();
    cut.read_all_bytes()
        .expect_err("the rest of a POST answer is not re-requested");
    assert_eq!(server.request_count(), 1);

    // Nor is a closed one re-opened.
    let mut closed = session
        .post(&server.url("/echo"), payload)
        .unwrap()
        .stream()
        .unwrap();
    let mut head = [0_u8; 10];
    closed.pread_exact(0, &mut head).expect("the first bytes");
    closed.close().expect("let go of the transfer");
    let error = closed
        .pread_exact(10, &mut head)
        .expect_err("a POST answer is not re-opened");
    assert!(error.is_unsupported(), "{error:?}");
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_streamed_head_or_304_answers_an_empty_body() {
    let server = HttpServer::start();
    server.put_resource("/doc", &body(4096), Some("application/octet-stream"));
    let session = Session::with_options(HttpOptions::default()).expect("a session");
    let head = session.head(&server.url("/doc")).unwrap().stream().unwrap();
    assert_eq!(head.status(), Status::OK);
    assert_eq!(head.headers().content_length().unwrap(), Some(4096));
    assert!(head.read_all_bytes().expect("no body to read").is_empty());

    let etag = head.headers().get("etag").expect("an ETag").to_owned();
    let unchanged = session
        .get(&server.url("/doc"))
        .unwrap()
        .with_header("If-None-Match", &etag)
        .unwrap()
        .stream()
        .unwrap();
    assert_eq!(unchanged.status(), Status::NOT_MODIFIED);
    assert!(unchanged.read_all_bytes().expect("no body").is_empty());
}

#[test]
fn a_refusal_is_answered_whatever_the_size_of_its_body() {
    let server = HttpServer::start();
    let text = vec![b'x'; 5 << 20];
    server.server().respond(
        None,
        "/huge-error",
        Response::new(Status::new(503).unwrap()).with_body(text.clone()),
    );
    let session =
        Session::with_options(HttpOptions::default().with_max_attempts(1)).expect("a session");
    for streamed in [false, true] {
        let request = session.get(&server.url("/huge-error")).unwrap();
        let response = if streamed {
            request.stream()
        } else {
            request.send()
        }
        .expect("the refusal is an answer");
        assert_eq!(response.status().code(), 503);
        assert_eq!(response.bytes().expect("its body").len(), text.len());
    }
}

#[test]
fn a_range_the_caller_asked_for_resumes_inside_that_window() {
    let server = HttpServer::start();
    let bytes = body(8192);
    server.put_resource("/r", &bytes, Some("application/octet-stream"));
    server.cut_body_at("/r", 1000);
    let session = Session::with_options(HttpOptions::default()).expect("a session");
    let response = session
        .get(&server.url("/r"))
        .unwrap()
        .with_header("Range", "bytes=4096-")
        .unwrap()
        .stream()
        .unwrap();
    assert_eq!(response.status(), Status::PARTIAL_CONTENT);
    let read = response.read_all_bytes().expect("the window, resumed");
    assert_eq!(read, bytes[4096..]);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    // The window's end is known, so the resume asks for exactly what is left.
    assert_eq!(requests[1].header("range"), Some("bytes=5096-8191"));
}

// --- the headers -------------------------------------------------------------

#[test]
fn cookies_links_and_the_next_request_read_the_headers() {
    let server = HttpServer::start();
    server.put_resource("/c", b"ok", Some("text/plain"));
    server.set_cookie("/c", "session=abc; Path=/; HttpOnly");
    let cookies = get(&server, "/c").cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "session");
    assert_eq!(cookies[0].value, "abc");
    assert!(cookies[0].http_only);

    server.paginate(
        "/p",
        vec!["[1]".to_owned(), "[2]".to_owned()],
        PageMode::Link,
    );
    let first = Request::get(&server.url("/p"))
        .expect("a URL")
        .with_header("x-probe", "yes")
        .expect("a header")
        .send()
        .expect("a response");
    let links = first.links().expect("links");
    assert_eq!(links.len(), 1);
    assert!(links[0].has_rel("next"));
    let next = first
        .next_request()
        .expect("a next page")
        .expect("one more");
    assert_eq!(
        next.url(),
        &Url::from_str(&server.url("/p?page=1")).expect("a URL")
    );
    // The same request at the next URL: its own headers go along.
    assert_eq!(next.headers().get("x-probe"), Some("yes"));
    let last = next.send().expect("the last page");
    assert!(last.next_request().expect("no next page").is_none());
    assert_eq!(server.request_count(), 3);
}

#[test]
fn size_mtime_and_media_type_come_from_the_headers() {
    let server = HttpServer::start();
    server.put_resource("/doc.json", JSON, Some("application/json; charset=utf-8"));
    // A scripted path states the fixed `LAST_MODIFIED`; the bare mount would
    // state the write time.
    server.set_ranges("/doc.json", true);
    let response = get(&server, "/doc.json");
    assert_eq!(response.size(), JSON.len() as u64);
    assert_eq!(
        response.mtime(),
        Some(parse_http_date(LAST_MODIFIED).expect("a date"))
    );
    assert_eq!(response.media_type().base(), &yggdryl::MimeType::JSON);
    assert_eq!(response.encoding(), Some(Charset::Utf8));
    assert_eq!(response.url().to_string(), server.url("/doc.json"));
    assert!(response.history().is_empty());
    assert_eq!(response.request().method(), yggdryl::http::Method::Get);
    assert_eq!(response.version(), yggdryl::http::HttpVersion::Http11);
    assert!(!response.is_redirect());
    assert!(!response.is_container());
}

// --- messages ----------------------------------------------------------------

#[test]
fn from_bytes_and_into_bytes_round_trip_a_message_and_into_scalar_describes_it() {
    // Headers in lexical order, which is how a section renders back.
    let wire =
        b"HTTP/1.1 404 Not Found\r\ncontent-length: 5\r\ncontent-type: text/plain\r\n\r\ngone!";
    let response = Response::from_bytes(wire).expect("a message");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert_eq!(response.text().expect("text"), "gone!");
    assert_eq!(response.into_bytes().expect("wire"), wire);
    assert!(matches!(
        response.raise_for_status(),
        Err(Error::Remote { status: 404, .. })
    ));

    let described = response.into_scalar().expect("a record");
    assert_eq!(
        described.get_key_str("status"),
        Some(&Scalar::from(404_i64))
    );
    assert_eq!(
        described.get_key_str("reason").and_then(Scalar::as_str),
        Some("Not Found")
    );
    assert_eq!(
        described.get_key_str("version").and_then(Scalar::as_str),
        Some("HTTP/1.1")
    );
    assert_eq!(
        described
            .path("headers.content-type")
            .as_deref()
            .and_then(Scalar::as_str),
        Some("text/plain")
    );
    assert_eq!(
        described.get_key_str("body").and_then(Scalar::as_bytes),
        Some(&b"gone!"[..])
    );

    // A chunked message reads, and renders back under the length it decoded.
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n";
    let response = Response::from_bytes(chunked).expect("a message");
    assert_eq!(response.text().expect("text"), "abc");
    assert_eq!(response.content_length(), Some(3));
    assert!(Response::from_bytes(b"HTTP/1.1 200\r\n").is_err());
}

#[test]
fn a_response_is_built_by_hand() {
    let created = Response::new(Status::CREATED).with_text("hi");
    assert_eq!(created.status(), Status::CREATED);
    assert_eq!(created.text().expect("text"), "hi");
    assert_eq!(
        created.headers().get("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert!(
        created
            .into_bytes()
            .expect("wire")
            .starts_with(b"HTTP/1.1 201 Created\r\n")
    );

    let document = yggdryl::from_json_scalar(br#"{"ok":true,"n":2}"#).expect("json");
    let json = Response::new(Status::OK)
        .with_json(&document)
        .expect("a json body")
        .with_status(Status::ACCEPTED)
        .with_headers(
            yggdryl::http::Headers::from_entries([
                ("X-Trace", "t1"),
                ("Content-Type", "application/json; charset=utf-8"),
            ])
            .expect("headers"),
        )
        .expect("headers");
    assert_eq!(json.status(), Status::ACCEPTED);
    assert_eq!(json.scalar().expect("json"), document);
    assert_eq!(json.headers().get("x-trace"), Some("t1"));
    // The given headers win a name both carry.
    assert_eq!(json.encoding(), Some(Charset::Utf8));
    assert_eq!(json.size(), json.bytes().expect("bytes").len() as u64);
    assert_eq!(json.kind(), IOKind::Memory);

    let held = Response::default().with_body(body(10));
    let mut stream = held.into_stream().expect("a stream");
    let mut read = Vec::new();
    stream.read_to_end(&mut read).expect("read");
    assert_eq!(read, body(10));
    assert_eq!(stream.total(), Some(10));
}

// --- streaming bodies --------------------------------------------------------

#[test]
fn a_streaming_body_reads_forward_and_materializes_once() {
    let server = HttpServer::start();
    let bytes = body(8192);
    server.put_resource("/s", &bytes, Some("application/octet-stream"));
    // A session of its own: the default one's counters are other tests'.
    let session = Session::with_options(HttpOptions::default()).expect("a session");
    let response = session.get(&server.url("/s")).unwrap().stream().unwrap();
    assert_eq!(response.kind(), IOKind::File);
    assert_eq!(response.size(), 8192);
    let mut buffer = [0_u8; 16];
    assert_eq!(response.pread(0, &mut buffer).expect("a read"), 16);
    assert_eq!(buffer, bytes[..16]);
    // Forward: skipped to, no request.
    assert_eq!(response.pread(4096, &mut buffer).expect("a read"), 16);
    assert_eq!(buffer, bytes[4096..4112]);
    assert_eq!(server.request_count(), 1);
    // Whole: from the first byte, behind the cursor, so one ranged GET.
    assert_eq!(&*response.bytes().expect("the body"), &bytes[..]);
    assert_eq!(response.kind(), IOKind::Memory);
    assert_eq!(server.request_count(), 2);
    let ranged = &server.requests()[1];
    assert_eq!(ranged.header("range"), Some("bytes=0-"));
    assert_eq!(ranged.status.code(), 206);
    let stats = response.request().stats();
    assert_eq!(stats.resumes, 1);
    // Held now: the same bytes, no request.
    assert_eq!(&*response.bytes().expect("the body"), &bytes[..]);
    assert_eq!(server.request_count(), 2);

    // A fresh stream read whole from the start costs the one GET.
    server.clear_requests();
    let response = stream(&server, "/s");
    assert_eq!(&*response.bytes().expect("the body"), &bytes[..]);
    assert_eq!(response.read_all_bytes().expect("raw"), bytes);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn pstream_bytes_over_a_streaming_body_hands_the_live_stream_in_one_request() {
    let server = HttpServer::start();
    let bytes = body(10_000);
    server.put_resource("/s", &bytes, Some("application/octet-stream"));
    let response = stream(&server, "/s");
    let drained: Vec<u8> = response
        .pstream_bytes(0, 1024)
        .expect("a stream")
        .flat_map(|chunk| chunk.expect("a chunk"))
        .collect();
    assert_eq!(drained, bytes);
    assert_eq!(server.request_count(), 1);

    let response = stream(&server, "/s");
    let stream = response.into_stream().expect("the live stream");
    assert_eq!(stream.delivered(), 0);
    assert_eq!(stream.total(), Some(10_000));
    assert_eq!(stream.read_all_bytes().expect("drained"), bytes);
    assert_eq!(stream.delivered(), 10_000);
    assert_eq!(server.request_count(), 2);
}
