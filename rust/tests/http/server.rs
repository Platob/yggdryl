//! `rust/src/http/server.rs`: the base HTTP/1.1 server hosting holders and
//! routes, driven over raw `TcpStream` writes - a foreign reading of the wire
//! - and through the crate's own `Request` for the round trips.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use yggdryl::fs::{FsFolder, MemoryFileSystem};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{
    Fault, Headers, HttpOptions, Method, Request, Response, ResponseHead, Server, ServerOptions,
    Session, Status, parse_response,
};
use yggdryl::{Error, IOBase, MediaType};

const ROWS: &[u8] = b"[1, 2, 3, 4, 5, 6, 7, 8, 9]";

/// A server with a memory folder mounted at `/`, holding `rows.json` and
/// `dir/a.txt`.
fn served() -> Server {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let root = memory_root();
    root.child_by_path("rows.json")
        .expect("child")
        .write_all_bytes(ROWS)
        .expect("write");
    root.child_by_path("dir/a.txt")
        .expect("child")
        .write_all_bytes(b"alpha")
        .expect("write");
    server.mount("/", root).expect("mount");
    server
}

fn memory_root() -> Holder {
    let memory = Arc::new(MemoryFileSystem::new());
    Holder::from(FsFolder::from_path(memory, "", None).expect("a memory root"))
}

/// One raw exchange on a fresh connection: `request` written, the answer
/// read to the end of the stream and parsed whole.
fn raw(server: &Server, request: &[u8]) -> (ResponseHead, Vec<u8>) {
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream.write_all(request).expect("write");
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).expect("read");
    parse_response(&answer).unwrap_or_else(|error| panic!("{error}: {answer:?}"))
}

/// The raw bytes of one exchange, unparsed.
fn raw_bytes(server: &Server, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream.write_all(request).expect("write");
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).expect("read");
    answer
}

/// The head of a `HEAD` answer, which a foreign reading cannot frame by its
/// `Content-Length`: the status code and the field lines split by hand, and
/// whatever bytes followed the empty line.
fn raw_head(server: &Server, request: &[u8]) -> (Status, Headers, Vec<u8>) {
    let bytes = raw_bytes(server, request);
    let end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("an empty line ends the head");
    let head = std::str::from_utf8(&bytes[..end]).expect("an ASCII head");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("a status line");
    let code: u16 = status_line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status code");
    let headers =
        Headers::from_entries(lines.map(|line| line.split_once(": ").expect("a field line")))
            .expect("field lines");
    (
        Status::new(code).expect("a status"),
        headers,
        bytes[end + 4..].to_vec(),
    )
}

/// One message read off an open connection: bytes accumulate until the
/// grammar reads a complete message.
fn read_message(stream: &mut TcpStream) -> (ResponseHead, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut answer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).expect("read");
        assert!(
            read > 0,
            "connection closed before a whole message: {answer:?}"
        );
        answer.extend_from_slice(&chunk[..read]);
        match parse_response(&answer) {
            Ok(message) => return message,
            Err(Error::Parse { reason, .. }) if reason.contains("incomplete body") => continue,
            Err(error) => panic!("{error}: {answer:?}"),
        }
    }
}

fn get(server: &Server, path: &str) -> Response {
    Request::get(&server.url_of(path).expect("url").to_string())
        .expect("request")
        .send()
        .expect("send")
}

fn request_line(method: &str, path: &str, extra: &str) -> Vec<u8> {
    format!("{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n{extra}\r\n")
        .into_bytes()
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_malformed_request_line_answers_400_and_closes() {
    let server = served();
    let (head, body) = raw(&server, b"GET\r\n\r\n");
    assert_eq!(head.status, Status::BAD_REQUEST);
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert!(
        head.headers
            .get("server")
            .is_some_and(|server| server.starts_with("yggdryl/"))
    );
    assert!(head.headers.get("date").is_some());
    assert!(!body.is_empty(), "the refusal names what was wrong");
    assert_eq!(
        server.request_count(),
        0,
        "a head the grammar refuses is no request"
    );
}

#[test]
fn a_folded_field_line_answers_400() {
    let server = served();
    let (head, _) = raw(
        &server,
        b"GET /rows.json HTTP/1.1\r\nHost: test\r\n Folded: yes\r\n\r\n",
    );
    assert_eq!(head.status, Status::BAD_REQUEST);
}

#[test]
fn a_head_over_the_bound_answers_431() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_max_head_size(256),
    )
    .expect("bind");
    let filler = "x".repeat(300);
    let (head, _) = raw(
        &server,
        format!("GET / HTTP/1.1\r\nHost: test\r\nX-Filler: {filler}\r\n\r\n").as_bytes(),
    );
    assert_eq!(head.status.code(), 431);
    assert_eq!(server.request_count(), 0);
}

#[test]
fn a_body_over_the_bound_answers_413_and_closes() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_max_body_size(8),
    )
    .expect("bind");
    server.mount("/", memory_root()).expect("mount");
    let (head, _) = raw(
        &server,
        b"PUT /big.bin HTTP/1.1\r\nHost: test\r\nContent-Length: 9\r\n\r\n123456789",
    );
    assert_eq!(head.status.code(), 413);
    let (head, _) = raw(
        &server,
        b"PUT /big.bin HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n9\r\n123456789\r\n0\r\n\r\n",
    );
    assert_eq!(head.status.code(), 413);
    assert_eq!(server.request_count(), 0, "a refused body reaches no route");
    let (head, _) = raw(
        &server,
        b"PUT /ok.bin HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 8\r\n\r\n12345678",
    );
    assert_eq!(
        head.status,
        Status::CREATED,
        "exactly the bound is accepted"
    );
}

#[test]
fn an_ambiguous_framing_answers_400() {
    let server = served();
    let (head, _) = raw(
        &server,
        b"PUT /x HTTP/1.1\r\nHost: test\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n",
    );
    assert_eq!(head.status, Status::BAD_REQUEST);
}

#[test]
fn a_mount_prefix_with_a_query_is_refused() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let error = server
        .mount("/rows?x=1", memory_root())
        .expect_err("a query is no prefix");
    match error {
        Error::Parse {
            target, position, ..
        } => {
            assert_eq!(target, "http path");
            assert_eq!(position, 5);
        }
        other => panic!("{other:?}"),
    }
    assert!(!server.unmount("/rows?x=1"));
}

#[test]
fn a_path_no_mount_covers_is_404() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.mount("/data", memory_root()).expect("mount");
    let response = get(&server, "/elsewhere");
    assert_eq!(response.status(), Status::NOT_FOUND);
    let response = get(&server, "/database");
    assert_eq!(
        response.status(),
        Status::NOT_FOUND,
        "a prefix covers only whole segments"
    );
    assert_eq!(server.request_count(), 2);
}

#[test]
fn an_absent_leaf_is_404_and_a_post_to_a_mount_is_405() {
    let server = served();
    let response = get(&server, "/missing.json");
    assert_eq!(response.status(), Status::NOT_FOUND);
    let response = Request::post(&server.url_of("/rows.json").expect("url").to_string(), "x")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get("allow"),
        Some("GET, HEAD, PUT, DELETE, OPTIONS")
    );
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_put_on_a_container_is_409() {
    let server = served();
    let response = Request::put(&server.url_of("/dir").expect("url").to_string(), "x")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::CONFLICT);
}

#[test]
fn a_range_past_the_end_is_416_naming_the_total() {
    let server = served();
    let (head, body) = raw(
        &server,
        &request_line("GET", "/rows.json", "Range: bytes=100-\r\n"),
    );
    assert_eq!(head.status, Status::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        head.headers.get("content-range"),
        Some(format!("bytes */{}", ROWS.len()).as_str())
    );
    assert!(body.is_empty());
}

#[test]
fn a_handler_error_answers_500_with_its_text() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(None, "/boom", |_| {
        Err(Error::unsupported("booming", "test"))
    });
    let response = get(&server, "/boom");
    assert_eq!(response.status(), Status::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert!(response.text().expect("text").contains("booming"));
}

#[test]
fn a_delete_of_nothing_is_404() {
    let server = served();
    let response = Request::delete(&server.url_of("/nothing.bin").expect("url").to_string())
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert_eq!(server.request_count(), 1);
}

// --- mounts: reading ---------------------------------------------------------

#[test]
fn a_get_of_a_leaf_streams_it_with_its_validators() {
    let server = served();
    let response = get(&server, "/rows.json");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(&*response.bytes().expect("body"), ROWS);
    let headers = response.headers();
    assert_eq!(headers.get("content-type"), Some("application/json"));
    assert_eq!(
        headers.content_length().expect("length"),
        Some(ROWS.len() as u64)
    );
    assert_eq!(headers.get("accept-ranges"), Some("bytes"));
    let etag = headers.etag().expect("etag").expect("an etag");
    assert!(!etag.is_weak());
    assert_eq!(
        etag.opaque.len(),
        16,
        "the lower-case hex of a 64-bit digest"
    );
    assert!(headers.last_modified().expect("date").is_some());
    assert!(
        headers
            .get("server")
            .is_some_and(|server| server.starts_with("yggdryl/"))
    );
    assert!(headers.date().expect("date").is_some());
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_head_declares_the_length_and_writes_no_body() {
    let server = served();
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/rows.json", ""));
    assert_eq!(status, Status::OK);
    assert_eq!(
        headers.content_length().expect("length"),
        Some(ROWS.len() as u64)
    );
    assert_eq!(headers.get("accept-ranges"), Some("bytes"));
    assert!(headers.get("etag").is_some());
    assert!(body.is_empty());
}

#[test]
fn a_single_byte_range_is_206_in_its_three_spellings() {
    let server = served();
    let total = ROWS.len();
    for (range, start, last) in [
        ("bytes=2-4", 2, 4),
        ("bytes=20-", 20, total - 1),
        ("bytes=-3", total - 3, total - 1),
        ("bytes=0-1000", 0, total - 1),
    ] {
        let (head, body) = raw(
            &server,
            &request_line("GET", "/rows.json", &format!("Range: {range}\r\n")),
        );
        assert_eq!(head.status, Status::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            head.headers.get("content-range"),
            Some(format!("bytes {start}-{last}/{total}").as_str()),
            "{range}"
        );
        assert_eq!(body, &ROWS[start..=last], "{range}");
        assert_eq!(
            head.headers.content_length().expect("length"),
            Some(body.len() as u64)
        );
    }
}

#[test]
fn several_ranges_or_a_malformed_one_are_answered_whole() {
    let server = served();
    for range in ["bytes=0-1,3-4", "items=0-1", "bytes=x-y", "bytes=4-2"] {
        let (head, body) = raw(
            &server,
            &request_line("GET", "/rows.json", &format!("Range: {range}\r\n")),
        );
        assert_eq!(head.status, Status::OK, "{range}");
        assert_eq!(body, ROWS, "{range}");
    }
}

#[test]
fn if_range_holds_for_the_etag_and_the_date_and_fails_for_another() {
    let server = served();
    let response = get(&server, "/rows.json");
    let etag = response.headers().get("etag").expect("etag").to_owned();
    let date = response
        .headers()
        .get("last-modified")
        .expect("date")
        .to_owned();
    for validator in [etag.as_str(), date.as_str()] {
        let (head, body) = raw(
            &server,
            &request_line(
                "GET",
                "/rows.json",
                &format!("Range: bytes=0-2\r\nIf-Range: {validator}\r\n"),
            ),
        );
        assert_eq!(head.status, Status::PARTIAL_CONTENT, "{validator}");
        assert_eq!(body, &ROWS[..3]);
    }
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            "Range: bytes=0-2\r\nIf-Range: \"0000000000000000\"\r\n",
        ),
    );
    assert_eq!(head.status, Status::OK, "a mismatch answers the whole");
    assert_eq!(body, ROWS);
}

#[test]
fn if_none_match_and_if_modified_since_answer_304_with_the_validators() {
    let server = served();
    let response = get(&server, "/rows.json");
    let etag = response.headers().get("etag").expect("etag").to_owned();
    let date = response
        .headers()
        .get("last-modified")
        .expect("date")
        .to_owned();
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            &format!("If-None-Match: \"other\", {etag}\r\n"),
        ),
    );
    assert_eq!(head.status, Status::NOT_MODIFIED);
    assert_eq!(head.headers.get("etag"), Some(etag.as_str()));
    assert!(body.is_empty());
    assert_eq!(head.headers.get("content-length"), None);
    let (head, _) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            &format!("If-Modified-Since: {date}\r\n"),
        ),
    );
    assert_eq!(head.status, Status::NOT_MODIFIED);
    assert_eq!(head.headers.get("last-modified"), Some(date.as_str()));
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            "If-Modified-Since: Wed, 01 Jan 2020 00:00:00 GMT\r\n",
        ),
    );
    assert_eq!(head.status, Status::OK, "modified since then");
    assert_eq!(body, ROWS);
    let (head, _) = raw(
        &server,
        &request_line("GET", "/rows.json", "If-None-Match: \"other\"\r\n"),
    );
    assert_eq!(head.status, Status::OK);
}

#[test]
fn a_coded_leaf_is_served_coded_with_content_encoding() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let root = memory_root();
    let coded = yggdryl::gzip::dump(ROWS).expect("gzip");
    root.child_by_path("rows.json.gz")
        .expect("child")
        .write_all_bytes(&coded)
        .expect("write");
    server.mount("/", root).expect("mount");
    let response = get(&server, "/rows.json.gz");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        response.headers().get("content-type"),
        Some("application/json")
    );
    assert_eq!(response.headers().get("content-encoding"), Some("gzip"));
    // The byte surface is the body as sent; `bytes` is the decoded view.
    assert_eq!(
        response.read_all_bytes().expect("body"),
        coded,
        "the bytes as stored"
    );
    assert_eq!(&*response.bytes().expect("decoded"), ROWS);
}

#[test]
fn a_declared_media_type_is_served_with_its_charset() {
    let server = served();
    server
        .set_media_type(
            "/rows.json",
            MediaType::from_content_headers(Some("text/plain; charset=windows-1252"), None)
                .expect("media type"),
        )
        .expect("declare");
    let response = get(&server, "/rows.json");
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/plain; charset=windows-1252")
    );
    let narrow = Server::bind("127.0.0.1:0").expect("bind");
    narrow.mount("/data", memory_root()).expect("mount");
    let error = narrow
        .set_media_type("/nowhere", MediaType::default())
        .expect_err("no mount covers the path");
    assert!(matches!(error, Error::Absent { .. }), "{error:?}");
}

#[test]
fn a_get_of_a_container_lists_its_children_as_json() {
    let server = served();
    let response = get(&server, "/");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        response.headers().get("content-type"),
        Some("application/json")
    );
    let listing = response.scalar().expect("json");
    let rows = listing.sequence_rows().expect("an array");
    assert_eq!(rows.len(), 2);
    let text = response.text().expect("text");
    assert!(text.contains("\"name\":\"rows.json\""), "{text}");
    assert!(
        text.contains(&format!("\"url\":\"{}rows.json\"", server.url())),
        "{text}"
    );
    assert!(text.contains("\"kind\":\"file\""), "{text}");
    assert!(text.contains(&format!("\"size\":{}", ROWS.len())), "{text}");
    assert!(
        text.contains("\"media_type\":\"application/json\""),
        "{text}"
    );
    assert!(text.contains("\"name\":\"dir\""), "{text}");
    let response = get(&server, "/dir");
    let text = response.text().expect("text");
    assert!(
        text.contains(&format!("\"url\":\"{}dir/a.txt\"", server.url())),
        "{text}"
    );
}

#[test]
fn a_percent_encoded_target_reaches_the_decoded_child() {
    let server = served();
    let root = memory_root();
    root.child_by_path("a b.txt")
        .expect("child")
        .write_all_bytes(b"space")
        .expect("write");
    server.mount("/files", root).expect("mount");
    let response = get(&server, "/files/a%20b.txt");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(&*response.bytes().expect("body"), b"space");
    let recorded = server.requests();
    assert_eq!(recorded[0].path, "/files/a b.txt");
    assert_eq!(recorded[0].target, "/files/a%20b.txt");
}

#[test]
fn the_longest_prefix_wins_and_unmount_uncovers() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let outer = memory_root();
    outer
        .child_by_path("inner/x.txt")
        .expect("child")
        .write_all_bytes(b"outer")
        .expect("write");
    let inner = memory_root();
    inner
        .child_by_path("x.txt")
        .expect("child")
        .write_all_bytes(b"inner")
        .expect("write");
    server.mount("/", outer).expect("mount");
    server.mount("data/inner", inner).expect("mount");
    assert_eq!(
        &*get(&server, "/data/inner/x.txt").bytes().expect("body"),
        b"inner"
    );
    assert_eq!(
        &*get(&server, "/inner/x.txt").bytes().expect("body"),
        b"outer"
    );
    assert!(server.unmount("/data/inner/"));
    assert!(!server.unmount("/data/inner"));
    assert_eq!(
        get(&server, "/data/inner/x.txt").status(),
        Status::NOT_FOUND
    );
}

#[test]
fn a_mounted_leaf_is_the_prefix_itself() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let mut buffer = Holder::Buffer(Buffer::new());
    buffer.write_all_bytes(b"one leaf").expect("write");
    server.mount("/leaf", buffer).expect("mount");
    let response = get(&server, "/leaf");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(&*response.bytes().expect("body"), b"one leaf");
    let (head, body) = raw(
        &server,
        &request_line("GET", "/leaf", "Range: bytes=4-\r\n"),
    );
    assert_eq!(head.status, Status::PARTIAL_CONTENT);
    assert_eq!(body, b"leaf");
    let response = Request::put(&server.url_of("/leaf").expect("url").to_string(), "two")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    assert_eq!(&*get(&server, "/leaf").bytes().expect("body"), b"two");
}

// --- mounts: writing ---------------------------------------------------------

#[test]
fn a_put_creates_then_replaces_and_declares_the_content_type() {
    let server = served();
    let url = server.url_of("/new.bin").expect("url").to_string();
    let response = Request::put(&url, "first")
        .expect("request")
        .with_header("content-type", "text/csv; charset=utf-8")
        .expect("header")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::CREATED);
    let response = Request::put(&url, "second")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    let response = get(&server, "/new.bin");
    assert_eq!(&*response.bytes().expect("body"), b"second");
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/csv; charset=utf-8"),
        "the declared type outlives the handle that wrote it"
    );
    assert_eq!(server.request_count(), 3);
}

#[test]
fn a_delete_removes_and_the_next_one_is_404() {
    let server = served();
    let url = server.url_of("/rows.json").expect("url").to_string();
    let response = Request::delete(&url)
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    let response = Request::delete(&url)
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert_eq!(get(&server, "/rows.json").status(), Status::NOT_FOUND);
    let statuses: Vec<u16> = server
        .requests()
        .iter()
        .map(|recorded| recorded.status.code())
        .collect();
    assert_eq!(statuses, [204, 404, 404]);
}

#[test]
fn options_names_the_five_methods() {
    let server = served();
    let (head, body) = raw(&server, &request_line("OPTIONS", "/rows.json", ""));
    assert_eq!(head.status, Status::NO_CONTENT);
    assert_eq!(
        head.headers.get("allow"),
        Some("GET, HEAD, PUT, DELETE, OPTIONS")
    );
    assert!(body.is_empty());
}

// --- routes ------------------------------------------------------------------

#[test]
fn a_route_answers_with_the_handlers_response_and_wins_over_a_mount() {
    let server = served();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    server.route(Some(Method::Get), "/rows.json", move |request| {
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(request.method(), Method::Get);
        assert_eq!(request.url().path_text(false).expect("path"), "/rows.json");
        assert_eq!(request.headers().get("x-probe"), Some("yes"));
        Response::new(Status::OK)
            .with_header("x-answer", "route")
            .map(|response| response.with_text("routed"))
    });
    let response = Request::get(&server.url_of("/rows.json").expect("url").to_string())
        .expect("request")
        .with_header("x-probe", "yes")
        .expect("header")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.headers().get("x-answer"), Some("route"));
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        response.headers().content_length().expect("length"),
        Some(6)
    );
    assert_eq!(response.text().expect("text"), "routed");
    assert_eq!(seen.load(Ordering::Relaxed), 1);
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/rows.json", ""));
    assert_eq!(
        status,
        Status::OK,
        "the route is GET only, HEAD reaches the mount"
    );
    assert_eq!(
        headers.content_length().expect("length"),
        Some(ROWS.len() as u64)
    );
    assert!(body.is_empty());
    assert!(server.unroute(Some(Method::Get), "/rows.json"));
    assert!(!server.unroute(Some(Method::Get), "/rows.json"));
    assert_eq!(&*get(&server, "/rows.json").bytes().expect("body"), ROWS);
}

#[test]
fn an_any_method_route_receives_the_body_and_the_exact_method_wins() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(None, "/echo", |request| {
        Ok(Response::new(Status::OK)
            .with_header("x-method", request.method().as_str())?
            .with_body(request.body().clone()))
    });
    server.route(Some(Method::Delete), "/echo", |_| {
        Ok(Response::new(Status::NO_CONTENT))
    });
    let url = server.url_of("/echo").expect("url").to_string();
    let response = Request::post(&url, "payload")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.headers().get("x-method"), Some("POST"));
    assert_eq!(&*response.bytes().expect("body"), b"payload");
    let response = Request::delete(&url)
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn respond_answers_the_same_every_time() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.respond(
        None,
        "/fixed",
        Response::new(Status::ACCEPTED)
            .with_header("x-fixed", "1")
            .expect("header")
            .with_body("same"),
    );
    for _ in 0..2 {
        let response = get(&server, "/fixed");
        assert_eq!(response.status(), Status::ACCEPTED);
        assert_eq!(response.headers().get("x-fixed"), Some("1"));
        assert_eq!(&*response.bytes().expect("body"), b"same");
    }
}

#[test]
fn a_chunked_route_answer_is_framed_in_chunks_and_a_head_declares_a_length() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(None, "/chunked", |_| {
        Response::new(Status::OK)
            .with_header("transfer-encoding", "chunked")
            .map(|response| response.with_body("chunky"))
    });
    let bytes = raw_bytes(&server, &request_line("GET", "/chunked", ""));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("transfer-encoding: chunked\r\n"), "{text}");
    assert!(!text.contains("content-length"), "{text}");
    assert!(text.ends_with("6\r\nchunky\r\n0\r\n\r\n"), "{text}");
    let (head, body) = parse_response(&bytes).expect("a chunked message");
    assert_eq!(body, b"chunky");
    assert_eq!(
        head.headers.get("content-length"),
        Some("6"),
        "decoded by the grammar"
    );
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/chunked", ""));
    assert_eq!(status, Status::OK);
    assert_eq!(headers.get("content-length"), Some("6"));
    assert_eq!(headers.get("transfer-encoding"), None);
    assert!(body.is_empty());
}

// --- faults ------------------------------------------------------------------

#[test]
fn cut_body_at_writes_the_head_and_that_many_bytes_then_closes() {
    let server = served();
    server.inject("/rows.json", Fault::CutBodyAt(5), 1);
    let bytes = raw_bytes(&server, &request_line("GET", "/rows.json", ""));
    let error = parse_response(&bytes).expect_err("a severed body");
    assert!(error.to_string().contains("incomplete body"), "{error}");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(&format!("content-length: {}\r\n", ROWS.len())),
        "{text}"
    );
    assert!(text.ends_with("\r\n\r\n[1, 2"), "{text}");
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].status, Status::OK);
    assert_eq!(
        &*get(&server, "/rows.json").bytes().expect("body"),
        ROWS,
        "once only"
    );
}

#[test]
fn close_before_answer_writes_nothing_and_records_the_closed_status() {
    let server = served();
    server.inject("/rows.json", Fault::CloseBeforeAnswer, 1);
    let bytes = raw_bytes(&server, &request_line("GET", "/rows.json", ""));
    assert!(bytes.is_empty(), "{bytes:?}");
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].is_closed());
    assert_eq!(recorded[0].status.code(), 499);
    assert_eq!(get(&server, "/rows.json").status(), Status::OK);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn refuse_answers_the_status_with_retry_after_before_any_route_or_mount() {
    let server = served();
    // One attempt per request: a retried 503 would reach the route.
    let once = Session::with_options(HttpOptions::default().with_max_attempts(1)).expect("session");
    let get = |path: &str| {
        once.get(&server.url_of(path).expect("url").to_string())
            .expect("request")
            .send()
            .expect("send")
    };
    server.route(None, "/rows.json", |_| panic!("the fault answers first"));
    server.inject(
        "/rows.json",
        Fault::Refuse {
            status: Status::SERVICE_UNAVAILABLE,
            retry_after: Some(Duration::from_secs(2)),
        },
        2,
    );
    for _ in 0..2 {
        let response = get("/rows.json");
        assert_eq!(response.status(), Status::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get("retry-after"), Some("2"));
        assert!(response.bytes().expect("body").is_empty());
    }
    server.unroute(None, "/rows.json");
    assert_eq!(get("/rows.json").status(), Status::OK, "twice only");
    let statuses: Vec<u16> = server
        .requests()
        .iter()
        .map(|recorded| recorded.status.code())
        .collect();
    assert_eq!(statuses, [503, 503, 200]);
}

#[test]
fn delay_sleeps_before_answering_and_zero_times_is_every_request() {
    let server = served();
    server.inject("/rows.json", Fault::Delay(Duration::from_millis(150)), 0);
    for _ in 0..2 {
        let started = Instant::now();
        let response = get(&server, "/rows.json");
        assert_eq!(response.status(), Status::OK);
        assert!(started.elapsed() >= Duration::from_millis(150));
    }
    server.clear_faults();
    let started = Instant::now();
    get(&server, "/rows.json");
    assert!(started.elapsed() < Duration::from_millis(150));
}

// --- connections -------------------------------------------------------------

#[test]
fn keep_alive_serves_two_requests_on_one_connection() {
    let server = served();
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .write_all(b"GET /rows.json HTTP/1.1\r\nHost: test\r\n\r\n")
        .expect("write");
    let (head, body) = read_message(&mut stream);
    assert_eq!(head.status, Status::OK);
    assert_eq!(body, ROWS);
    assert_eq!(head.headers.get("connection"), None);
    stream
        .write_all(b"GET /dir/a.txt HTTP/1.1\r\nHost: test\r\n\r\n")
        .expect("write");
    let (head, body) = read_message(&mut stream);
    assert_eq!(head.status, Status::OK);
    assert_eq!(body, b"alpha");
    assert_eq!(server.connections(), 1);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn connection_close_and_http_1_0_end_the_connection_after_the_answer() {
    let server = served();
    let (head, body) = raw(&server, &request_line("GET", "/rows.json", ""));
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert_eq!(body, ROWS);
    let (head, body) = raw(&server, b"GET /rows.json HTTP/1.0\r\nHost: test\r\n\r\n");
    assert_eq!(head.version.as_str(), "HTTP/1.1");
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert_eq!(body, ROWS);
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .write_all(b"GET /rows.json HTTP/1.0\r\nHost: test\r\nConnection: keep-alive\r\n\r\n")
        .expect("write");
    let (head, _) = read_message(&mut stream);
    assert_eq!(
        head.headers.get("connection"),
        None,
        "HTTP/1.0 asked to stay"
    );
    stream
        .write_all(b"GET /dir/a.txt HTTP/1.0\r\nHost: test\r\nConnection: close\r\n\r\n")
        .expect("write");
    let (_, body) = read_message(&mut stream);
    assert_eq!(body, b"alpha");
    assert_eq!(server.connections(), 3);
}

#[test]
fn keep_alive_off_closes_every_connection() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_keep_alive(false),
    )
    .expect("bind");
    server.mount("/", memory_root()).expect("mount");
    let (head, _) = raw(&server, b"GET /none HTTP/1.1\r\nHost: test\r\n\r\n");
    assert_eq!(head.status, Status::NOT_FOUND);
    assert_eq!(head.headers.get("connection"), Some("close"));
}

#[test]
fn a_chunked_request_body_is_decoded_and_its_trailer_folded_in() {
    let server = served();
    let (head, _) = raw(
        &server,
        b"PUT /up.bin HTTP/1.1\r\nHost: test\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nTrailer: X-Sum\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\nX-Sum: 5\r\n\r\n",
    );
    assert_eq!(head.status, Status::CREATED);
    assert_eq!(&*get(&server, "/up.bin").bytes().expect("body"), b"abcde");
    let recorded = &server.requests()[0];
    assert_eq!(recorded.method, Method::Put);
    assert_eq!(recorded.body_len, 5);
    assert_eq!(recorded.headers.get("content-length"), Some("5"));
    assert_eq!(recorded.headers.get("transfer-encoding"), None);
    assert_eq!(recorded.headers.get("x-sum"), Some("5"));
}

#[test]
fn expect_100_continue_is_acknowledged_before_the_body_is_read() {
    let server = served();
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    stream
        .write_all(
            b"PUT /expected.bin HTTP/1.1\r\nHost: test\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n",
        )
        .expect("write");
    let mut interim = [0_u8; 25];
    stream.read_exact(&mut interim).expect("the interim answer");
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    stream.write_all(b"body").expect("write");
    let (head, _) = read_message(&mut stream);
    assert_eq!(head.status, Status::CREATED);
    assert_eq!(
        &*get(&server, "/expected.bin").bytes().expect("body"),
        b"body"
    );
}

// --- recording and lifecycle -------------------------------------------------

#[test]
fn every_request_is_recorded_with_what_was_sent_and_answered() {
    let server = served();
    let response = Request::get(
        &server
            .url_of("/rows.json?x=1&y=%41")
            .expect("url")
            .to_string(),
    )
    .expect("request")
    .with_header("x-probe", "yes")
    .expect("header")
    .send()
    .expect("send");
    assert_eq!(response.status(), Status::OK);
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    let first = &recorded[0];
    assert_eq!(first.method, Method::Get);
    assert_eq!(first.target, "/rows.json?x=1&y=%41");
    assert_eq!(first.path, "/rows.json");
    assert_eq!(
        first.query,
        [
            ("x".to_owned(), "1".to_owned()),
            ("y".to_owned(), "A".to_owned())
        ]
    );
    assert_eq!(first.headers.get("x-probe"), Some("yes"));
    assert_eq!(first.body_len, 0);
    assert_eq!(first.status, Status::OK);
    assert!(!first.is_closed());
    server.set_recording(false);
    get(&server, "/rows.json");
    assert_eq!(server.requests().len(), 1, "counted, not logged");
    assert_eq!(server.request_count(), 2);
    server.clear_requests();
    assert!(server.requests().is_empty());
    assert_eq!(server.request_count(), 0);
    let quiet = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_recording(false),
    )
    .expect("bind");
    get(&quiet, "/none");
    assert!(quiet.requests().is_empty());
    assert_eq!(quiet.request_count(), 1);
}

#[test]
fn the_url_the_address_and_the_options_describe_the_binding() {
    let options = ServerOptions::default().with_server_header("test/1");
    let server = Server::bind_with("127.0.0.1:0", options).expect("bind");
    assert_eq!(server.address().port(), server.port());
    assert_eq!(
        server.url().to_string(),
        format!("http://127.0.0.1:{}/", server.port())
    );
    assert_eq!(
        server.url_of("/a/b?c=1").expect("url").to_string(),
        format!("http://127.0.0.1:{}/a/b?c=1", server.port())
    );
    assert_eq!(server.options().server_header(), "test/1");
    let (head, _) = raw(&server, &request_line("GET", "/", ""));
    assert_eq!(head.headers.get("server"), Some("test/1"));
    assert_eq!(head.status, Status::NOT_FOUND, "nothing mounted");
}

#[test]
fn shutdown_stops_accepting() {
    let server = served();
    let address = server.address();
    assert_eq!(get(&server, "/rows.json").status(), Status::OK);
    server.shutdown().expect("shutdown");
    let refused =
        TcpStream::connect_timeout(&address, Duration::from_millis(500)).and_then(|mut stream| {
            stream.write_all(b"GET / HTTP/1.1\r\nHost: test\r\n\r\n")?;
            let mut answer = Vec::new();
            stream.read_to_end(&mut answer)?;
            Ok(answer)
        });
    let stopped = match &refused {
        Err(_) => true,
        Ok(bytes) => bytes.is_empty(),
    };
    assert!(stopped, "{refused:?}");
}

#[test]
fn the_etag_can_be_turned_off() {
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_etag(false)).expect("bind");
    let root = memory_root();
    root.child_by_path("x.txt")
        .expect("child")
        .write_all_bytes(b"x")
        .expect("write");
    server.mount("/", root).expect("mount");
    let response = get(&server, "/x.txt");
    assert_eq!(response.headers().get("etag"), None);
    assert_eq!(response.headers().get("accept-ranges"), Some("bytes"));
}
