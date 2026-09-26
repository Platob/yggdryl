//! `rust/src/http/mod.rs`: the doors at the module root, the process-wide
//! default session and the `located` holders, against the loopback server.

use std::time::Duration;

use yggdryl::holder::Holder;
use yggdryl::http::{self, Body, HttpOptions};
use yggdryl::{Error, IOBase, MimeType};

use crate::http_server::HttpServer;
use crate::http_server::RecordedExt as _;

// --- refusals ----------------------------------------------------------------

#[test]
fn every_door_refuses_a_url_of_another_scheme() {
    let refused = |result: yggdryl::Result<http::Response>| {
        assert!(
            matches!(
                result,
                Err(Error::Parse {
                    target: "http url",
                    ..
                })
            ),
            "{result:?}"
        );
    };
    refused(http::get("ftp://x/y"));
    refused(http::head("ftp://x/y"));
    refused(http::post("ftp://x/y", Body::from("b")));
    refused(http::put("ftp://x/y", Body::from("b")));
    refused(http::patch("ftp://x/y", Body::from("b")));
    refused(http::delete("ftp://x/y"));
    assert!(matches!(
        http::located("relative/path"),
        Err(Error::Parse {
            target: "http url",
            ..
        })
    ));
}

// --- the default session -----------------------------------------------------

#[test]
fn the_default_session_is_one_session_for_the_process() {
    let first = http::session();
    let second = http::session();
    assert_eq!(first.stats(), second.stats());
    assert_eq!(first.options().timeout(), HttpOptions::DEFAULT_TIMEOUT);
    // A request built without a session rides on it.
    let request = http::Request::get("http://127.0.0.1:1/x").unwrap();
    assert_eq!(request.session().stats(), first.stats());
}

#[test]
fn session_with_builds_a_session_for_the_options() {
    let session = http::session_with(
        HttpOptions::default()
            .with_timeout(Duration::from_secs(7))
            .with_max_attempts(1),
    )
    .unwrap();
    assert_eq!(session.options().timeout(), Duration::from_secs(7));
    assert_eq!(session.options().max_attempts(), 1);
    assert_eq!(session.stats().requests, 0);
}

// --- the doors ---------------------------------------------------------------

#[test]
fn the_doors_send_on_the_default_session() {
    let server = HttpServer::start();
    server.put_resource("/thing", b"thing", Some("text/plain"));
    // A scripted path echoes a POST or PATCH body; the bare mount refuses them.
    server.echo("/thing");
    let url = server.url("/thing");
    let before = http::session().stats();

    let got = http::get(&url).unwrap();
    assert_eq!(got.status().code(), 200);
    assert_eq!(got.text().unwrap(), "thing");

    let head = http::head(&url).unwrap();
    assert_eq!(head.status().code(), 200);
    assert_eq!(head.content_length(), Some(5));
    assert_eq!(&*head.bytes().unwrap(), b"");

    let posted = http::post(&url, Body::from("p")).unwrap();
    assert_eq!(posted.text().unwrap(), "p");
    let patched = http::patch(&url, Body::from("q")).unwrap();
    assert_eq!(patched.text().unwrap(), "q");

    let put = http::put(&url, Body::from("replaced")).unwrap();
    assert_eq!(put.status().code(), 204);
    assert_eq!(server.resource("/thing").unwrap().0, b"replaced".to_vec());

    let deleted = http::delete(&url).unwrap();
    assert_eq!(deleted.status().code(), 204);
    assert!(server.resource("/thing").is_none());
    let gone = http::delete(&url).unwrap();
    assert_eq!(gone.status().code(), 404);

    let methods: Vec<String> = server
        .requests()
        .iter()
        .map(|recorded| recorded.method.to_string())
        .collect();
    assert_eq!(
        methods,
        ["GET", "HEAD", "POST", "PATCH", "PUT", "DELETE", "DELETE"]
    );
    // The default session counted them, whatever else this process sent on it.
    assert!(http::session().stats().requests >= before.requests + 7);
}

// --- located -----------------------------------------------------------------

#[test]
fn located_holds_the_resource_as_a_request_and_costs_nothing() {
    let server = HttpServer::start();
    server.put_resource("/lake/part.json", br#"{"a":1}"#, Some("application/json"));

    let held = http::located(&server.url("/lake/part.json")).unwrap();
    let Holder::HttpRequest(request) = &held else {
        panic!("expected a request, got {held:?}");
    };
    assert_eq!(request.url().to_string(), server.url("/lake/part.json"));
    assert_eq!(held.media_type().base(), &MimeType::JSON);
    assert_eq!(server.request_count(), 0);

    assert_eq!(held.read_all_bytes().unwrap(), br#"{"a":1}"#);
    assert_eq!(held.size(), 7);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn located_with_reads_its_options() {
    let server = HttpServer::start();
    server.put_resource("/private", b"secret", Some("text/plain"));
    server.require_basic("/private", "u", "p");
    let options = HttpOptions::default()
        .with_authorization(http::Authorization::basic("u", "p"))
        .with_header("X-Trace", "held")
        .unwrap()
        .with_timeout(Duration::from_secs(9));

    let held = http::located_with(&server.url("/private"), options).unwrap();

    assert_eq!(held.read_all_bytes().unwrap(), b"secret");
    let Holder::HttpRequest(request) = &held else {
        panic!("expected a request, got {held:?}");
    };
    assert_eq!(
        request.session().options().timeout(),
        Duration::from_secs(9)
    );
    assert_eq!(server.requests()[0].header("x-trace"), Some("held"));
    // Its own session, its own counters.
    assert_eq!(request.stats().requests, 1);
}
