//! `rust/src/http/h3.rs`: HTTP/3 under the client - QUIC to an origin asked
//! for it, or one that advertised it in `Alt-Svc`, and back to HTTP/2 for a
//! while when QUIC cannot reach it.
//!
//! The crate's `Server` answers HTTP/3 on the UDP port beside its TCP one
//! under a certificate it signs for itself, and TLS on the TCP port under the
//! same one; the client trusts it by naming it as its CA bundle.

use std::path::PathBuf;
use std::time::Duration;

use yggdryl::IOBase;
use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{
    Fault, HttpOptions, HttpVersion, Method, Response, Server, ServerOptions, Session, Status,
};

/// A server answering HTTP/3, with a blob and a greeting.
fn served(bytes: &[u8]) -> Server {
    let server = Server::bind_with("127.0.0.1:0", ServerOptions::default().with_http3(true))
        .expect("bind with HTTP/3");
    let mut blob = Holder::Buffer(Buffer::new());
    blob.write_all_bytes(bytes).expect("the blob");
    server.mount("/blob", blob).expect("mount");
    server.respond(None, "/hello", Response::new(Status::OK).with_text("hello"));
    server
}

/// The server's certificate as a bundle file a client can name.
fn trusting(server: &Server) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "yggdryl-h3-{}-{}.pem",
        std::process::id(),
        server.port()
    ));
    std::fs::write(&path, server.certificate().expect("a certificate")).expect("the bundle");
    path
}

fn session(server: &Server, version: Option<HttpVersion>) -> Session {
    Session::with_options(
        HttpOptions::default()
            .with_ca_bundle(trusting(server))
            .with_http_version(version)
            .with_connect_timeout(Duration::from_secs(2)),
    )
    .expect("a session trusting the server")
}

fn https(server: &Server, path: &str) -> String {
    format!("https://127.0.0.1:{}{path}", server.port())
}

fn blob(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

#[test]
fn an_origin_asked_for_http3_is_reached_over_quic() {
    let server = served(b"");
    let response = session(&server, Some(HttpVersion::Http3))
        .get(&https(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http3);
    assert_eq!(response.text().unwrap(), "hello");
    // An HTTP/3 answer advertises nothing: it is the alternative.
    assert_eq!(response.headers().get("alt-svc"), None);
    // A QUIC connection is counted with the TCP port's.
    assert_eq!(server.connections(), 1);
}

#[test]
fn negotiating_goes_h2_by_alpn_then_h3_by_alt_svc() {
    let server = served(b"");
    let session = session(&server, None);
    let first = session
        .get(&https(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(first.version(), HttpVersion::Http2);
    assert_eq!(
        first.headers().get("alt-svc"),
        Some(format!("h3=\":{}\"; ma=86400", server.port()).as_str())
    );
    let second = session
        .get(&https(&server, "/hello"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(second.version(), HttpVersion::Http3);
    assert_eq!(second.text().unwrap(), "hello");
}

#[test]
fn a_body_goes_out_and_comes_back_over_quic() {
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_http3(true)).expect("bind");
    server.route(Some(Method::Post), "/echo", |request| {
        Ok(Response::new(Status::OK).with_body(request.body().as_bytes().to_vec()))
    });
    let payload = blob(3 << 20);
    let response = session(&server, Some(HttpVersion::Http3))
        .post(&https(&server, "/echo"), payload.clone())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http3);
    assert_eq!(&*response.bytes().unwrap(), payload.as_slice());
}

#[test]
fn a_stream_stopped_mid_body_resumes_from_the_delivered_byte() {
    let bytes = blob(1 << 20);
    let server = served(&bytes);
    server.inject("/blob", Fault::CutBodyAt(400_000), 1);
    let session = session(&server, Some(HttpVersion::Http3));
    let response = session
        .get(&https(&server, "/blob"))
        .unwrap()
        .stream()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http3);
    assert_eq!(response.read_all_bytes().unwrap(), bytes);
    // A stopped stream discards what the client had not read yet: the body
    // resumes from the byte handed over - or, when the stop overtook the
    // head, the request is sent again whole.
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let stats = session.stats();
    assert_eq!(stats.resumes + stats.retries, 1, "{stats:?}");
    if let Some(range) = requests[1].headers.get("range") {
        assert!(
            range.starts_with("bytes=") && range.ends_with('-'),
            "{range}"
        );
    }
}

#[test]
fn an_alternative_quic_cannot_reach_is_left_for_http2_in_the_same_attempt() {
    let server = served(b"");
    // A UDP port nothing answers on, advertised by a route of its own.
    let closed = std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|socket| socket.local_addr())
        .expect("a free port")
        .port();
    server.respond(
        None,
        "/away",
        Response::new(Status::OK)
            .with_header("alt-svc", &format!("h3=\":{closed}\""))
            .unwrap()
            .with_text("away"),
    );
    let session = Session::with_options(
        HttpOptions::default()
            .with_ca_bundle(trusting(&server))
            .with_connect_timeout(Duration::from_millis(500)),
    )
    .unwrap();
    let first = session
        .get(&https(&server, "/away"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(first.version(), HttpVersion::Http2);
    // The second tries QUIC, gives up at the connect timeout and answers
    // over HTTP/2; the third does not try again.
    for _ in 0..2 {
        let answer = session
            .get(&https(&server, "/away"))
            .unwrap()
            .send()
            .unwrap();
        assert_eq!(answer.version(), HttpVersion::Http2);
        assert_eq!(answer.text().unwrap(), "away");
    }
}

#[test]
fn a_server_without_http3_holds_no_certificate() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    assert!(server.certificate().is_none());
    let with = served(b"");
    let pem = with.certificate().expect("a certificate");
    assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"), "{pem}");
}
