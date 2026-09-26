//! `rust/src/http/framed.rs`: which version one request speaks - the
//! options' `http_version` read against what the client learned of the
//! origin, and HTTP/1.1 for anything that goes through a proxy.

use yggdryl::http::{HttpOptions, HttpVersion, Response, Server, ServerOptions, Session, Status};

fn greeting() -> Server {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.respond(None, "/hello", Response::new(Status::OK).with_text("hello"));
    server
}

fn asking(version: Option<HttpVersion>) -> HttpOptions {
    HttpOptions::default().with_http_version(version)
}

#[test]
fn a_plain_origin_speaks_what_was_asked_and_http11_when_negotiating() {
    let server = greeting();
    let target = server.url_of("/hello").unwrap().to_string();
    for (asked, spoken) in [
        (None, HttpVersion::Http11),
        (Some(HttpVersion::Http10), HttpVersion::Http11),
        (Some(HttpVersion::Http11), HttpVersion::Http11),
        (Some(HttpVersion::Http2), HttpVersion::Http2),
        // HTTP/3 needs TLS: a plain origin asked for it speaks HTTP/2.
        #[cfg(feature = "http3")]
        (Some(HttpVersion::Http3), HttpVersion::Http2),
    ] {
        let session = Session::with_options(asking(asked)).unwrap();
        let response = session.get(&target).unwrap().send().unwrap();
        assert_eq!(response.version(), spoken, "{asked:?}");
        assert_eq!(response.text().unwrap(), "hello");
    }
}

#[test]
fn a_request_through_a_proxy_speaks_http11_whatever_was_asked() {
    let origin = greeting();
    let proxy = Server::bind_with("127.0.0.1:0", ServerOptions::default().with_tunnel(true))
        .expect("a proxy");
    let session =
        Session::with_options(asking(Some(HttpVersion::Http2)).with_proxy(proxy.url().to_string()))
            .unwrap();
    let response = session
        .get(&origin.url_of("/hello").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http11);
    assert_eq!(response.text().unwrap(), "hello");
    assert!(
        proxy
            .requests()
            .iter()
            .any(|recorded| recorded.method.as_str() == "CONNECT"),
        "the request went through the proxy"
    );
}
