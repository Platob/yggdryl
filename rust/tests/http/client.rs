//! `rust/src/http/client.rs`: the pooled client, its counters and the retry
//! loop, against the loopback server.

use std::time::Duration;

use yggdryl::holder::Holder;
use yggdryl::http::{Body, Client, HttpOptions, Session, StatsSnapshot};
use yggdryl::{Error, IOBase, IOKind};

use crate::http_server::RecordedExt as _;
use crate::http_server::{HttpServer, Recorded};

/// One recorded request's header, over whichever shape the fixture records.
fn header<'a>(recorded: &'a Recorded, name: &str) -> Option<&'a str> {
    recorded.header(name)
}

fn statuses(server: &HttpServer) -> Vec<u16> {
    server
        .requests()
        .iter()
        .map(|recorded| recorded.status.code())
        .collect()
}

fn session(options: HttpOptions) -> Session {
    Session::with_options(options).expect("a session over the default transport")
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_proxy_that_is_not_a_url_is_refused_naming_the_option() {
    let error = Client::with_options(&HttpOptions::default().with_proxy("not a proxy ::"))
        .expect_err("a refusal");
    match error {
        Error::Parse { target, reason, .. } => {
            assert_eq!(target, "http option");
            assert!(reason.contains("proxy"), "{reason}");
        }
        other => panic!("expected a parse refusal, got {other:?}"),
    }
}

#[test]
fn a_ca_bundle_that_cannot_be_read_is_refused() {
    let error = Client::with_options(
        &HttpOptions::default().with_ca_bundle("/nonexistent/yggdryl-bundle.pem"),
    )
    .expect_err("a refusal");
    assert!(matches!(error, Error::Io(_)), "{error:?}");
    assert!(error.to_string().contains("yggdryl-bundle.pem"), "{error}");
}

#[test]
fn a_relative_child_of_a_client_is_refused() {
    let error = Client::new()
        .child_by_path("items/1")
        .expect_err("a client has no base to resolve against");
    assert!(
        matches!(
            error,
            Error::Parse {
                target: "http url",
                ..
            }
        ),
        "{error:?}"
    );
}

// --- construction ------------------------------------------------------------

#[test]
fn building_a_client_costs_no_request_and_reports_the_budget() {
    let client = Client::new();
    let stats = client.stats();
    assert_eq!(stats.requests, 0);
    assert_eq!(stats.retries, 0);
    assert!(stats.retry_tokens > 0);
    assert_eq!(StatsSnapshot::default().retry_tokens, 0);
    assert_eq!(Client::default().stats().requests, 0);
    // A client's session reads the client's counters.
    assert_eq!(client.session().stats(), client.stats());
}

#[test]
fn a_client_is_a_container_with_no_location() {
    let mut client = Client::new();
    assert_eq!(client.kind(), IOKind::Directory);
    assert!(client.is_container());
    assert!(!client.is_atomic());
    assert_eq!(client.url(), None);
    assert_eq!(client.uri(), None);
    assert_eq!(client.size(), 0);
    assert_eq!(client.pread(0, &mut [0; 4]).unwrap(), 0);
    assert!(client.ls(true, true).next().is_none());
    assert_eq!(client.media_type().base(), &yggdryl::MimeType::DIRECTORY);
    let refused = client
        .pwrite(0, b"x")
        .expect_err("a container takes no bytes");
    match refused {
        Error::Io(error) => assert_eq!(error.kind(), std::io::ErrorKind::IsADirectory),
        other => panic!("expected an I/O refusal, got {other:?}"),
    }
    assert!(client.truncate(0).is_ok());
    assert!(client.truncate(1).is_err());
    assert!(client.clear().is_ok());
    assert!(client.remove(true).is_ok());
    let child = client
        .child_by_path("http://127.0.0.1:1/items/1.json")
        .unwrap();
    match &child {
        Holder::HttpRequest(request) => {
            assert_eq!(request.url().to_string(), "http://127.0.0.1:1/items/1.json");
        }
        other => panic!("expected a request, got {other:?}"),
    }
    // Resolving a child costs nothing.
    assert_eq!(client.stats().requests, 0);
}

// --- retries -----------------------------------------------------------------

#[test]
fn a_retryable_status_is_retried_after_its_retry_after() {
    let server = HttpServer::start();
    server.put_resource("/flaky", b"eventually", Some("text/plain"));
    server.fail_status("/flaky", 503, Some("0"), 2);
    let session = session(HttpOptions::default().with_max_attempts(3));

    let response = session.get(&server.url("/flaky")).unwrap().send().unwrap();

    assert_eq!(response.status().code(), 200);
    assert_eq!(response.text().unwrap(), "eventually");
    assert_eq!(statuses(&server), [503, 503, 200]);
    let stats = session.stats();
    assert_eq!(stats.requests, 3);
    assert_eq!(stats.gets, 3);
    assert_eq!(stats.retries, 2);
    // Two retries withdrawn, the one that succeeded refunded: 500 - 10 + 5.
    assert_eq!(stats.retry_tokens, 495);
}

#[test]
fn a_connection_cut_before_any_byte_is_retried() {
    let server = HttpServer::start();
    server.put_resource("/cut", b"short body", Some("text/plain"));
    server.fail_after("/cut", 1);
    let session = session(HttpOptions::default().with_max_attempts(3));

    let response = session.get(&server.url("/cut")).unwrap().send().unwrap();

    assert_eq!(response.text().unwrap(), "short body");
    assert_eq!(statuses(&server), [499, 200]);
    assert_eq!(session.stats().requests, 2);
    assert_eq!(session.stats().retries, 1);
}

#[test]
fn attempts_stop_at_max_attempts_and_the_last_answer_is_handed_over() {
    let server = HttpServer::start();
    server.put_resource("/down", b"up", Some("text/plain"));
    server.fail_status("/down", 503, Some("0"), 5);
    let session = session(HttpOptions::default().with_max_attempts(2));

    let response = session.get(&server.url("/down")).unwrap().send().unwrap();

    assert_eq!(response.status().code(), 503);
    assert!(!response.is_ok());
    assert_eq!(server.request_count(), 2);
    assert_eq!(session.stats().retries, 1);
    let refused = response.raise_for_status().expect_err("a 503 is a refusal");
    match refused {
        Error::Remote {
            service,
            operation,
            status,
            ..
        } => {
            assert_eq!(service, "http");
            assert_eq!(operation, "GET");
            assert_eq!(status, 503);
        }
        other => panic!("expected a remote refusal, got {other:?}"),
    }
}

#[test]
fn a_held_body_is_replayed_on_retry() {
    let server = HttpServer::start();
    server.fail_status("/echo", 503, Some("0"), 1);
    let session = session(HttpOptions::default().with_max_attempts(3));

    let response = session
        .post(&server.url("/echo"), Body::from("payload"))
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 200);
    assert_eq!(response.text().unwrap(), "payload");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    for recorded in &requests {
        assert_eq!(recorded.method.to_string(), "POST");
        assert_eq!(recorded.body_len, 7);
    }
    assert_eq!(session.stats().posts, 2);
}

#[test]
fn a_client_error_is_never_retried() {
    let server = HttpServer::start();
    server.set_status("/gone", 404, None);
    let session = session(HttpOptions::default().with_max_attempts(3));

    let response = session.get(&server.url("/gone")).unwrap().send().unwrap();

    assert_eq!(response.status().code(), 404);
    assert_eq!(server.request_count(), 1);
    assert_eq!(session.stats().retries, 0);
}

#[test]
fn a_max_attempts_of_one_never_retries() {
    let server = HttpServer::start();
    server.put_resource("/once", b"x", Some("text/plain"));
    server.fail_status("/once", 503, Some("0"), 1);
    let session = session(HttpOptions::default().with_max_attempts(1));

    let response = session.get(&server.url("/once")).unwrap().send().unwrap();

    assert_eq!(response.status().code(), 503);
    assert_eq!(server.request_count(), 1);
}

// --- counters ----------------------------------------------------------------

#[test]
fn every_method_is_counted_under_its_own_name() {
    let server = HttpServer::start();
    server.put_resource("/thing", b"thing", Some("text/plain"));
    let session = Session::new();
    let url = server.url("/thing");

    session.get(&url).unwrap().send().unwrap();
    session.head(&url).unwrap().send().unwrap();
    session.post(&url, "p").unwrap().send().unwrap();
    session.patch(&url, "q").unwrap().send().unwrap();
    session.options_request(&url).unwrap().send().unwrap();
    session.put(&url, "new").unwrap().send().unwrap();
    session.delete(&url).unwrap().send().unwrap();

    let stats = session.stats();
    assert_eq!(stats.gets, 1);
    assert_eq!(stats.heads, 1);
    assert_eq!(stats.posts, 1);
    assert_eq!(stats.patches, 1);
    assert_eq!(stats.others, 1);
    assert_eq!(stats.puts, 1);
    assert_eq!(stats.deletes, 1);
    assert_eq!(stats.requests, 7);
    assert_eq!(server.request_count(), 7);
    let methods: Vec<String> = server
        .requests()
        .iter()
        .map(|recorded| recorded.method.to_string())
        .collect();
    assert_eq!(
        methods,
        ["GET", "HEAD", "POST", "PATCH", "OPTIONS", "PUT", "DELETE"]
    );
    assert!(header(&server.requests()[0], "user-agent").is_some());
}

#[test]
fn a_request_timeout_of_its_own_still_reaches_the_server() {
    let server = HttpServer::start();
    server.put_resource("/quick", b"quick", Some("text/plain"));
    let session = Session::new();

    let response = session
        .get(&server.url("/quick"))
        .unwrap()
        .with_timeout(Duration::from_secs(5))
        .send()
        .unwrap();

    assert_eq!(response.text().unwrap(), "quick");
    assert_eq!(server.request_count(), 1);
}
