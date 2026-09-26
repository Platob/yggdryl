//! `rust/src/http/stream.rs`: a body still on the wire, read forward and
//! re-opened from the delivered cursor when the transport dies under it.
//!
//! The fixture cuts an answer at a chosen byte, so every resume is
//! scripted and counted: a drain costs one `GET` plus one ranged `GET` per
//! resume, each carrying the cursor in `Range` and the first answer's
//! validator in `If-Range`.

use std::io::Read;

use yggdryl::http::{
    Fault, HttpOptions, Method, Request, Response, Server, Session, Status, Stream,
};
use yggdryl::{Error, IOBase, IOKind};

use crate::http_server::HttpServer;
use crate::http_server::RecordedExt as _;

/// A body of `len` bytes with no repeating window shorter than 251.
fn body(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

/// Another body of `len` bytes, differing from [`body`] at every byte.
fn other_body(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 241) as u8 ^ 0x80).collect()
}

fn open(server: &HttpServer, path: &str) -> Stream {
    Request::get(&server.url(path))
        .expect("a URL")
        .stream()
        .expect("a response")
        .into_stream()
        .expect("the live stream")
}

/// The rest of the body from the delivered cursor, read forward: what
/// resumes, where `read_all_bytes` is the whole body from its first byte.
fn rest(stream: &Stream) -> yggdryl::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = vec![0_u8; 4096];
    loop {
        let read = stream.pread(stream.delivered(), &mut buffer)?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

fn open_with(session: &Session, server: &HttpServer, path: &str) -> Stream {
    session
        .get(&server.url(path))
        .expect("a URL")
        .stream()
        .expect("a response")
        .into_stream()
        .expect("the live stream")
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_resumed_transfer_whose_validator_changed_is_a_conflict() {
    let server = HttpServer::start();
    server.put_resource("/r", &body(8192), Some("application/octet-stream"));
    server.cut_body_at("/r", 4096);
    let stream = open(&server, "/r");
    let mut head = [0_u8; 100];
    stream.pread_exact(0, &mut head).expect("the first bytes");
    assert_eq!(head, body(8192)[..100]);
    // The resource changes under the transfer: a different ETag.
    server.put_resource("/r", &other_body(8192), Some("application/octet-stream"));
    match rest(&stream).expect_err("a refusal") {
        Error::Conflict {
            expected,
            actual,
            path,
        } => {
            assert_eq!(expected, "resource");
            assert_eq!(actual, "changed resource");
            assert!(path.contains("/r"), "{path}");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), Some("bytes=4096-"));
    assert!(requests[1].header("if-range").is_some());
    // RFC 9110 answers a failed `If-Range` with the whole representation; its
    // validator is what says the resource moved under the transfer.
    assert_eq!(requests[1].status.code(), 200);
    assert_eq!(stream.delivered(), 4096);
}

#[test]
fn a_resumed_206_that_states_no_range_is_refused_rather_than_spliced() {
    // The first answer is whole, ranged and cut; the re-open answers `206`
    // with no `Content-Range`, so where its bytes belong is unknown.
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Get), "/r", |request| {
        let response = if request.headers().get("range").is_some() {
            Response::new(Status::PARTIAL_CONTENT).with_body(body(8192)[4096..].to_vec())
        } else {
            Response::new(Status::OK)
                .with_header("Accept-Ranges", "bytes")?
                .with_body(body(8192))
        };
        Ok(response)
    });
    server.inject("/r", Fault::CutBodyAt(4096), 1);
    let stream = Request::get(&server.url_of("/r").expect("url").to_string())
        .expect("a URL")
        .stream()
        .expect("a response")
        .into_stream()
        .expect("the live stream");
    match rest(&stream).expect_err("a refusal") {
        Error::Io(error) => assert!(error.to_string().contains("no Content-Range"), "{error}"),
        other => panic!("expected a refused splice, got {other:?}"),
    }
    assert_eq!(stream.delivered(), 4096);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn reading_behind_the_cursor_of_a_resource_without_ranges_is_refused() {
    let server = HttpServer::start();
    server.put_resource("/flat", &body(4096), Some("application/octet-stream"));
    server.set_ranges("/flat", false);
    let stream = open(&server, "/flat");
    let mut buffer = [0_u8; 8];
    assert_eq!(stream.pread(10, &mut buffer).expect("a forward read"), 8);
    assert_eq!(stream.delivered(), 18);
    match stream.pread(0, &mut buffer).expect_err("a refusal") {
        Error::Unsupported { operation, .. } => {
            assert!(operation.contains("without ranges"), "{operation}");
        }
        other => panic!("expected an unsupported read, got {other:?}"),
    }
    assert_eq!(server.request_count(), 1);
}

#[test]
fn consecutive_failures_past_max_attempts_fail_the_read() {
    let server = HttpServer::start();
    server.put_resource("/r", &body(8192), Some("application/octet-stream"));
    let session =
        Session::with_options(HttpOptions::default().with_max_attempts(2)).expect("a session");
    server.cut_body_at("/r", 4096);
    let stream = open_with(&session, &server, "/r");
    let mut delivered = vec![0_u8; 4096];
    stream
        .pread_exact(0, &mut delivered)
        .expect("what the server sent");
    // The re-open is cut before any byte: the second consecutive failure.
    server.cut_body_at("/r", 0);
    match rest(&stream).expect_err("a failure") {
        Error::Io(error) => assert!(
            error.to_string().contains("ended after") || error.to_string().contains("disconnected"),
            "{error}"
        ),
        other => panic!("expected a transport failure, got {other:?}"),
    }
    assert_eq!(server.request_count(), 2);
    assert_eq!(stream.resumes(), 1);
    assert_eq!(stream.delivered(), 4096);
}

#[test]
fn writes_are_refused() {
    let server = HttpServer::start();
    server.put_resource("/r", &body(16), Some("text/plain"));
    let mut stream = open(&server, "/r");
    assert!(matches!(
        stream.pwrite(0, b"x"),
        Err(Error::Unsupported {
            operation: "writing a response body",
            ..
        })
    ));
    assert!(stream.truncate(0).is_err());
    assert!(stream.reserve(1).is_err());
    assert_eq!(stream.kind(), IOKind::File);
    assert!(!stream.is_container());
    assert_eq!(
        stream.url().map(ToString::to_string),
        Some(server.url("/r"))
    );
    assert!(stream.mtime().is_some());
    assert_eq!(stream.media_type().base(), &yggdryl::MimeType::PLAIN_TEXT);
}

// --- resuming ----------------------------------------------------------------

#[test]
fn a_cut_body_resumes_from_the_delivered_cursor_with_one_request_per_resume() {
    let server = HttpServer::start();
    let bytes = body(8192);
    server.put_resource("/r", &bytes, Some("application/octet-stream"));
    server.cut_body_at("/r", 4096);
    // A session of its own, so the counters are this transfer's alone.
    let session = Session::new();
    let request = session.get(&server.url("/r")).expect("a URL");
    let mut stream = request
        .stream()
        .expect("a response")
        .into_stream()
        .expect("the live stream");
    let mut read = Vec::new();
    stream.read_to_end(&mut read).expect("a resumed drain");
    assert_eq!(read, bytes);
    assert_eq!(stream.delivered(), 8192);
    assert_eq!(stream.total(), Some(8192));
    assert_eq!(stream.resumes(), 1);
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "one GET plus one per resume");
    assert_eq!(requests[0].header("range"), None);
    assert_eq!(requests[1].header("range"), Some("bytes=4096-"));
    assert_eq!(
        requests[1].header("if-range"),
        stream.headers().get("etag"),
        "the first answer's strong ETag travels as If-Range"
    );
    assert_eq!(requests[1].status.code(), 206);
    let stats = request.stats();
    assert_eq!(stats.requests, 2);
    assert_eq!(stats.resumes, 1);
    assert_eq!(stats.gets, 2);
}

#[test]
fn a_transfer_that_keeps_moving_survives_every_cut() {
    let server = HttpServer::start();
    let bytes = body(5000);
    server.put_resource("/r", &bytes, Some("application/octet-stream"));
    // Each of the next five answers is cut after 1 KiB, and a byte arriving
    // resets the failure count, so four resumes follow under a limit of three
    // attempts; the fifth answer, a body of 904 bytes, fits under the cut and
    // completes the transfer.
    server.fail_after("/r", 5);
    let session = Session::new();
    let mut stream = open_with(&session, &server, "/r");
    let mut read = Vec::new();
    stream.read_to_end(&mut read).expect("a resumed drain");
    assert_eq!(read, bytes);
    assert_eq!(stream.resumes(), 4);
    assert_eq!(server.request_count(), 5, "one GET, four resumes");
    let stats = session.stats();
    assert_eq!(stats.requests, 5);
    assert_eq!(stats.resumes, 4);
    assert_eq!(stats.retries, 0);
}

#[test]
fn delivered_is_the_resume_cursor_and_pread_moves_it() {
    let server = HttpServer::start();
    let bytes = body(8192);
    server.put_resource("/r", &bytes, Some("application/octet-stream"));
    let stream = open(&server, "/r");
    let mut buffer = [0_u8; 16];
    assert_eq!(stream.pread(0, &mut buffer).expect("a read"), 16);
    assert_eq!(stream.delivered(), 16);
    // Forward: discarded up to the position, no request.
    assert_eq!(stream.pread(4096, &mut buffer).expect("a read"), 16);
    assert_eq!(buffer, bytes[4096..4112]);
    assert_eq!(stream.delivered(), 4112);
    assert_eq!(server.request_count(), 1);
    // Behind: one ranged GET from the position, and the cursor follows.
    assert_eq!(stream.pread(8, &mut buffer[..8]).expect("a read"), 8);
    assert_eq!(buffer[..8], bytes[8..16]);
    assert_eq!(stream.delivered(), 16);
    assert_eq!(stream.resumes(), 1);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("range"), Some("bytes=8-"));
    // Past the end: empty, and the cursor stands there.
    assert_eq!(stream.pread(10_000, &mut buffer).expect("a read"), 0);
    assert_eq!(stream.size(), 8192);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn read_all_bytes_and_pstream_bytes_drain_in_one_request() {
    let server = HttpServer::start();
    let bytes = body(10_000);
    server.put_resource("/r", &bytes, Some("application/octet-stream"));
    let stream = open(&server, "/r");
    let drained: Vec<u8> = stream
        .pstream_bytes(0, 999)
        .expect("a byte stream")
        .flat_map(|chunk| chunk.expect("a chunk"))
        .collect();
    assert_eq!(drained, bytes);
    assert_eq!(stream.delivered(), 10_000);
    assert_eq!(server.request_count(), 1);

    let stream = open(&server, "/r");
    assert_eq!(stream.read_all_bytes().expect("drained"), bytes);
    assert_eq!(server.request_count(), 2);
    // A whole read is the whole body from its first byte, whatever the
    // cursor: once drained, one ranged `GET` fetches it again.
    assert_eq!(stream.read_all_bytes().expect("again"), bytes);
    assert_eq!(server.request_count(), 3);
    assert_eq!(server.requests()[2].header("range"), Some("bytes=0-"));
}

#[test]
fn a_whole_read_after_a_forward_read_without_ranges_is_refused_not_truncated() {
    let server = HttpServer::start();
    server.put_resource("/flat", &body(4096), Some("application/octet-stream"));
    server.set_ranges("/flat", false);
    let stream = open(&server, "/flat");
    let mut head = [0_u8; 10];
    stream.pread_exact(0, &mut head).expect("a forward read");
    match stream.read_all_bytes().expect_err("a refusal") {
        Error::Unsupported { operation, .. } => {
            assert!(operation.contains("without ranges"), "{operation}");
        }
        other => panic!("expected an unsupported read, got {other:?}"),
    }
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_whole_read_holds_no_more_than_max_body_size() {
    let server = HttpServer::start();
    server.put_resource("/r", &body(4096), Some("application/octet-stream"));
    let session =
        Session::with_options(HttpOptions::default().with_max_body_size(1000)).expect("a session");
    let stream = open_with(&session, &server, "/r");
    match stream.read_all_bytes().expect_err("past the bound") {
        Error::Io(error) => assert!(error.to_string().contains("max_body_size"), "{error}"),
        other => panic!("expected an oversized body, got {other:?}"),
    }
}

#[test]
fn a_chunked_body_streams_without_a_stated_length() {
    let server = HttpServer::start();
    let bytes = body(10_000);
    server.put_resource("/c", &bytes, Some("application/octet-stream"));
    server.set_chunked("/c", true);
    let mut stream = open(&server, "/c");
    assert_eq!(stream.total(), None);
    assert_eq!(stream.size(), 0, "nothing stated, nothing delivered yet");
    let mut read = Vec::new();
    stream.read_to_end(&mut read).expect("a drain");
    assert_eq!(read, bytes);
    assert_eq!(
        stream.size(),
        10_000,
        "the delivered count once nothing is stated"
    );
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_held_body_streams_over_its_bytes() {
    let server = HttpServer::start();
    let bytes = body(300);
    server.put_resource("/h", &bytes, Some("text/plain"));
    let stream = Request::get(&server.url("/h"))
        .expect("a URL")
        .send()
        .expect("a response")
        .into_stream()
        .expect("a stream over the bytes");
    assert_eq!(stream.total(), Some(300));
    assert_eq!(stream.headers().get("content-type"), Some("text/plain"));
    let mut buffer = [0_u8; 10];
    assert_eq!(stream.pread(290, &mut buffer).expect("a read"), 10);
    assert_eq!(buffer, bytes[290..]);
    // Behind the cursor costs nothing over held bytes.
    assert_eq!(stream.pread(0, &mut buffer).expect("a read"), 10);
    assert_eq!(buffer, bytes[..10]);
    assert_eq!(stream.resumes(), 0);
    assert_eq!(
        stream.url().map(ToString::to_string),
        Some(server.url("/h"))
    );
    assert_eq!(server.request_count(), 1);
}

#[cfg(feature = "internals")]
mod internal {
    //! The ranged door the `Request` leaf reads through, which no caller can
    //! reach: one ranged `GET` as a stream over the asked window.

    use std::io::Read;

    use yggdryl::http::Request;
    use yggdryl::internals::http_stream::stream_range;

    use super::body;
    use crate::http_server::HttpServer;
    use crate::http_server::RecordedExt as _;

    #[test]
    fn a_206_answer_streams_the_asked_window() {
        let server = HttpServer::start();
        let bytes = body(8192);
        server.put_resource("/r", &bytes, Some("application/octet-stream"));
        let request = Request::get(&server.url("/r")).expect("a URL");
        let mut stream = stream_range(&request, 100, Some(199)).expect("a window");
        let mut read = Vec::new();
        stream.read_to_end(&mut read).expect("the window");
        assert_eq!(read, bytes[100..200]);
        assert_eq!(stream.total(), Some(8192), "the Content-Range total");
        assert_eq!(stream.delivered(), 100);
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].header("range"), Some("bytes=100-199"));
        assert_eq!(requests[0].status.code(), 206);
    }

    #[test]
    fn a_200_answer_to_a_range_skips_into_the_body() {
        let server = HttpServer::start();
        let bytes = body(8192);
        server.put_resource("/flat", &bytes, Some("application/octet-stream"));
        server.set_ranges("/flat", false);
        let request = Request::get(&server.url("/flat")).expect("a URL");
        let mut stream = stream_range(&request, 100, Some(199)).expect("a window");
        let mut read = Vec::new();
        stream.read_to_end(&mut read).expect("the window");
        assert_eq!(
            read,
            bytes[100..200],
            "the prefix skipped, the window bounded"
        );
        assert_eq!(stream.total(), Some(8192), "the Content-Length");
        assert_eq!(server.requests()[0].status.code(), 200);
        // An open window from a 200 skips and reads to the end.
        let mut stream = stream_range(&request, 8000, None).expect("a window");
        let mut read = Vec::new();
        stream.read_to_end(&mut read).expect("the rest");
        assert_eq!(read, bytes[8000..]);
        assert_eq!(server.request_count(), 2);
    }

    #[test]
    fn a_resumed_window_carries_the_cursor_within_it() {
        let server = HttpServer::start();
        let bytes = body(8192);
        server.put_resource("/r", &bytes, Some("application/octet-stream"));
        server.cut_body_at("/r", 1000);
        let request = Request::get(&server.url("/r")).expect("a URL");
        let mut stream = stream_range(&request, 1000, Some(4999)).expect("a window");
        let mut read = Vec::new();
        stream.read_to_end(&mut read).expect("the window");
        assert_eq!(read, bytes[1000..5000]);
        assert_eq!(stream.resumes(), 1);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].header("range"), Some("bytes=1000-4999"));
        assert_eq!(requests[1].header("range"), Some("bytes=2000-4999"));
    }
}
