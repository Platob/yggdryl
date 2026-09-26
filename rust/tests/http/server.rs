//! `rust/src/http/server.rs`: the base HTTP/1.1 server hosting holders and
//! routes, driven over raw `TcpStream` writes - a foreign reading of the wire
//! - and through the crate's own `Request` for the round trips. The
//! connection and the mount each have their own file under `server/`,
//! sharing the fixtures below.

#[path = "server/connection.rs"]
mod connection;
#[path = "server/mount.rs"]
mod mount;

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
fn an_escaped_dot_segment_or_separator_never_reaches_a_mount() {
    // A memory root below a mount, beside a file the mount must not reach.
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let root = memory_root();
    root.child_by_path("secret.txt")
        .expect("child")
        .write_all_bytes(b"classified-bytes")
        .expect("write");
    root.child_by_path("public/a.txt")
        .expect("child")
        .write_all_bytes(b"alpha")
        .expect("write");
    server
        .mount("/files", root.child_by_path("public").expect("child"))
        .expect("mount");
    for target in [
        "/files/%2e%2e/secret.txt",
        "/files/%2E%2E/secret.txt",
        "/files/.%2e/secret.txt",
        "/files/%2e",
        "/files/..%2Fsecret.txt",
        "/files/a%2Fb.txt",
        "/files/a%5Cb.txt",
        "/files/a%00.txt",
    ] {
        let (head, body) = raw(&server, &request_line("GET", target, ""));
        assert_eq!(head.status, Status::BAD_REQUEST, "{target}");
        assert!(
            !body.windows(10).any(|window| window == b"classified"),
            "{target} served the file above the mount"
        );
    }
    // A literal `..` is resolved away before any mount is asked: it names the
    // root's own sibling, which no route or mount answers.
    let (head, _) = raw(&server, &request_line("GET", "/files/../secret.txt", ""));
    assert_eq!(head.status, Status::NOT_FOUND);
    // An ordinary escape still reads as the name it spells.
    let (head, body) = raw(&server, &request_line("GET", "/files/%61.txt", ""));
    assert_eq!(head.status, Status::OK);
    assert_eq!(body, b"alpha");
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

// --- binding, paths and mounts -----------------------------------------------

#[test]
fn a_zero_read_or_write_timeout_is_refused_by_bind() {
    for options in [
        ServerOptions::default().with_read_timeout(Duration::ZERO),
        ServerOptions::default().with_write_timeout(Duration::ZERO),
    ] {
        match Server::bind_with("127.0.0.1:0", options).expect_err("a refusal") {
            Error::Parse { target, reason, .. } => {
                assert_eq!(target, "http server option");
                assert!(reason.contains("timeout"), "{reason}");
            }
            other => panic!("expected a parse refusal, got {other:?}"),
        }
    }
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

// --- faults ------------------------------------------------------------------

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

// --- recording and lifecycle -------------------------------------------------

#[test]
fn every_request_is_recorded_with_what_was_sent_and_answered() {
    // The log keeps the newest requests up to its bound, never all of them.
    assert_eq!(Server::MAX_RECORDED, 65_536);
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
