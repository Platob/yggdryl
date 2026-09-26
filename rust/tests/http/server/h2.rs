//! `rust/src/http/server/h2.rs`: HTTP/2 on the server's TCP port - by prior
//! knowledge beside HTTP/1.1 on the same port, and over TLS when the server
//! holds a certificate.

use super::*;
use yggdryl::http::HttpVersion;

fn asking(version: HttpVersion) -> Session {
    Session::with_options(HttpOptions::default().with_http_version(Some(version)))
        .expect("a session")
}

#[test]
fn one_port_answers_http11_and_http2_alike() {
    let server = served();
    let target = server.url_of("/rows.json").unwrap().to_string();
    let one = asking(HttpVersion::Http11)
        .get(&target)
        .unwrap()
        .send()
        .unwrap();
    let two = asking(HttpVersion::Http2)
        .get(&target)
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(one.version(), HttpVersion::Http11);
    assert_eq!(two.version(), HttpVersion::Http2);
    assert_eq!(&*one.bytes().unwrap(), ROWS);
    assert_eq!(&*two.bytes().unwrap(), ROWS);
    assert_eq!(one.headers().get("etag"), two.headers().get("etag"));
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_body_over_the_bound_is_413_on_its_own_stream() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_max_body_size(1000),
    )
    .expect("bind");
    server.route(None, "/sink", |_| Ok(Response::new(Status::OK)));
    let session = asking(HttpVersion::Http2);
    let target = server.url_of("/sink").unwrap().to_string();
    let refused = session
        .post(&target, vec![0_u8; 5000])
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(refused.status().code(), 413);
    // The connection serves the next stream.
    let fine = session
        .post(&target, vec![0_u8; 10])
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(fine.status(), Status::OK);
    assert_eq!(server.connections(), 1);
}

#[test]
fn a_path_nothing_answers_is_404_and_a_request_line_naming_http2_is_505() {
    let server = served();
    let missing = asking(HttpVersion::Http2)
        .get(&server.url_of("/nothing/here").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(missing.status(), Status::NOT_FOUND);
    // HTTP/2 opens with its preface, never with a request line naming it.
    let (status, _, _) = raw_head(&server, b"GET / HTTP/2\r\nHost: x\r\n\r\n");
    assert_eq!(status.code(), 505);
}

#[test]
fn a_connection_quiet_for_the_read_timeout_is_closed_and_the_client_opens_another() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_read_timeout(Duration::from_millis(200)),
    )
    .expect("bind");
    server.respond(None, "/ping", Response::new(Status::OK).with_text("pong"));
    let session = asking(HttpVersion::Http2);
    let target = server.url_of("/ping").unwrap().to_string();
    assert_eq!(
        session
            .get(&target)
            .unwrap()
            .send()
            .unwrap()
            .text()
            .unwrap(),
        "pong"
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        session
            .get(&target)
            .unwrap()
            .send()
            .unwrap()
            .text()
            .unwrap(),
        "pong"
    );
    assert_eq!(server.connections(), 2);
}

#[test]
fn tls_to_a_server_without_a_certificate_is_closed_unanswered() {
    let server = served();
    let error = Session::new()
        .get(&format!("https://127.0.0.1:{}/rows.json", server.port()))
        .unwrap()
        .send()
        .expect_err("no certificate to answer TLS with");
    assert!(!error.to_string().is_empty());
    assert_eq!(server.request_count(), 0);
}
