//! `rust/src/http/server/framed.rs`: what an answer is when HTTP/2 or HTTP/3
//! frames it - no field of one HTTP/1.1 connection, `Server`, `Date` and
//! the length wherever it is known.

use super::*;
use yggdryl::http::HttpVersion;

fn http2() -> Session {
    Session::with_options(HttpOptions::default().with_http_version(Some(HttpVersion::Http2)))
        .expect("a session asking for HTTP/2")
}

#[test]
fn a_framed_answer_carries_no_connection_field_and_states_its_length() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.respond(
        None,
        "/fields",
        Response::new(Status::OK)
            .with_header("Connection", "keep-alive")
            .unwrap()
            .with_header("Keep-Alive", "timeout=5")
            .unwrap()
            .with_header("X-Kept", "yes")
            .unwrap()
            .with_text("body"),
    );
    let response = http2()
        .get(&server.url_of("/fields").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http2);
    let headers = response.headers();
    for name in ["connection", "keep-alive", "transfer-encoding"] {
        assert_eq!(headers.get(name), None, "{name}");
    }
    assert_eq!(headers.get("x-kept"), Some("yes"));
    assert_eq!(headers.content_length().unwrap(), Some(4));
    assert!(headers.get("date").is_some());
    assert!(
        headers
            .get("server")
            .is_some_and(|server| server.starts_with("yggdryl/"))
    );
    assert_eq!(response.text().unwrap(), "body");
}

#[test]
fn a_framed_request_reaches_the_handler_with_its_authority_as_host() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(None, "/host", |request| {
        Ok(Response::new(Status::OK).with_text(request.headers().get("host").unwrap_or("none")))
    });
    let response = http2()
        .get(&server.url_of("/host?x=1").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(
        response.text().unwrap(),
        format!("127.0.0.1:{}", server.port())
    );
    let recorded = server.requests();
    assert_eq!(recorded[0].path, "/host");
    assert_eq!(recorded[0].target, "/host?x=1");
}
