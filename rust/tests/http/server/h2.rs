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
fn a_body_refused_over_the_bound_is_traced_with_its_413() {
    let (dir, trace) = trace_folder("h2-413");
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default()
            .with_max_body_size(1000)
            .with_trace(trace),
    )
    .expect("bind");
    server.route(None, "/sink", |_| Ok(Response::new(Status::OK)));
    let target = server.url_of("/sink").unwrap().to_string();
    let refused = asking(HttpVersion::Http2)
        .post(&target, vec![0_u8; 5000])
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(refused.status().code(), 413);
    let answer = wait_for(&dir.join("0000-response.http"), |bytes| {
        bytes.starts_with(b"HTTP/2 413") && bytes.ends_with(b"bytes")
    });
    let request = std::fs::read(dir.join("0000-request.http")).expect("the request file");
    assert!(
        request.starts_with(b"POST /sink HTTP/2\r\n"),
        "{}",
        String::from_utf8_lossy(&request)
    );
    assert!(String::from_utf8_lossy(&answer).contains("request body longer than 1000 bytes"));
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
fn a_written_body_over_http2_arrives_whole_with_no_content_length() {
    let server = served();
    server.route(Some(Method::Get), "/written", |_| {
        Ok(Response::new(Status::OK).with_writer(|body| {
            body.write_all(b"streamed-over-h2")?;
            Ok(())
        }))
    });
    let response = asking(HttpVersion::Http2)
        .get(&server.url_of("/written").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http2);
    assert!(response.headers().get("content-length").is_none());
    assert_eq!(&*response.bytes().unwrap(), b"streamed-over-h2");
}

#[test]
fn a_traced_http2_exchange_is_written_as_the_http11_messages_its_frames_carried() {
    let trace_dir =
        std::env::temp_dir().join(format!("yggdryl-http-h2-trace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&trace_dir);
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_trace(Holder::folder(&trace_dir).expect("a trace folder")),
    )
    .expect("bind");
    server.respond(None, "/x", Response::new(Status::OK).with_text("ok"));
    let response = asking(HttpVersion::Http2)
        .get(&server.url_of("/x").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.text().unwrap(), "ok");
    // The files complete a moment after the client read the last byte, on
    // the connection's own task.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && !std::fs::read(trace_dir.join("0000-response.http"))
            .is_ok_and(|bytes| bytes.ends_with(b"ok"))
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let request = std::fs::read(trace_dir.join("0000-request.http")).expect("request trace");
    assert!(
        request.starts_with(b"GET /x HTTP/2\r\n"),
        "{}",
        String::from_utf8_lossy(&request)
    );
    let answer = std::fs::read(trace_dir.join("0000-response.http")).expect("response trace");
    assert!(
        answer.starts_with(b"HTTP/2 200 OK\r\n"),
        "{}",
        String::from_utf8_lossy(&answer)
    );
    assert!(
        answer.ends_with(b"ok"),
        "{}",
        String::from_utf8_lossy(&answer)
    );
    let _ = std::fs::remove_dir_all(&trace_dir);
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

/// The text an HTTP/2 request in the clear, by prior knowledge, answers
/// when it states `uri` whole - its `:scheme` and `:authority` included,
/// which the crate's own client never sends over plain TCP as `https`.
fn h2c_text(server: &Server, uri: &str) -> String {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let tcp = tokio::net::TcpStream::connect(server.address())
            .await
            .expect("connect");
        let (client, connection) = ::h2::client::handshake(tcp).await.expect("a handshake");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut client = client.ready().await.expect("ready");
        let request = ureq::http::Request::get(uri).body(()).expect("a request");
        let (response, _) = client.send_request(request, true).expect("sent");
        let mut body = response.await.expect("an answer").into_body();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.data().await {
            let chunk = chunk.expect("a chunk");
            let _ = body.flow_control().release_capacity(chunk.len());
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).expect("UTF-8")
    })
}

#[test]
fn a_cleartext_scheme_of_https_is_believed_from_a_trusted_proxy_alone() {
    let echo = |server: &Server| {
        server.route(None, "/echo", |request| {
            Ok(Response::new(Status::OK).with_text(&request.url().to_string()))
        });
    };
    // Any peer may write `:scheme https` over plain TCP; the connection
    // is not TLS, so the request was made under `http`, as an HTTP/1.1
    // absolute-form `https://` target is.
    let plain = Server::bind("127.0.0.1:0").expect("bind");
    echo(&plain);
    assert_eq!(
        h2c_text(&plain, "https://pub.example/echo"),
        "http://pub.example/echo"
    );
    assert_eq!(
        h2c_text(&plain, "http://pub.example/echo"),
        "http://pub.example/echo"
    );

    // A trusted proxy speaking HTTP/2 onward after ending TLS states the
    // scheme the client used there, as it would in `X-Forwarded-Proto`.
    let proxied = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default()
            .with_trusted_proxies(["127.0.0.1"])
            .expect("a network"),
    )
    .expect("bind");
    echo(&proxied);
    assert_eq!(
        h2c_text(&proxied, "https://pub.example/echo"),
        "https://pub.example/echo"
    );
    assert_eq!(
        h2c_text(&proxied, "http://pub.example/echo"),
        "http://pub.example/echo"
    );
}
