//! `rust/src/http/client.rs`: the pooled client, its counters and the retry
//! loop, against the loopback server.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yggdryl::holder::Holder;
use yggdryl::http::{
    Body, Client, Fault, Headers, HttpOptions, Method, Response, Server, ServerOptions, Session,
    StatsSnapshot, Status,
};
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

/// What STS answers a caller it throttles: `400`, the code in the body.
const THROTTLED: &str =
    "<ErrorResponse><Error><Type>Sender</Type><Code>Throttling</Code></Error></ErrorResponse>";

/// Whether an STS-shaped body names the throttling code.
fn names_throttling(body: &[u8]) -> bool {
    body.windows(b"<Code>Throttling</Code>".len())
        .any(|window| window == b"<Code>Throttling</Code>")
}

/// Route `path` on `server` to answer `400` with [`THROTTLED`] for the first
/// `times` requests, then `200` with `ok`.
fn throttle(server: &HttpServer, path: &str, times: u32) {
    let answered = AtomicU32::new(0);
    server.server().route(None, path, move |_| {
        if answered.fetch_add(1, Ordering::SeqCst) < times {
            Ok(Response::new(Status::new(400)?)
                .with_header("Content-Type", "text/xml")?
                .with_body(THROTTLED))
        } else {
            Ok(Response::new(Status::OK).with_text("ok"))
        }
    });
}

/// A loopback listener that takes no connection: its backlog filled by
/// connections nobody accepts, held beside it.
struct Saturated {
    address: SocketAddr,
    /// Whether the stack drops a `SYN` past the full backlog, so a connect
    /// waits for its timeout (Linux, macOS), rather than refusing it
    /// outright (Windows).
    drops: bool,
    _listener: TcpListener,
    _held: Vec<TcpStream>,
}

fn saturated() -> Saturated {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let address = listener.local_addr().expect("its address");
    let mut held = Vec::new();
    let mut drops = false;
    while held.len() < 4096 {
        match TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
            Ok(stream) => held.push(stream),
            Err(error) => {
                drops = matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                );
                break;
            }
        }
    }
    Saturated {
        address,
        drops,
        _listener: listener,
        _held: held,
    }
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
fn a_socks_proxy_is_refused_rather_than_bypassed() {
    for proxy in ["socks5://127.0.0.1:1080", "socks5h://h:1", "socks4://h:1"] {
        let error =
            Client::with_options(&HttpOptions::default().with_proxy(proxy)).expect_err("a refusal");
        assert!(error.is_unsupported(), "{proxy}: {error:?}");
        assert!(error.to_string().contains("SOCKS"), "{error}");
    }
}

#[test]
fn a_post_no_connection_took_is_retried() {
    // Bind and drop a listener: its port now refuses every connection.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port();
    let session =
        Session::with_options(HttpOptions::default().with_max_attempts(3)).expect("a session");
    let request = session
        .post(&format!("http://127.0.0.1:{port}/items"), b"{}".to_vec())
        .expect("a URL");
    request.send().expect_err("nothing listens");
    let stats = session.stats();
    assert_eq!(stats.requests, 3, "a refused connection sent nothing");
    assert_eq!(stats.retries, 2);
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

#[test]
fn a_named_proxy_tunnels_every_request() {
    // A server that tunnels `CONNECT` stands in for a forward proxy.
    let origin = HttpServer::start();
    origin.put_resource("/rows.json", b"[1]", Some("application/json"));
    let proxy = Server::bind_with("127.0.0.1:0", ServerOptions::default().with_tunnel(true))
        .expect("a proxy");
    let session = session(HttpOptions::default().with_proxy(proxy.url().to_string()));

    let response = session
        .get(&origin.url("/rows.json"))
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(response.text().unwrap(), "[1]");
    let tunnelled = proxy.requests();
    assert_eq!(tunnelled.len(), 1);
    assert_eq!(tunnelled[0].method, Method::Connect);
    assert_eq!(tunnelled[0].target, format!("127.0.0.1:{}", origin.port()));
    assert_eq!(origin.request_count(), 1);
}

#[test]
fn a_proxy_that_does_not_tunnel_is_refused_by_status() {
    let proxy = Server::bind("127.0.0.1:0").expect("a proxy");
    let origin = HttpServer::start();
    let session = session(
        HttpOptions::default()
            .with_proxy(proxy.url().to_string())
            .with_max_attempts(1),
    );
    let error = session
        .get(&origin.url("/x"))
        .unwrap()
        .send()
        .expect_err("the proxy refused the tunnel");
    assert!(error.to_string().contains("405"), "{error}");
    assert_eq!(origin.request_count(), 0);
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
    server.fail_status("/stored", 503, Some("0"), 1);
    let session = session(HttpOptions::default().with_max_attempts(3));

    let response = session
        .put(&server.url("/stored"), Body::from("payload"))
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 201);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    for recorded in &requests {
        assert_eq!(recorded.method.to_string(), "PUT");
        assert_eq!(recorded.body_len, 7);
    }
    assert_eq!(session.stats().puts, 2);
    let stored = session.get(&server.url("/stored")).unwrap().send().unwrap();
    assert_eq!(stored.text().unwrap(), "payload");
}

#[test]
fn a_post_answered_not_now_is_sent_once_and_handed_over() {
    // A POST the server may have acted on is never sent twice on its own:
    // the 503 is the caller's to judge.
    let server = HttpServer::start();
    server.fail_status("/echo", 503, Some("0"), 1);
    let session = session(HttpOptions::default().with_max_attempts(3));

    let response = session
        .post(&server.url("/echo"), Body::from("payload"))
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 503);
    assert_eq!(server.request_count(), 1);
    assert_eq!(session.stats().posts, 1);
    assert_eq!(session.stats().retries, 0);
}

#[test]
fn a_retry_after_longer_than_max_pause_ends_the_retries() {
    let server = HttpServer::start();
    server.put_resource("/busy", b"later", Some("text/plain"));
    server.fail_status("/busy", 503, Some("120"), 1);
    let session = session(
        HttpOptions::default()
            .with_max_attempts(3)
            .with_max_pause(Duration::from_secs(1)),
    );

    let started = std::time::Instant::now();
    let response = session.get(&server.url("/busy")).unwrap().send().unwrap();

    assert_eq!(response.status().code(), 503);
    assert_eq!(response.headers().get("retry-after"), Some("120"));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(server.request_count(), 1);
    assert_eq!(session.stats().retries, 0);
}

#[test]
fn a_retry_after_within_max_pause_is_waited_out() {
    let server = HttpServer::start();
    server.put_resource("/busy", b"later", Some("text/plain"));
    server.fail_status("/busy", 429, Some("1"), 1);
    let session = session(
        HttpOptions::default()
            .with_max_attempts(3)
            .with_max_pause(Duration::from_secs(2)),
    );

    let started = std::time::Instant::now();
    let response = session.get(&server.url("/busy")).unwrap().send().unwrap();

    assert_eq!(response.text().unwrap(), "later");
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(statuses(&server), [429, 200]);
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

// --- what a request states for its own attempts -----------------------------

#[test]
fn an_attempt_headers_error_is_the_requests_error_and_never_retried() {
    let server = HttpServer::start();
    server.put_resource("/proof", b"x", Some("text/plain"));
    server.fail_status("/proof", 503, Some("0"), 1);
    let session = session(HttpOptions::default().with_max_attempts(3));

    // Refused before the first attempt: nothing goes out.
    let refused = session
        .get(&server.url("/proof"))
        .unwrap()
        .with_attempt_headers(|_| Err(Error::unsupported("a signing key", "dpop")))
        .send()
        .expect_err("the hook refused");
    assert!(refused.is_unsupported(), "{refused:?}");
    assert_eq!(server.request_count(), 0);
    assert_eq!(session.stats().requests, 0);

    // Refused at the second: the first went out, the error ends the retries.
    let refused = session
        .get(&server.url("/proof"))
        .unwrap()
        .with_attempt_headers(|attempt| {
            if attempt.number() == 1 {
                Ok(Headers::new())
            } else {
                Err(Error::unsupported("a second proof", "dpop"))
            }
        })
        .send()
        .expect_err("the hook refused the retry");
    assert!(refused.is_unsupported(), "{refused:?}");
    assert_eq!(statuses(&server), [503]);
    let stats = session.stats();
    assert_eq!(stats.requests, 1);
    assert_eq!(stats.retries, 0);
}

#[test]
fn attempt_headers_are_made_for_each_attempt_over_the_requests_own() {
    let server = HttpServer::start();
    server.put_resource("/flaky", b"eventually", Some("text/plain"));
    server.fail_status("/flaky", 503, Some("0"), 2);
    let session = session(HttpOptions::default().with_max_attempts(3));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hook_seen = Arc::clone(&seen);

    let response = session
        .get(&server.url("/flaky"))
        .unwrap()
        .with_header("DPoP", "stale")
        .unwrap()
        .with_header("X-Kept", "kept")
        .unwrap()
        .with_attempt_headers(move |attempt| {
            // The attempt shows what goes out before the hook's own: the
            // request's headers, and no body for a `GET`.
            assert_eq!(attempt.headers().get("dpop"), Some("stale"));
            assert_eq!(attempt.headers().get("x-kept"), Some("kept"));
            assert_eq!(attempt.body(), None);
            assert!(!attempt.is_streamed());
            hook_seen.lock().unwrap().push((
                attempt.number(),
                attempt.method(),
                attempt.url().to_string(),
            ));
            let mut headers = Headers::new();
            headers.insert("dpop", &format!("proof-{}", attempt.number()))?;
            Ok(headers)
        })
        .send()
        .unwrap();

    assert_eq!(response.text().unwrap(), "eventually");
    assert_eq!(statuses(&server), [503, 503, 200]);
    let requests = server.requests();
    let proofs: Vec<_> = requests
        .iter()
        .map(|recorded| header(recorded, "dpop").unwrap_or_default().to_owned())
        .collect();
    assert_eq!(proofs, ["proof-1", "proof-2", "proof-3"]);
    for recorded in &requests {
        assert_eq!(header(recorded, "x-kept"), Some("kept"));
    }
    let url = server.url("/flaky");
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (1, Method::Get, url.clone()),
            (2, Method::Get, url.clone()),
            (3, Method::Get, url),
        ]
    );
}

#[test]
fn a_post_answered_not_now_is_retried_only_when_declared_idempotent() {
    let server = HttpServer::start();
    server.echo("/token");
    let session = session(HttpOptions::default().with_max_attempts(3));

    server.fail_status("/token", 503, Some("0"), 1);
    let declared = session
        .post(
            &server.url("/token"),
            Body::from("grant_type=refresh_token"),
        )
        .unwrap()
        .with_idempotent(true)
        .send()
        .unwrap();
    assert_eq!(declared.status().code(), 200);
    assert_eq!(declared.text().unwrap(), "grant_type=refresh_token");
    assert_eq!(statuses(&server), [503, 200]);
    assert_eq!(session.stats().posts, 2);
    assert_eq!(session.stats().retries, 1);

    server.clear_requests();
    server.fail_status("/token", 503, Some("0"), 1);
    let undeclared = session
        .post(
            &server.url("/token"),
            Body::from("grant_type=refresh_token"),
        )
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(undeclared.status().code(), 503);
    assert_eq!(statuses(&server), [503]);

    // A GET declared not idempotent goes out once too.
    server.clear_requests();
    server.put_resource("/once", b"x", Some("text/plain"));
    server.fail_status("/once", 503, Some("0"), 1);
    let get = session
        .get(&server.url("/once"))
        .unwrap()
        .with_idempotent(false)
        .send()
        .unwrap();
    assert_eq!(get.status().code(), 503);
    assert_eq!(statuses(&server), [503]);
}

#[test]
fn a_requests_own_attempt_count_stands_in_for_the_clients() {
    let server = HttpServer::start();
    server.put_resource("/down", b"up", Some("text/plain"));
    server.fail_status("/down", 503, Some("0"), 10);

    // One attempt, whatever the client grants.
    let generous = session(HttpOptions::default().with_max_attempts(5));
    let once = generous
        .get(&server.url("/down"))
        .unwrap()
        .with_max_attempts(1)
        .send()
        .unwrap();
    assert_eq!(once.status().code(), 503);
    assert_eq!(server.request_count(), 1);
    assert_eq!(generous.stats().retries, 0);

    // Three, where the client grants one: two retries, paid from its budget.
    server.clear_requests();
    let sparing = session(HttpOptions::default().with_max_attempts(1));
    let thrice = sparing
        .get(&server.url("/down"))
        .unwrap()
        .with_max_attempts(3)
        .send()
        .unwrap();
    assert_eq!(thrice.status().code(), 503);
    assert_eq!(server.request_count(), 3);
    let stats = sparing.stats();
    assert_eq!(stats.retries, 2);
    assert_eq!(stats.retry_tokens, 500 - 2 * 5);
}

#[test]
fn a_connect_timeout_bounds_a_connection_no_listener_takes() {
    let listener = saturated();
    let session = session(HttpOptions::default());
    let started = Instant::now();

    let error = session
        .get(&format!("http://{}/x", listener.address))
        .unwrap()
        .with_connect_timeout(Duration::from_millis(250))
        .with_max_attempts(1)
        .send()
        .expect_err("no connection is taken");

    // Under the pool's ten seconds: the request's own bound was spent.
    assert!(started.elapsed() < Duration::from_secs(5), "{error:?}");
    let Error::Io(io) = &error else {
        panic!("expected a transport failure, got {error:?}");
    };
    // Where the stack drops the attempt, only the bound ends it; where it
    // refuses, the refusal is the answer and no bound is reached.
    let expected = if listener.drops {
        std::io::ErrorKind::TimedOut
    } else {
        std::io::ErrorKind::ConnectionRefused
    };
    assert_eq!(io.kind(), expected, "{io:?}");
    assert!(error.to_string().contains("failed"), "{error}");
}

#[test]
fn a_deadline_bounds_the_whole_of_an_attempt() {
    let server = HttpServer::start();
    server.put_resource("/slow", b"late", Some("text/plain"));
    server
        .server()
        .inject("/slow", Fault::Delay(Duration::from_secs(2)), 1);
    let session = session(HttpOptions::default());
    let started = Instant::now();

    let error = session
        .get(&server.url("/slow"))
        .unwrap()
        .with_deadline(Duration::from_millis(300))
        .with_max_attempts(1)
        .send()
        .expect_err("the answer comes after the deadline");

    assert!(
        started.elapsed() < Duration::from_millis(1_500),
        "{error:?}"
    );
    match &error {
        Error::Io(io) => assert_eq!(io.kind(), std::io::ErrorKind::TimedOut, "{io:?}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
    // The same request with room to finish reads the answer.
    let answered = session
        .get(&server.url("/slow"))
        .unwrap()
        .with_deadline(Duration::from_secs(30))
        .send()
        .unwrap();
    assert_eq!(answered.text().unwrap(), "late");
}

#[test]
fn a_retry_rule_retries_a_failing_answer_its_body_names() {
    let server = HttpServer::start();
    throttle(&server, "/sts", 2);
    let session = session(HttpOptions::default().with_max_attempts(3));
    let asked = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&asked);

    let response = session
        .post(&server.url("/sts"), Body::from("Action=AssumeRole"))
        .unwrap()
        .with_idempotent(true)
        .with_retry_on(move |status, headers, body| {
            counted.fetch_add(1, Ordering::SeqCst);
            status.code() == 400
                && headers.content_type() == Some("text/xml")
                && names_throttling(body)
        })
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 200);
    assert_eq!(response.text().unwrap(), "ok");
    assert_eq!(statuses(&server), [400, 400, 200]);
    assert_eq!(
        asked.load(Ordering::SeqCst),
        2,
        "a success is never asked about"
    );
    let stats = session.stats();
    assert_eq!(stats.posts, 3);
    assert_eq!(stats.retries, 2);
}

#[test]
fn a_retry_rule_that_declines_hands_the_body_back_whole() {
    let server = HttpServer::start();
    // Past what a rule is handed, so the rest must follow what it read.
    let body: Vec<u8> = (0..100 * 1024).map(|index| (index % 251) as u8).collect();
    let served = body.clone();
    server.server().route(None, "/invalid", move |_| {
        Ok(Response::new(Status::new(400)?).with_body(served.clone()))
    });
    let session = session(HttpOptions::default().with_max_attempts(3));
    let handed = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&handed);

    let response = session
        .get(&server.url("/invalid"))
        .unwrap()
        .with_retry_on(move |_, _, peeked| {
            kept.lock().unwrap().push(peeked.len());
            false
        })
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 400);
    assert_eq!(&*response.bytes().unwrap(), body.as_slice());
    assert_eq!(*handed.lock().unwrap(), [64 * 1024]);
    assert_eq!(server.request_count(), 1);
    assert_eq!(session.stats().retries, 0);
}

#[test]
fn a_retry_rule_is_asked_only_where_a_retry_could_follow() {
    let server = HttpServer::start();
    throttle(&server, "/sts", 10);
    server.put_resource("/busy", b"later", Some("text/plain"));
    server.fail_status("/busy", 503, Some("0"), 1);
    let session = session(HttpOptions::default().with_max_attempts(3));
    let asked = Arc::new(AtomicU32::new(0));
    let rule = {
        let asked = Arc::clone(&asked);
        move |_: Status, _: &Headers, body: &[u8]| {
            asked.fetch_add(1, Ordering::SeqCst);
            names_throttling(body)
        }
    };

    // A POST nobody declared idempotent is sent once, the rule unasked.
    let post = session
        .post(&server.url("/sts"), Body::from("Action=AssumeRole"))
        .unwrap()
        .with_retry_on(rule.clone())
        .send()
        .unwrap();
    assert_eq!(post.status().code(), 400);
    assert_eq!(post.text().unwrap(), THROTTLED);
    assert_eq!(server.request_count(), 1);
    assert_eq!(asked.load(Ordering::SeqCst), 0);

    // A status the client retries anyway is never the rule's to judge.
    server.clear_requests();
    let busy = session
        .get(&server.url("/busy"))
        .unwrap()
        .with_retry_on(rule.clone())
        .send()
        .unwrap();
    assert_eq!(busy.text().unwrap(), "later");
    assert_eq!(statuses(&server), [503, 200]);
    assert_eq!(asked.load(Ordering::SeqCst), 0);

    // The last attempt's answer is handed over unread: nothing could follow.
    server.clear_requests();
    let last = session
        .get(&server.url("/sts"))
        .unwrap()
        .with_max_attempts(2)
        .with_retry_on(rule)
        .send()
        .unwrap();
    assert_eq!(last.status().code(), 400);
    assert_eq!(last.text().unwrap(), THROTTLED);
    assert_eq!(server.request_count(), 2);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[test]
fn a_direct_request_goes_past_a_named_proxy() {
    let origin = HttpServer::start();
    origin.put_resource("/metadata", b"i-0123", Some("text/plain"));
    let proxy = Server::bind_with("127.0.0.1:0", ServerOptions::default().with_tunnel(true))
        .expect("a proxy");
    let session = session(HttpOptions::default().with_proxy(proxy.url().to_string()));

    let direct = session
        .get(&origin.url("/metadata"))
        .unwrap()
        .with_direct(true)
        .send()
        .unwrap();
    assert_eq!(direct.text().unwrap(), "i-0123");
    assert!(proxy.requests().is_empty());
    assert_eq!(origin.request_count(), 1);

    // The same session's other requests still go through it.
    let proxied = session
        .get(&origin.url("/metadata"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(proxied.text().unwrap(), "i-0123");
    assert_eq!(proxy.requests().len(), 1);
    assert_eq!(origin.request_count(), 2);
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
