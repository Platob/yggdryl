//! `rust/src/http/h2.rs`: HTTP/2 under the client - one multiplexed
//! connection per origin, spoken with prior knowledge to a plain origin that
//! is asked for it, and a plain origin that does not read the preface
//! remembered as HTTP/1.1.
//!
//! Every case runs against the crate's own `Server`, which answers a
//! connection opening with the HTTP/2 preface as HTTP/2 on the same port it
//! answers HTTP/1.1 on.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use yggdryl::IOBase;
use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{Fault, HttpOptions, HttpVersion, Method, Response, Server, Session, Status};

fn http2() -> Session {
    Session::with_options(HttpOptions::default().with_http_version(Some(HttpVersion::Http2)))
        .expect("a session asking for HTTP/2")
}

fn blob(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn served(bytes: &[u8]) -> Server {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let mut blob = Holder::Buffer(Buffer::new());
    blob.write_all_bytes(bytes).expect("the blob");
    server.mount("/blob", blob).expect("mount");
    server.respond(None, "/hello", Response::new(Status::OK).with_text("hello"));
    server
}

fn url(server: &Server, path: &str) -> String {
    server.url_of(path).expect("a URL").to_string()
}

#[test]
fn a_plain_origin_asked_for_http2_is_spoken_to_with_prior_knowledge() {
    let server = served(b"");
    let response = http2()
        .get(&url(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.version(), HttpVersion::Http2);
    assert_eq!(response.text().unwrap(), "hello");
    // Negotiating, a plain origin is spoken to in HTTP/1.1.
    let negotiated = Session::with_options(HttpOptions::default())
        .unwrap()
        .get(&url(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(negotiated.version(), HttpVersion::Http11);
}

#[test]
fn every_request_to_one_origin_shares_one_connection() {
    let server = served(b"");
    let session = http2();
    let requests: Vec<_> = (0..48)
        .map(|_| session.get(&url(&server, "/hello")).unwrap())
        .collect();
    let answered = session
        .send_all(requests, Some(8))
        .filter(|answer| {
            answer
                .as_ref()
                .is_ok_and(|answer| answer.version() == HttpVersion::Http2)
        })
        .count();
    assert_eq!(answered, 48);
    assert_eq!(
        server.connections(),
        1,
        "streams multiplexed on one connection"
    );
}

#[test]
fn a_body_goes_out_under_flow_control_and_comes_back_whole() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Post), "/echo", |request| {
        Ok(Response::new(Status::OK).with_body(request.body().as_bytes().to_vec()))
    });
    // Larger than the initial windows of both directions.
    let payload = blob(3 << 20);
    let response = http2()
        .post(&url(&server, "/echo"), payload.clone())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http2);
    assert_eq!(&*response.bytes().unwrap(), payload.as_slice());
}

#[test]
fn a_body_read_from_a_reader_streams_out() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(Some(Method::Put), "/upload", |request| {
        Ok(Response::new(Status::OK).with_text(&request.body().len().to_string()))
    });
    let payload = blob(200_000);
    let request = http2().put(&url(&server, "/upload"), Vec::new()).unwrap();
    let response = request
        .send_reader(&mut payload.as_slice(), payload.len() as u64)
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http2);
    assert_eq!(response.text().unwrap(), "200000");
}

#[test]
fn a_head_states_the_length_and_carries_no_body() {
    let bytes = blob(10_000);
    let server = served(&bytes);
    let response = http2()
        .head(&url(&server, "/blob"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http2);
    assert_eq!(response.headers().content_length().unwrap(), Some(10_000));
    assert!(response.bytes().unwrap().is_empty());
}

#[test]
fn a_stream_reset_mid_body_resumes_from_the_delivered_byte() {
    let bytes = blob(1 << 20);
    let server = served(&bytes);
    server.inject("/blob", Fault::CutBodyAt(300_000), 1);
    let session = http2();
    let response = session
        .get(&url(&server, "/blob"))
        .unwrap()
        .stream()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http2);
    assert_eq!(response.read_all_bytes().unwrap(), bytes);
    // A reset discards the frames the client had not read yet: the body
    // resumes from the byte handed over, at or before the cut - or, when
    // the reset overtook the head, the request is sent again whole.
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "one GET plus one more");
    let stats = session.stats();
    assert_eq!(stats.resumes + stats.retries, 1, "{stats:?}");
    if stats.resumes == 1 {
        let range = requests[1].headers.get("range").expect("a ranged GET");
        let start: u64 = range
            .strip_prefix("bytes=")
            .and_then(|rest| rest.strip_suffix('-'))
            .and_then(|start| start.parse().ok())
            .expect("an open range");
        assert!(start <= 300_000, "{range}");
    }
}

#[test]
fn a_stream_closed_without_an_answer_is_retried_for_an_idempotent_method() {
    let server = served(b"");
    server.inject("/hello", Fault::CloseBeforeAnswer, 1);
    let session = http2();
    let response = session
        .get(&url(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.text().unwrap(), "hello");
    assert_eq!(session.stats().retries, 1);
    // The reset closed one stream, not the connection.
    assert_eq!(server.connections(), 1);
}

#[test]
fn a_plain_origin_that_does_not_speak_http2_is_remembered_as_http1() {
    // An HTTP/1.1 server that answers the preface as the malformed request
    // line it is to one, and everything else with `ok`.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().unwrap();
    let prefaces = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&prefaces);
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut connection) = connection else {
                return;
            };
            let mut head = [0_u8; 3];
            if connection.read_exact(&mut head).is_err() {
                continue;
            }
            let answer: &[u8] = if &head == b"PRI" {
                seen.fetch_add(1, Ordering::SeqCst);
                b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            } else {
                b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
            };
            let _ = connection.write_all(answer);
        }
    });
    let session = http2();
    let target = format!("http://{address}/");
    for _ in 0..3 {
        let response = session.get(&target).unwrap().send().unwrap();
        assert_eq!(response.version(), HttpVersion::Http11);
        assert_eq!(response.text().unwrap(), "ok");
    }
    assert_eq!(
        prefaces.load(Ordering::SeqCst),
        1,
        "the refusal is learned once"
    );
}

#[test]
fn a_deadline_bounds_an_http2_attempt_as_it_does_an_http1_one() {
    let server = served(b"");
    // The connection is opened and spoken over first, so the deadline is
    // spent on the answer alone.
    let session = http2();
    let warm = session
        .get(&url(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(warm.version(), HttpVersion::Http2);
    server.inject("/hello", Fault::Delay(Duration::from_secs(2)), 1);
    let started = Instant::now();

    let error = session
        .get(&url(&server, "/hello"))
        .unwrap()
        .with_deadline(Duration::from_millis(300))
        .with_max_attempts(1)
        .send()
        .expect_err("the answer comes after the deadline");

    assert!(
        started.elapsed() < Duration::from_millis(1_500),
        "{error:?}"
    );
    let yggdryl::Error::Io(io) = &error else {
        panic!("expected a timeout, got {error:?}");
    };
    assert_eq!(io.kind(), std::io::ErrorKind::TimedOut, "{io:?}");
}
