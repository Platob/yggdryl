//! `rust/src/http/session.rs`: the defaults, the credential, the cookies, the
//! redirects and the container role, against the loopback server.

use yggdryl::holder::Holder;
use yggdryl::http::{Authorization, Cookie, HttpOptions, Session};
use yggdryl::{Error, IOBase, IOKind, Url};

use crate::http_server::RecordedExt as _;
use crate::http_server::{HttpServer, Recorded, basic_credential};

/// One recorded request's header, over whichever shape the fixture records.
fn header<'a>(recorded: &'a Recorded, name: &str) -> Option<&'a str> {
    recorded.header(name)
}

fn configured(options: HttpOptions) -> Session {
    Session::with_options(options).expect("a session over the default transport")
}

/// The reason of a URL refusal: the HTTP door's own, or the URI parser's
/// for text that is no URL at all.
fn url_refusal(error: Error) -> String {
    match error {
        Error::Parse { target, reason, .. } => {
            assert!(target == "http url" || target == "url", "{target}");
            reason.to_string()
        }
        other => panic!("expected an http url refusal, got {other:?}"),
    }
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_relative_url_on_a_session_with_no_base_is_refused() {
    let reason = url_refusal(Session::new().get("items/1").expect_err("no base"));
    assert!(reason.contains("items/1"), "{reason}");
    assert!(reason.contains("base URL"), "{reason}");
}

#[test]
fn a_url_of_another_scheme_is_refused_naming_it() {
    let reason = url_refusal(
        Session::new()
            .get("ftp://files.example.com/x")
            .expect_err("not http"),
    );
    assert!(reason.contains("ftp"), "{reason}");
    url_refusal(Session::new().get("file:///tmp/x").expect_err("not http"));
    url_refusal(
        Session::new()
            .get("mailto:someone@example.com")
            .expect_err("not a URL"),
    );
}

#[test]
fn a_header_that_will_not_validate_is_refused() {
    let request = Session::new()
        .get("http://127.0.0.1:1/x")
        .unwrap()
        .with_header("Bad Name", "x")
        .expect_err("a name with a space is no token");
    assert!(
        matches!(
            request,
            Error::Parse {
                target: "http header",
                ..
            }
        ),
        "{request:?}"
    );
    let options = HttpOptions::default()
        .with_header("X-Trace", "a\u{1}b")
        .expect_err("a control byte is no field value");
    assert!(matches!(options, Error::Parse { .. }), "{options:?}");
}

#[test]
fn too_many_redirects_are_refused_naming_the_last_location() {
    let server = HttpServer::start();
    server.redirect("/loop", 302, "/loop");
    let session = configured(HttpOptions::default().with_max_redirects(2));

    let error = session
        .get(&server.url("/loop"))
        .unwrap()
        .send()
        .expect_err("a loop ends");

    match error {
        Error::Remote {
            service,
            status,
            code,
            message,
            ..
        } => {
            assert_eq!(service, "http");
            assert_eq!(status, 302);
            assert_eq!(code, "TooManyRedirects");
            assert!(message.contains("/loop"), "{message}");
        }
        other => panic!("expected a remote refusal, got {other:?}"),
    }
    // The first request and the two hops allowed: one per redirect.
    assert_eq!(server.request_count(), 3);
    assert_eq!(session.stats().redirects, 2);
}

// --- resolution and defaults -------------------------------------------------

#[test]
fn a_relative_url_joins_onto_the_base() {
    let server = HttpServer::start();
    server.put_resource("/v1/items", b"[1,2]", Some("application/json"));
    let base = Url::from_str(&server.url("/v1/")).unwrap();
    let session = configured(HttpOptions::default().with_base_url(base.clone()));

    let request = session.get("items?limit=2").unwrap();
    assert_eq!(request.url().to_string(), format!("{base}items?limit=2"));
    assert_eq!(session.base_url(), Some(&base));
    assert_eq!(session.stats().requests, 0);

    let response = request.send().unwrap();
    assert_eq!(response.text().unwrap(), "[1,2]");
    let recorded = &server.requests()[0];
    assert_eq!(recorded.path, "/v1/items");
    assert_eq!(recorded.query, [("limit".to_owned(), "2".to_owned())]);
    // An absolute URL on a session with a base is taken as it is.
    let absolute = session.get(&server.url("/other")).unwrap();
    assert_eq!(absolute.url().to_string(), server.url("/other"));
}

#[test]
fn defaults_go_under_the_request_headers_which_win() {
    let server = HttpServer::start();
    server.put_resource("/h", b"h", Some("text/plain"));
    let session = configured(
        HttpOptions::default()
            .with_header("X-Api-Key", "k-1")
            .unwrap()
            .with_header("X-Trace", "session")
            .unwrap(),
    );

    session
        .get(&server.url("/h"))
        .unwrap()
        .with_header("X-Trace", "request")
        .unwrap()
        .send()
        .unwrap();

    let recorded = &server.requests()[0];
    assert_eq!(header(recorded, "x-api-key"), Some("k-1"));
    assert_eq!(header(recorded, "x-trace"), Some("request"));
}

#[test]
fn accept_encoding_and_user_agent_are_filled_in_when_unset() {
    let server = HttpServer::start();
    server.put_resource("/h", b"h", Some("text/plain"));
    let session = Session::new();
    let request = session.get(&server.url("/h")).unwrap();

    request.send().unwrap();
    request.stream().unwrap();
    request
        .clone()
        .with_header("Accept-Encoding", "br")
        .unwrap()
        .with_header("User-Agent", "probe/1")
        .unwrap()
        .send()
        .unwrap();

    let requests = server.requests();
    assert_eq!(
        header(&requests[0], "accept-encoding"),
        Some("gzip, deflate, zstd")
    );
    assert!(
        header(&requests[0], "user-agent").is_some_and(|agent| agent.starts_with("yggdryl/")),
        "{:?}",
        header(&requests[0], "user-agent")
    );
    // A streaming request asks for the bytes as they are.
    assert_eq!(header(&requests[1], "accept-encoding"), Some("identity"));
    assert_eq!(header(&requests[2], "accept-encoding"), Some("br"));
    assert_eq!(header(&requests[2], "user-agent"), Some("probe/1"));
}

// --- the credential ----------------------------------------------------------

#[test]
fn the_session_credential_is_sent_when_the_request_names_none() {
    let server = HttpServer::start();
    server.put_resource("/private", b"secret", Some("text/plain"));
    server.require_basic("/private", "aladdin", "open sesame");
    let session = configured(
        HttpOptions::default().with_authorization(Authorization::basic("aladdin", "open sesame")),
    );

    let response = session
        .get(&server.url("/private"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.status().code(), 200);
    assert_eq!(response.text().unwrap(), "secret");
    assert_eq!(
        header(&server.requests()[0], "authorization"),
        Some(basic_credential("aladdin", "open sesame").as_str())
    );

    // The request's own credential wins over the session's.
    let response = session
        .get(&server.url("/private"))
        .unwrap()
        .with_authorization(Authorization::bearer("t-1"))
        .send()
        .unwrap();
    assert_eq!(response.status().code(), 401);
    assert_eq!(
        header(&server.requests()[1], "authorization"),
        Some("Bearer t-1")
    );

    // Without any, the challenge is the answer and not an error.
    let response = Session::new()
        .get(&server.url("/private"))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.status().code(), 401);
    assert!(response.raise_for_status().is_err());
}

#[test]
fn a_credential_in_the_url_is_sent_as_basic() {
    let server = HttpServer::start();
    server.put_resource("/private", b"secret", Some("text/plain"));
    server.require_basic("/private", "user", "pass");
    let url = format!("http://user:pass@127.0.0.1:{}/private", server.port());

    let response = Session::new().get(&url).unwrap().send().unwrap();

    assert_eq!(response.status().code(), 200);
    assert_eq!(
        header(&server.requests()[0], "authorization"),
        Some(basic_credential("user", "pass").as_str())
    );
}

#[test]
fn the_credential_is_not_carried_to_another_host() {
    let server = HttpServer::start();
    server.put_resource("/there", b"there", Some("text/plain"));
    server.redirect(
        "/away",
        302,
        &format!("http://localhost:{}/there", server.port()),
    );
    server.redirect("/nearby", 302, "/there");
    let session =
        configured(HttpOptions::default().with_authorization(Authorization::bearer("t-1")));

    let response = session.get(&server.url("/away")).unwrap().send().unwrap();
    assert_eq!(response.text().unwrap(), "there");
    assert_eq!(
        response.url().to_string(),
        format!("http://localhost:{}/there", server.port())
    );
    let requests = server.requests();
    assert_eq!(header(&requests[0], "authorization"), Some("Bearer t-1"));
    assert_eq!(requests[1].path, "/there");
    assert_eq!(header(&requests[1], "authorization"), None);

    server.clear_requests();
    session.get(&server.url("/nearby")).unwrap().send().unwrap();
    let requests = server.requests();
    assert_eq!(header(&requests[1], "authorization"), Some("Bearer t-1"));
}

// --- redirects ---------------------------------------------------------------

#[test]
fn a_303_and_a_302_on_post_become_a_get_without_the_body() {
    let server = HttpServer::start();
    server.put_resource("/done", b"done", Some("text/plain"));
    server.redirect("/submit", 303, "/done");
    server.redirect("/legacy", 302, "/done");
    let session = Session::new();

    for path in ["/submit", "/legacy"] {
        server.clear_requests();
        let response = session
            .post(&server.url(path), "payload")
            .unwrap()
            .with_header("Content-Type", "text/plain")
            .unwrap()
            .send()
            .unwrap();
        assert_eq!(response.status().code(), 200, "{path}");
        assert_eq!(response.text().unwrap(), "done", "{path}");
        assert_eq!(response.url().to_string(), server.url("/done"), "{path}");
        assert_eq!(response.history().len(), 1, "{path}");
        assert!(response.history()[0].is_redirect(), "{path}");
        let requests = server.requests();
        assert_eq!(requests.len(), 2, "{path}");
        assert_eq!(requests[0].method.to_string(), "POST", "{path}");
        assert_eq!(requests[0].body_len, 7, "{path}");
        assert_eq!(requests[1].method.to_string(), "GET", "{path}");
        assert_eq!(requests[1].body_len, 0, "{path}");
        assert_eq!(header(&requests[1], "content-type"), None, "{path}");
    }
    assert_eq!(session.stats().redirects, 2);
    assert_eq!(session.stats().requests, 4);
}

#[test]
fn a_307_keeps_the_method_and_the_body() {
    let server = HttpServer::start();
    server.redirect("/temp", 307, "/echo");
    server.echo("/echo");
    let session = Session::new();

    let response = session
        .post(&server.url("/temp"), "payload")
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 200);
    assert_eq!(response.text().unwrap(), "payload");
    let requests = server.requests();
    assert_eq!(requests[1].method.to_string(), "POST");
    assert_eq!(requests[1].path, "/echo");
    assert_eq!(requests[1].body_len, 7);
    assert_eq!(response.history()[0].status().code(), 307);
}

#[test]
fn a_relative_location_is_resolved_against_the_hop() {
    let server = HttpServer::start();
    server.put_resource("/c/d", b"d", Some("text/plain"));
    server.redirect("/a/b", 301, "../c/d");
    let session = Session::new();

    let response = session.get(&server.url("/a/b")).unwrap().send().unwrap();

    assert_eq!(response.text().unwrap(), "d");
    assert_eq!(server.requests()[1].path, "/c/d");
    // A 301 on GET keeps the method.
    assert_eq!(server.requests()[1].method.to_string(), "GET");
}

#[test]
fn redirects_are_not_followed_when_the_options_or_the_request_say() {
    let server = HttpServer::start();
    server.redirect("/moved", 302, "/elsewhere");
    let session = configured(HttpOptions::default().with_follow_redirects(false));

    let response = session.get(&server.url("/moved")).unwrap().send().unwrap();
    assert_eq!(response.status().code(), 302);
    assert!(response.is_redirect());
    assert!(response.is_ok());
    assert_eq!(response.headers().location(), Some("/elsewhere"));
    assert_eq!(server.request_count(), 1);

    let response = Session::new()
        .get(&server.url("/moved"))
        .unwrap()
        .with_follow_redirects(false)
        .send()
        .unwrap();
    assert_eq!(response.status().code(), 302);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_3xx_without_a_location_is_the_answer() {
    let server = HttpServer::start();
    server.set_status("/fresh", 304, None);

    let response = Session::new()
        .get(&server.url("/fresh"))
        .unwrap()
        .send()
        .unwrap();

    assert_eq!(response.status().code(), 304);
    assert_eq!(server.request_count(), 1);
}

// --- cookies -----------------------------------------------------------------

#[test]
fn set_cookie_lands_in_the_jar_and_rides_the_next_request() {
    let server = HttpServer::start();
    server.put_resource("/login", b"ok", Some("text/plain"));
    server.put_resource("/data", b"data", Some("text/plain"));
    server.set_cookie("/login", "sid=abc; Path=/");
    let session = Session::new();

    session.get(&server.url("/login")).unwrap().send().unwrap();
    session.get(&server.url("/data")).unwrap().send().unwrap();

    assert_eq!(server.cookies(), ["sid=abc"]);
    let cookies = session.cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "sid");
    assert_eq!(cookies[0].value, "abc");
    assert_eq!(header(&server.requests()[0], "cookie"), None);
}

#[test]
fn a_cookie_set_by_hand_is_sent_and_cookies_can_be_turned_off() {
    let server = HttpServer::start();
    server.put_resource("/data", b"data", Some("text/plain"));
    server.set_cookie("/data", "sid=abc; Path=/");
    let url = Url::from_str(&server.url("/data")).unwrap();

    let session = Session::new();
    session.set_cookie(Cookie::from_set_cookie("tok=1", &url, 0).unwrap());
    session.get(&server.url("/data")).unwrap().send().unwrap();
    assert_eq!(header(&server.requests()[0], "cookie"), Some("tok=1"));

    let silent = configured(HttpOptions::default().with_cookies(false));
    silent.get(&server.url("/data")).unwrap().send().unwrap();
    silent.get(&server.url("/data")).unwrap().send().unwrap();
    assert_eq!(header(&server.requests()[2], "cookie"), None);
    assert!(silent.cookies().is_empty());
}

#[test]
fn a_cookie_set_on_a_redirect_hop_is_kept() {
    let server = HttpServer::start();
    server.put_resource("/home", b"home", Some("text/plain"));
    server.redirect("/enter", 302, "/home");
    server.set_cookie("/enter", "sid=hop; Path=/");
    let session = Session::new();

    session.get(&server.url("/enter")).unwrap().send().unwrap();

    assert_eq!(header(&server.requests()[1], "cookie"), Some("sid=hop"));
    assert_eq!(session.cookies().len(), 1);
}

// --- send_all ----------------------------------------------------------------

#[test]
fn send_all_answers_in_order_under_concurrency() {
    let server = HttpServer::start();
    for index in 0..8 {
        server.put_resource(
            &format!("/n{index}"),
            format!("body {index}").as_bytes(),
            Some("text/plain"),
        );
    }
    let session = configured(HttpOptions::default().with_concurrency(4));
    let requests: Vec<_> = (0..8)
        .map(|index| session.get(&server.url(&format!("/n{index}"))).unwrap())
        .collect();

    let bodies: Vec<String> = session
        .send_all(requests)
        .map(|response| response.unwrap().text().unwrap())
        .collect();

    let expected: Vec<String> = (0..8).map(|index| format!("body {index}")).collect();
    assert_eq!(bodies, expected);
    assert_eq!(server.request_count(), 8);
    assert_eq!(session.stats().requests, 8);
}

#[test]
fn send_all_on_one_thread_is_the_sequential_map() {
    let server = HttpServer::start();
    server.put_resource("/a", b"a", Some("text/plain"));
    server.set_status("/b", 500, None);
    let session = configured(
        HttpOptions::default()
            .with_concurrency(1)
            .with_max_attempts(1),
    );
    let requests = vec![
        session.get(&server.url("/a")).unwrap(),
        session.get(&server.url("/b")).unwrap(),
    ];

    let statuses: Vec<u16> = session
        .send_all(requests)
        .map(|response| response.unwrap().status().code())
        .collect();

    assert_eq!(statuses, [200, 500]);
}

// --- the container role ------------------------------------------------------

#[test]
fn a_session_is_a_container_over_its_base_url() {
    let base = Url::from_str("http://127.0.0.1:1/v1/").unwrap();
    let mut container = configured(HttpOptions::default().with_base_url(base.clone()));

    assert_eq!(container.kind(), IOKind::Directory);
    assert!(container.is_container());
    assert!(!container.is_atomic());
    assert!(!container.is_tabular());
    assert_eq!(container.url(), Some(&base));
    assert_eq!(
        container.uri().map(ToString::to_string),
        Some(base.to_string())
    );
    assert_eq!(container.size(), 0);
    assert_eq!(container.pread(0, &mut [0; 4]).unwrap(), 0);
    assert!(container.ls(true, true).next().is_none());
    assert!(container.pwrite(0, b"x").is_err());
    assert!(container.truncate(0).is_ok());
    assert!(container.clear().is_ok());
    assert!(container.remove(true).is_ok());
    assert_eq!(container.media_type().base(), &yggdryl::MimeType::DIRECTORY);

    let child = container.child_by_path("items/1").unwrap();
    let Holder::HttpRequest(request) = &child else {
        panic!("expected a request, got {child:?}");
    };
    assert_eq!(request.url().to_string(), "http://127.0.0.1:1/v1/items/1");
    let absolute = container.child_by_path("http://127.0.0.1:2/x").unwrap();
    assert_eq!(
        absolute.url().map(ToString::to_string).as_deref(),
        Some("http://127.0.0.1:2/x")
    );

    let held = Holder::HttpSession(container.clone());
    assert_eq!(held.kind(), IOKind::Directory);
    assert!(held.child_by_path("items/2").is_ok());
    assert_eq!(container.stats().requests, 0);
}

#[test]
fn a_session_with_no_base_has_no_location() {
    let session = Session::new();
    assert_eq!(session.url(), None);
    assert!(session.child_by_path("items/1").is_err());
}

#[test]
fn close_evicts_lapsed_cookies_and_keeps_the_session_usable() {
    let server = HttpServer::start();
    server.put_resource("/data", b"data", Some("text/plain"));
    let url = Url::from_str(&server.url("/data")).unwrap();
    let session = Session::new();
    session.set_cookie(Cookie::from_set_cookie("old=1; Max-Age=0", &url, 0).unwrap());
    session.set_cookie(Cookie::from_set_cookie("live=1", &url, 0).unwrap());

    session.close();

    let cookies = session.cookies();
    let names: Vec<&str> = cookies.iter().map(|cookie| cookie.name.as_str()).collect();
    assert!(!names.contains(&"old"), "{names:?}");
    session.get(&server.url("/data")).unwrap().send().unwrap();
    assert_eq!(server.request_count(), 1);
}
