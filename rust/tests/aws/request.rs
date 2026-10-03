//! `rust/src/aws/request.rs`: Signature Version 4 on a request the HTTP
//! client sends, against the crate's own server on loopback.
//!
//! What goes out is what is pinned: every recorded request's `authorization`
//! is computed again by the signer from the request as the server received
//! it - the target as sent, the `Host`, the signed headers, the body - so a
//! signature over anything but what was sent fails here. The signer's own
//! bytes are pinned against botocore in `rust/tests/aws/sigv4.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::aws::{Credentials, Session};
use yggdryl::http::{Headers, Method, Recorded, Response, Server, Status};
use yggdryl::internals::aws_sigv4::{
    EMPTY_PAYLOAD_SHA256, Signer, UNSIGNED_PAYLOAD, sha256_hex, signed_access_key,
};
use yggdryl::{Error, Url};

use crate::mod_::scratch;

const ACCESS_KEY: &str = "AKIDEXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const REGION: &str = "eu-west-3";

fn server() -> Server {
    Server::bind("127.0.0.1:0").expect("a loopback server")
}

fn url(server: &Server, target: &str) -> String {
    format!("http://127.0.0.1:{}{target}", server.port())
}

/// A session of the test's own: no cookie, no proxy and no `.netrc` of the
/// process reaches a request.
fn http() -> yggdryl::http::Session {
    yggdryl::http::Session::with_options(
        yggdryl::http::HttpOptions::default().with_read_environment(false),
    )
    .expect("a session")
}

/// A session that states one set and consults nothing else.
fn stated(token: Option<&str>) -> Session {
    let mut credentials = Credentials::new(ACCESS_KEY, SECRET_KEY);
    if let Some(token) = token {
        credentials = credentials.with_session_token(token);
    }
    Session::new()
        .with_environment(false)
        .with_credentials(credentials)
}

/// A session reading the credentials file under `directory` and nothing
/// else: its variables are the test's own and the metadata services are off.
fn reading_files(directory: &std::path::Path) -> Session {
    Session::new()
        .with_variables([(
            "BOTO_CONFIG",
            directory.join("no-such-boto.cfg").display().to_string(),
        )])
        .with_directory(directory)
        .with_metadata_disabled(true)
}

/// A temporary set for `key`, as a credentials file holds one.
fn dumped(key: &str) -> String {
    format!(
        "[default]\naws_access_key_id = {key}\naws_secret_access_key = {key}-secret\n\
         aws_session_token = {key}-token\n"
    )
}

/// The instant an `x-amz-date` names.
fn instant(amz_date: &str) -> SystemTime {
    let number = |range: std::ops::Range<usize>| -> i64 {
        amz_date[range].parse().expect("digits in x-amz-date")
    };
    let (year, month, day) = (number(0..4), number(4..6), number(6..8));
    // Days from the civil date (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year.rem_euclid(400);
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    let seconds = days * 86_400 + number(9..11) * 3600 + number(11..13) * 60 + number(13..15);
    UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).expect("an instant after the epoch"))
}

/// Assert `recorded` carries exactly the signature the signer computes over
/// the request as the server received it, for `service` under the set
/// `(access_key, secret_key, token)`, the body hashing to `payload_hash` and
/// `signed_headers` being the names it signs.
fn assert_signed(
    recorded: &Recorded,
    service: &str,
    (access_key, secret_key, token): (&str, &str, Option<&str>),
    payload_hash: &str,
    signed_headers: &str,
) {
    let header = |name: &str| recorded.headers.get(name).unwrap_or_default();
    let authorization = header("authorization");
    let date = header("x-amz-date");
    assert_eq!(header("x-amz-content-sha256"), payload_hash);
    assert_eq!(header("x-amz-security-token"), token.unwrap_or_default());
    assert!(
        authorization.contains(&format!(
            "Credential={access_key}/{}/{REGION}/{service}/aws4_request, SignedHeaders={signed_headers}, ",
            &date[..8]
        )),
        "{authorization}"
    );
    // Read off the target as sent rather than off the server's own parse of
    // it, which a request it refuses never gets.
    let (path, query) = recorded
        .target
        .split_once('?')
        .unwrap_or((recorded.target.as_str(), ""));
    let query: Vec<(String, String)> = yggdryl::Parameters::from_query(query, true)
        .expect("a query")
        .iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let carried: Vec<(String, String)> = signed_headers
        .split(';')
        .filter(|name| !name.starts_with("x-amz-") && *name != "host")
        .map(|name| (name.to_owned(), header(name).to_owned()))
        .chain(
            recorded
                .headers
                .iter()
                .filter(|(name, _)| name.starts_with("x-amz-"))
                .map(|(name, value)| (name.to_owned(), value.to_owned())),
        )
        .collect();
    let expected = Signer::for_service(
        service,
        access_key,
        secret_key,
        token.map(str::to_owned),
        REGION,
    )
    .sign(
        recorded.method.as_str(),
        header("host"),
        path,
        &query,
        &carried,
        payload_hash,
        instant(date),
    );
    let expected = &expected
        .iter()
        .find(|(name, _)| name == "authorization")
        .expect("an authorization header")
        .1;
    assert_eq!(
        authorization, expected,
        "{} {}",
        recorded.method, recorded.target
    );
}

/// Who signed each recorded request, and how it was answered.
fn signers(server: &Server) -> Vec<(String, u16)> {
    server
        .requests()
        .iter()
        .map(|recorded| {
            let key = recorded
                .headers
                .get("authorization")
                .and_then(signed_access_key)
                .unwrap_or_default()
                .to_owned();
            (key, recorded.status.code())
        })
        .collect()
}

// --- what is signed --------------------------------------------------------

#[test]
fn a_get_is_signed_for_its_service_over_the_path_and_the_query_as_sent() {
    let server = server();
    // A catalog's path: a bucket ARN as one encoded segment, a namespace
    // whose levels a unit separator joins.
    let target = "/iceberg/v1/arn%3Aaws%3As3tables%3Aeu-west-3%3A123456789012%3Abucket%2Flake\
                  /namespaces/a%1Fb/tables?pageToken=abc%2Fdef&pageSize=100";
    let response = http()
        .get(&url(&server, target))
        .expect("a request")
        .with_sigv4(&stated(None), "s3tables", REGION)
        .send()
        .expect("an answer");
    // The crate's own server refuses a control character in a path; the
    // request it recorded is what is under test.
    assert_eq!(response.status().code(), 400);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].target, target, "the path goes out as written");
    assert_eq!(
        requests[0].headers.get("host"),
        Some(format!("127.0.0.1:{}", server.port()).as_str()),
        "the port the URL names is part of the host"
    );
    assert_signed(
        &requests[0],
        "s3tables",
        (ACCESS_KEY, SECRET_KEY, None),
        EMPTY_PAYLOAD_SHA256,
        "host;x-amz-content-sha256;x-amz-date",
    );
}

#[test]
fn a_post_signs_the_hash_of_its_body_and_its_content_type() {
    let server = server();
    server.respond(
        Some(Method::Post),
        "/iceberg/v1/namespaces",
        Response::new(Status::OK).with_body("{}"),
    );
    let body = br#"{"namespace":["a","b"],"properties":{}}"#;
    let response = http()
        .post(&url(&server, "/iceberg/v1/namespaces"), body.to_vec())
        .expect("a request")
        .with_header("Content-Type", "application/json")
        .expect("a header")
        .with_header("X-Amz-Target", "Catalog.CreateNamespace")
        .expect("a header")
        .with_sigv4(&stated(Some("a-session-token")), "glue", REGION)
        .send()
        .expect("an answer");
    assert_eq!(response.status(), Status::OK);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body_len, body.len() as u64);
    assert_signed(
        &requests[0],
        "glue",
        (ACCESS_KEY, SECRET_KEY, Some("a-session-token")),
        &sha256_hex(body),
        "content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token;x-amz-target",
    );
}

#[test]
fn an_s3_family_service_signs_its_path_as_sent() {
    let server = server();
    http()
        .get(&url(&server, "/bucket/a%20b/c%2Fd.txt"))
        .expect("a request")
        .with_sigv4(&stated(None), "s3", REGION)
        .send()
        .expect("an answer");
    // The verifier's signer is the S3 one, which signs the path as sent:
    // a signature over the path encoded once more would not match it.
    assert_signed(
        &server.requests()[0],
        "s3",
        (ACCESS_KEY, SECRET_KEY, None),
        EMPTY_PAYLOAD_SHA256,
        "host;x-amz-content-sha256;x-amz-date",
    );
}

#[test]
fn every_attempt_is_signed_anew_and_a_retry_carries_its_own_signature() {
    let server = server();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    server.route(Some(Method::Get), "/flaky", move |_request| {
        Ok(if counted.fetch_add(1, Ordering::Relaxed) == 0 {
            Response::new(Status::new(503).expect("a status")).with_header("Retry-After", "0")?
        } else {
            Response::new(Status::OK).with_body("eventually")
        })
    });
    let response = http()
        .get(&url(&server, "/flaky"))
        .expect("a request")
        .with_sigv4(&stated(None), "execute-api", REGION)
        .send()
        .expect("an answer");
    assert_eq!(response.text().expect("a body"), "eventually");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    for recorded in &requests {
        assert_signed(
            recorded,
            "execute-api",
            (ACCESS_KEY, SECRET_KEY, None),
            EMPTY_PAYLOAD_SHA256,
            "host;x-amz-content-sha256;x-amz-date",
        );
    }
}

// --- refusals --------------------------------------------------------------

#[test]
fn no_credential_source_is_a_refusal_naming_the_service_and_nothing_goes_unsigned() {
    let server = server();
    let nobody = Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(scratch("request-nobody"))
        .with_metadata_disabled(true);
    let error = http()
        .get(&url(&server, "/v1/config"))
        .expect("a request")
        .with_sigv4(&nobody, "s3tables", REGION)
        .send()
        .expect_err("an unsigned request is refused");
    assert!(
        error
            .to_string()
            .contains("SigV4 for `s3tables` asked, no credential source answered"),
        "{error}"
    );
    assert_eq!(server.request_count(), 0);
}

#[test]
fn a_streamed_body_is_refused_for_a_service_that_signs_it_and_declared_unsigned_to_s3() {
    let server = server();
    let payload = b"a body read as it is sent".to_vec();

    let error = http()
        .put(&url(&server, "/v1/upload"), Vec::new())
        .expect("a request")
        .with_sigv4(&stated(None), "s3tables", REGION)
        .send_reader(&mut payload.as_slice(), payload.len() as u64)
        .expect_err("a body that cannot be hashed is not signed");
    assert!(matches!(error, Error::Io(_)), "{error:?}");
    assert!(
        error.to_string().contains("a streamed one cannot be read"),
        "{error}"
    );
    assert_eq!(server.request_count(), 0);

    http()
        .put(&url(&server, "/bucket/key"), Vec::new())
        .expect("a request")
        .with_sigv4(&stated(None), "s3", REGION)
        .send_reader(&mut payload.as_slice(), payload.len() as u64)
        .expect("an answer");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body_len, payload.len() as u64);
    assert_signed(
        &requests[0],
        "s3",
        (ACCESS_KEY, SECRET_KEY, None),
        UNSIGNED_PAYLOAD,
        "host;x-amz-content-sha256;x-amz-date",
    );
}

// --- a refused key ---------------------------------------------------------

/// Answer `path` with `refusal` while `refusing` is above zero, then `200`.
fn refusing(
    server: &Server,
    path: &'static str,
    refusals: usize,
    refusal: impl Fn() -> yggdryl::Result<Response> + Send + Sync + 'static,
) {
    let left = Arc::new(Mutex::new(refusals));
    server.route(None, path, move |_request| {
        let mut left = left.lock().expect("the counter");
        if *left > 0 {
            *left -= 1;
            return refusal();
        }
        Ok(Response::new(Status::OK).with_body("done"))
    });
}

fn json_refusal(status: u16, error_type: &'static str) -> yggdryl::Result<Response> {
    Ok(Response::new(Status::new(status).expect("a status"))
        .with_header("Content-Type", "application/x-amz-json-1.1")?
        .with_body(format!(
            r#"{{"__type":"com.amazonaws.catalog#{error_type}","message":"refused"}}"#
        )))
}

#[test]
fn a_set_dumped_anew_signs_the_post_the_service_refused_the_old_one_for() {
    let server = server();
    let directory = scratch("request-redumped");
    let credentials = directory.join("credentials");
    std::fs::write(&credentials, dumped("ASIAOLDDUMP")).expect("a dumped set");
    let session = reading_files(&directory);
    refusing(&server, "/v1/commit", 0, || json_refusal(400, "unused"));
    let commit = || {
        http()
            .post(&url(&server, "/v1/commit"), b"{}".to_vec())
            .expect("a request")
            .with_sigv4(&session, "s3tables", REGION)
            .send()
            .expect("an answer")
    };
    assert_eq!(commit().status(), Status::OK);

    // The developer dumps a fresh set; the process still holds the old one
    // until the service says it lapsed - then the file is read again and the
    // same request goes once more, a POST included, signed with what the
    // file says now.
    std::fs::write(&credentials, dumped("ASIANEWDUMPED")).expect("a set dumped anew");
    refusing(&server, "/v1/commit", 1, || {
        json_refusal(400, "ExpiredTokenException")
    });
    server.clear_requests();
    let response = commit();
    assert_eq!(response.text().expect("a body"), "done");
    assert_eq!(
        signers(&server),
        [
            ("ASIAOLDDUMP".to_owned(), 400),
            ("ASIANEWDUMPED".to_owned(), 200)
        ]
    );
    let requests = server.requests();
    assert_signed(
        &requests[1],
        "s3tables",
        (
            "ASIANEWDUMPED",
            "ASIANEWDUMPED-secret",
            Some("ASIANEWDUMPED-token"),
        ),
        &sha256_hex(b"{}"),
        "host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_key_a_json_service_does_not_recognize_is_read_again_by_the_error_type_header() {
    let server = server();
    let directory = scratch("request-unrecognized");
    let credentials = directory.join("credentials");
    std::fs::write(&credentials, dumped("AKIAJUSTMADE")).expect("a dumped set");
    let session = reading_files(&directory);
    assert!(
        session
            .credentials(SystemTime::now())
            .expect("a walk")
            .is_some()
    );
    std::fs::write(&credentials, dumped("AKIAPROPAGATED")).expect("a set dumped anew");
    // The code in `x-amzn-ErrorType`, a documentation URL after it, and a
    // body that names nothing.
    refusing(&server, "/v1/tables", 1, || {
        Ok(Response::new(Status::new(403).expect("a status"))
            .with_header(
                "x-amzn-ErrorType",
                "UnrecognizedClientException:http://internal.amazon.com/coral/",
            )?
            .with_body(r#"{"message":"The security token included in the request is invalid."}"#))
    });
    let response = http()
        .get(&url(&server, "/v1/tables"))
        .expect("a request")
        .with_sigv4(&session, "s3tables", REGION)
        .send()
        .expect("an answer");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        signers(&server),
        [
            ("AKIAJUSTMADE".to_owned(), 403),
            ("AKIAPROPAGATED".to_owned(), 200)
        ]
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_stated_set_the_service_refuses_is_sent_once_and_its_answer_handed_back_whole() {
    let server = server();
    refusing(&server, "/v1/tables", 5, || {
        json_refusal(403, "ExpiredTokenException")
    });
    let response = http()
        .get(&url(&server, "/v1/tables"))
        .expect("a request")
        .with_sigv4(&stated(Some("a-lapsed-token")), "s3tables", REGION)
        .send()
        .expect("the refusal is an answer");
    // The caller stated that set: the session answers it again, and the same
    // set would be refused the same way.
    assert_eq!(response.status().code(), 403);
    assert!(
        response
            .text()
            .expect("a body")
            .contains("ExpiredTokenException"),
        "the body the rule read is handed back in front"
    );
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_refusal_of_the_request_rather_than_its_key_is_never_sent_again() {
    let server = server();
    let directory = scratch("request-denied");
    std::fs::write(directory.join("credentials"), dumped("ASIAALLOWEDNOT")).expect("a set");
    let session = reading_files(&directory);
    for (status, error_type) in [
        (403, "AccessDeniedException"),
        (400, "ValidationException"),
        (404, "ExpiredTokenException"),
    ] {
        server.clear_requests();
        refusing(&server, "/v1/tables", 1, move || {
            json_refusal(status, error_type)
        });
        let response = http()
            .get(&url(&server, "/v1/tables"))
            .expect("a request")
            .with_sigv4(&session, "s3tables", REGION)
            .send()
            .expect("an answer");
        assert_eq!(response.status().code(), status, "{error_type}");
        assert_eq!(server.request_count(), 1, "{status} {error_type}");
    }
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_head_refused_with_no_body_goes_once_more_only_for_another_set() {
    let server = server();
    let directory = scratch("request-head");
    let credentials = directory.join("credentials");
    std::fs::write(&credentials, dumped("ASIAHEADOLD")).expect("a dumped set");
    let session = reading_files(&directory);
    assert!(
        session
            .credentials(SystemTime::now())
            .expect("a walk")
            .is_some()
    );
    let head = || {
        http()
            .head(&url(&server, "/v1/table"))
            .expect("a request")
            .with_sigv4(&session, "s3", REGION)
            .send()
            .expect("an answer")
    };

    std::fs::write(&credentials, dumped("ASIAHEADNEWER")).expect("a set dumped anew");
    refusing(&server, "/v1/table", 1, || {
        Ok(Response::new(Status::new(400).expect("a status")))
    });
    assert_eq!(head().status(), Status::OK);
    assert_eq!(
        signers(&server),
        [
            ("ASIAHEADOLD".to_owned(), 400),
            ("ASIAHEADNEWER".to_owned(), 200)
        ]
    );

    // Nothing moved: the same set answers, so the bare refusal stands.
    server.clear_requests();
    refusing(&server, "/v1/table", 1, || {
        Ok(Response::new(Status::new(403).expect("a status")))
    });
    assert_eq!(head().status().code(), 403);
    assert_eq!(server.request_count(), 1);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_redirect_to_another_origin_is_neither_signed_nor_sent_again() {
    let origin = server();
    let elsewhere = server();
    let moved = url(&elsewhere, "/there");
    origin.route(None, "/away", move |_request| {
        Response::new(Status::new(307).expect("a status")).with_header("Location", &moved)
    });
    refusing(&elsewhere, "/there", 1, || {
        json_refusal(403, "ExpiredTokenException")
    });
    let response = http()
        .get(&url(&origin, "/away"))
        .expect("a request")
        .with_sigv4(&stated(None), "s3tables", REGION)
        .send()
        .expect("an answer");
    assert_eq!(response.status().code(), 403);
    let landed = elsewhere.requests();
    assert_eq!(landed.len(), 1, "no signature was sent, so none is mended");
    for name in ["authorization", "x-amz-date", "x-amz-content-sha256"] {
        assert_eq!(landed[0].headers.get(name), None, "{name}");
    }
    assert!(origin.requests()[0].headers.get("authorization").is_some());
}

#[test]
fn the_headers_the_hook_adds_are_the_signers_and_nothing_else() {
    // The internals door signs one attempt at a stated instant: what the
    // benchmark measures, and the one place the instant is the test's.
    let session = stated(Some("token"));
    let target =
        Url::from_str("https://s3tables.eu-west-3.amazonaws.com/iceberg/v1/config").expect("a URL");
    let headers = yggdryl::internals::aws_request::signed_headers(
        &session,
        "s3tables",
        REGION,
        Method::Get,
        &target,
        &Headers::new(),
        None,
        UNIX_EPOCH + Duration::from_secs(1_440_938_160),
    )
    .expect("signed headers");
    let names: Vec<&str> = headers.iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        [
            "authorization",
            "x-amz-content-sha256",
            "x-amz-date",
            "x-amz-security-token"
        ]
    );
    assert_eq!(headers.get("x-amz-date"), Some("20150830T123600Z"));
    assert!(
        headers
            .get("authorization")
            .expect("an authorization")
            .contains("/20150830/eu-west-3/s3tables/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token, "),
    );
}
