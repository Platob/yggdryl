//! `rust/src/http/server.rs`: the server hosting holders and routes, driven
//! over raw `TcpStream` writes, a foreign reading of the wire, and through
//! the crate's own `Request` for the round trips. The connection, the mount
//! and the framed versions each have their own file under `server/`,
//! sharing the fixtures below.

#[path = "server/connection.rs"]
mod connection;
#[path = "server/forwarded.rs"]
mod forwarded;
#[cfg(feature = "http2")]
#[path = "server/framed.rs"]
mod framed;
#[cfg(feature = "http2")]
#[path = "server/h2.rs"]
mod h2;
#[cfg(feature = "http3")]
#[path = "server/h3.rs"]
mod h3;
#[path = "server/mount.rs"]
mod mount;
#[path = "server/trace.rs"]
mod trace;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
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

/// A fresh, empty temporary folder for one trace test, labelled `label` so
/// two tests never collide, and the `Holder` [`ServerOptions::with_trace`]
/// writes exchanges into.
fn trace_folder(label: &str) -> (PathBuf, Holder) {
    let dir =
        std::env::temp_dir().join(format!("yggdryl-http-trace-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a trace directory");
    let holder = Holder::folder(&dir).expect("a trace folder");
    (dir, holder)
}

/// The bytes of `path` once `ready` answers true of them, polled for up to
/// five seconds: a trace's files complete a moment after the connection
/// thread finishes writing the exchange, after the client already read it.
fn wait_for(path: &Path, ready: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = std::fs::read(path)
            && ready(&bytes)
        {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "{path:?} never reached the expected state"
        );
        std::thread::sleep(Duration::from_millis(20));
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

/// The reason `bind_with` refuses `options` with, as an `http server option`.
fn refused_at_bind(options: ServerOptions) -> String {
    match Server::bind_with("127.0.0.1:0", options).map(|_| ()) {
        Err(Error::Parse { target, reason, .. }) => {
            assert_eq!(target, "http server option");
            reason.to_string()
        }
        other => panic!("expected a parse refusal, got {other:?}"),
    }
}

#[test]
fn a_zero_or_unbounded_read_or_write_timeout_is_refused_by_bind() {
    for timeout in [
        Duration::ZERO,
        ServerOptions::MAX_TIMEOUT + Duration::from_nanos(1),
    ] {
        for (name, options) in [
            (
                "read_timeout",
                ServerOptions::default().with_read_timeout(timeout),
            ),
            (
                "write_timeout",
                ServerOptions::default().with_write_timeout(timeout),
            ),
        ] {
            let reason = refused_at_bind(options);
            assert!(reason.starts_with(name), "{reason}");
        }
    }
    // A timeout no deadline can hold is refused, never a connection that
    // panics adding it to the clock.
    let reason =
        refused_at_bind(ServerOptions::default().with_read_timeout(Duration::from_secs(u64::MAX)));
    assert!(reason.contains("at most 86400 seconds"), "{reason}");
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default()
            .with_read_timeout(ServerOptions::MAX_TIMEOUT)
            .with_write_timeout(ServerOptions::MAX_TIMEOUT),
    )
    .expect("the bound itself binds");
    server.mount("/", memory_root()).expect("mount");
    assert_eq!(get(&server, "/nothing").status(), Status::NOT_FOUND);
}

#[test]
fn a_public_url_that_is_no_plain_http_origin_is_refused_by_bind() {
    for public in [
        "ftp://data.example.com/",
        "https://data.example.com/olap?x=1",
        "https://data.example.com/olap#top",
        "https://user:secret@data.example.com/olap",
        "https://user@data.example.com/",
    ] {
        let options = ServerOptions::default()
            .with_public_url(yggdryl::Url::from_str(public).expect("a URL"));
        assert_eq!(
            options.public_url().map(ToString::to_string),
            Some(public.to_owned())
        );
        let reason = refused_at_bind(options);
        assert!(reason.starts_with("public_url"), "{public}: {reason}");
        // A refused credential is not repeated in the refusal.
        assert!(!reason.contains("secret"), "{reason}");
    }
    let refused = ServerOptions::default().with_path_prefix("/olap?x=1");
    assert!(matches!(refused, Err(Error::Parse { .. })));
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
        // A HEAD with no route of its own reaches this GET route too.
        assert!(matches!(request.method(), Method::Get | Method::Head));
        assert_eq!(request.url().path_text(false).expect("path"), "/rows.json");
        if request.method() == Method::Get {
            assert_eq!(request.headers().get("x-probe"), Some("yes"));
        }
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
        "a HEAD with no route of its own is answered by the GET route"
    );
    assert_eq!(headers.content_length().expect("length"), Some(6));
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
fn a_server_bound_to_every_address_states_its_loopback_url() {
    let server = Server::bind("0.0.0.0:0").expect("bind");
    assert_eq!(
        server.url().to_string(),
        format!("http://127.0.0.1:{}/", server.port())
    );
    assert!(server.address().ip().is_unspecified());
    let response = get(&server, "/nothing");
    assert_eq!(response.status(), Status::NOT_FOUND);
}

// --- the trailing-slash redirect ---------------------------------------------

#[test]
fn a_get_or_head_of_a_routes_path_with_a_trailing_slash_is_308_to_the_relative_path() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.respond(
        Some(Method::Get),
        "/olap/xmla",
        Response::new(Status::OK).with_text("described"),
    );
    server.route(Some(Method::Post), "/olap/xmla", |request| {
        Ok(Response::new(Status::OK).with_text(&format!(
            "posted {} to {}",
            String::from_utf8_lossy(request.body().as_bytes()),
            request.url().path_text(false).expect("a path")
        )))
    });
    let (head, body) = raw(&server, &request_line("GET", "/olap/xmla/?x=1", ""));
    assert_eq!(head.status, Status::PERMANENT_REDIRECT);
    assert_eq!(head.headers.get("location"), Some("../xmla?x=1"));
    assert!(body.is_empty());
    let (status, headers, _) = raw_head(&server, &request_line("HEAD", "/olap/xmla/", ""));
    assert_eq!(status, Status::PERMANENT_REDIRECT);
    assert_eq!(headers.get("location"), Some("../xmla"));

    // A POST is served by the route, never redirected.
    let (head, body) = raw(
        &server,
        b"POST /olap/xmla/ HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 4\r\n\r\nbody",
    );
    assert_eq!(head.status, Status::OK);
    assert_eq!(String::from_utf8_lossy(&body), "posted body to /olap/xmla/");

    // A method the bare path does not route is 405 there, as it would be
    // without the slash; a path routed nowhere stays 404.
    let (head, _) = raw(&server, &request_line("DELETE", "/olap/xmla/", ""));
    assert_eq!(head.status, Status::METHOD_NOT_ALLOWED);
    assert_eq!(head.headers.get("allow"), Some("GET, HEAD, POST"));
    let (head, _) = raw(&server, &request_line("GET", "/nothing/", ""));
    assert_eq!(head.status, Status::NOT_FOUND);
    let (head, _) = raw(&server, &request_line("GET", "/olap/xmla//", ""));
    assert_eq!(
        head.status,
        Status::NOT_FOUND,
        "one trailing slash, not two"
    );

    // The crate's client follows the relative Location to the route.
    let response = get(&server, "/olap/xmla/");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.text().expect("text"), "described");
    let recorded = server.requests();
    let last = &recorded[recorded.len() - 2..];
    assert_eq!(last[0].status, Status::PERMANENT_REDIRECT);
    assert_eq!(last[1].path, "/olap/xmla");
}

// --- method-not-allowed and the HEAD fallback --------------------------------

#[test]
fn a_path_routed_for_other_methods_is_405_naming_them_in_allow() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Post), "/only-post", |_| {
        Ok(Response::new(Status::NO_CONTENT))
    });
    let response = get(&server, "/only-post");
    assert_eq!(response.status(), Status::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers().get("allow"), Some("POST"));
    assert_eq!(
        response.text().expect("text"),
        "GET is not answered at /only-post; POST are"
    );
}

#[test]
fn a_head_is_inserted_right_after_get_in_the_405_allow_list() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Get), "/both", |_| {
        Ok(Response::new(Status::OK))
    });
    server.route(Some(Method::Post), "/both", |_| {
        Ok(Response::new(Status::NO_CONTENT))
    });
    let response = Request::put(&server.url_of("/both").expect("url").to_string(), "x")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers().get("allow"), Some("GET, HEAD, POST"));
    assert_eq!(
        response.text().expect("text"),
        "PUT is not answered at /both; GET, HEAD, POST are"
    );
}

#[test]
fn a_path_no_route_or_mount_names_is_404_not_405() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Get), "/elsewhere", |_| {
        Ok(Response::new(Status::OK))
    });
    let response = get(&server, "/nothing");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert!(response.headers().get("allow").is_none());
}

#[test]
fn an_any_method_route_never_answers_405() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(None, "/any", |_| Ok(Response::new(Status::OK)));
    let response = Request::delete(&server.url_of("/any").expect("url").to_string())
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::OK);
}

#[test]
fn a_mount_covering_the_path_answers_what_its_routes_do_not() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.mount("/", memory_root()).expect("mount");
    server.route(Some(Method::Post), "/rpc", |_| {
        Ok(Response::new(Status::NO_CONTENT))
    });
    // GET at /rpc is not routed, but the root mount covers it: the mount's
    // own answer for an absent child, never the route's 405.
    let response = get(&server, "/rpc");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert!(response.headers().get("allow").is_none());
}

#[test]
fn a_head_no_route_names_is_answered_by_the_get_route() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Get), "/greeting", |_| {
        Ok(Response::new(Status::OK).with_text("hello"))
    });
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/greeting", ""));
    assert_eq!(status, Status::OK);
    assert_eq!(headers.content_length().expect("length"), Some(5));
    assert!(body.is_empty());
}

#[test]
fn an_explicit_head_route_wins_over_the_get_route() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Get), "/greeting", |_| {
        Ok(Response::new(Status::OK).with_text("hello"))
    });
    server.route(Some(Method::Head), "/greeting", |_| {
        Response::new(Status::OK).with_header("x-head", "yes")
    });
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/greeting", ""));
    assert_eq!(status, Status::OK);
    assert_eq!(headers.get("x-head"), Some("yes"));
    assert!(body.is_empty());
}

// --- written bodies and tracing options ---------------------------------------

#[test]
fn respond_with_a_written_response_writes_it_once_at_registration() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    server.respond(
        Some(Method::Get),
        "/written",
        Response::new(Status::OK).with_writer(move |body| {
            counter.fetch_add(1, Ordering::Relaxed);
            body.write_all(b"fixed")?;
            Ok(())
        }),
    );
    for _ in 0..3 {
        let response = get(&server, "/written");
        assert_eq!(response.status(), Status::OK);
        assert_eq!(&*response.bytes().expect("body"), b"fixed");
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "the writer ran once, at registration"
    );
}

#[test]
fn respond_with_a_writer_that_fails_answers_the_failure() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.respond(
        Some(Method::Get),
        "/broken",
        Response::new(Status::OK)
            .with_writer(|_body| Err(Error::Io(std::io::Error::other("no rows today")))),
    );
    let answered = get(&server, "/broken");
    assert_eq!(answered.status(), Status::INTERNAL_SERVER_ERROR);
    assert!(answered.text().expect("the text").contains("no rows today"));
}

#[test]
fn trace_is_none_by_default_and_set_by_with_trace() {
    assert!(ServerOptions::default().trace().is_none());
    let options = ServerOptions::default().with_trace(memory_root());
    assert!(options.trace().is_some());
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
