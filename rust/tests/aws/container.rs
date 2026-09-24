//! `rust/src/aws/container.rs`: the credential endpoint a container platform
//! serves, and the hosts a task's token is ever presented to.
//!
//! The host rule is pinned by spelling through `yggdryl::internals`, because
//! it is the whole of what keeps a token off a stranger's host. The fetch is
//! driven over the identity fake through a sealed session whose environment
//! names the endpoint, and every request it makes is counted: a refusal that
//! is meant to come before the wire is proven by a count of zero.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::aws::{Credentials, Session};
use yggdryl::internals::aws_container::is_allowed_full_uri;

use crate::identity::{CONTAINER_PATH, Identity};
use crate::mod_::{scratch, sealed};

/// An expiry no test outlives, and the instant it names.
const FAR: &str = "2099-01-01T00:00:00Z";
const FAR_SECONDS: u64 = 4_070_908_800;

/// What a walk that ends at the instance metadata service asks it, in order.
const IMDS_WALK: [&str; 3] = [
    "PUT /latest/api/token",
    "GET /latest/meta-data/iam/security-credentials/",
    "GET /latest/meta-data/iam/security-credentials/instance-role",
];

/// The keys the fake's instance role answers.
const INSTANCE_KEY: &str = "ASIAINSTANCEROLE";

/// A sealed session reading exactly `pairs` as its environment.
fn walled(identity: &Identity, name: &str, pairs: &[(&str, &str)]) -> Session {
    sealed(identity, name).with_variables(pairs.iter().copied())
}

/// The set the session signs with now, which the test expects to exist.
fn found(session: &Session) -> Credentials {
    session
        .credentials(SystemTime::now())
        .expect("a walk that answers")
        .expect("a credential set rather than unsigned requests")
}

/// The refusal the session answers now, rendered.
fn refused(session: &Session) -> String {
    session
        .credentials(SystemTime::now())
        .expect_err("a walk that refuses")
        .to_string()
}

/// Every request the fake handled, as `METHOD path`.
fn shape(identity: &Identity) -> Vec<String> {
    identity
        .requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

/// A file holding `text` in a directory of the test's own.
fn token_file(name: &str, text: &str) -> PathBuf {
    let path = scratch(name).join("token");
    std::fs::write(&path, text).expect("a token file the test writes");
    path
}

// --- the host rule ------------------------------------------------------------

#[test]
fn https_is_accepted_on_any_host() {
    for url in [
        "https://credentials.example.com/v1/task",
        "HTTPS://credentials.example.com/v1/task",
        "https://10.0.0.1:8443/creds",
        "https://169.254.169.254/latest",
    ] {
        assert!(
            is_allowed_full_uri(url),
            "{url} is https, so a token may travel to it"
        );
    }
}

#[test]
fn plain_http_is_accepted_on_loopback_and_on_the_container_agent_addresses() {
    for url in [
        "http://localhost/creds",
        "http://localhost:51679/creds",
        "http://LocalHost/creds",
        "HTTP://127.0.0.1/creds",
        "http://127.0.0.1:8080/v2/credentials/task",
        "http://127.1.2.3/creds",
        "http://127.0.0.1?query",
        "http://[::1]/creds",
        "http://[::1]:8080/creds",
        "http://169.254.170.2/v2/credentials/abc",
        "http://169.254.170.23/v1/credentials",
        "http://[fd00:ec2::23]/v1/credentials",
        "http://[FD00:EC2::23]:80/v1/credentials",
        "http://user:secret@127.0.0.1/creds",
    ] {
        assert!(
            is_allowed_full_uri(url),
            "{url} is a loopback or container-agent address"
        );
    }
}

#[test]
fn plain_http_to_any_other_host_is_refused() {
    for url in [
        "http://credentials.example.com/creds",
        "http://10.0.0.1/creds",
        "http://0.0.0.0/creds",
        "http://169.254.169.254/latest/meta-data/",
        "http://169.254.170.3/creds",
        "http://169.254.170.22/creds",
        "http://[fd00:ec2::254]/latest",
        "http://[fd00:ec2::24]/creds",
        "http://[::2]/creds",
        "http://localhost.attacker.example/creds",
        "http://169.254.170.2.attacker.example/creds",
        "http://attacker.example/127.0.0.1",
        "http://attacker.example?host=127.0.0.1",
        "http://attacker.example#@127.0.0.1",
    ] {
        assert!(
            !is_allowed_full_uri(url),
            "{url} is neither https nor a loopback or container-agent address"
        );
    }
}

#[test]
fn user_information_before_the_host_never_passes_for_the_host() {
    for url in [
        "http://127.0.0.1@attacker.example/creds",
        "http://127.0.0.1:8080@attacker.example/creds",
        "http://localhost@attacker.example/creds",
        "http://169.254.170.2@attacker.example/creds",
    ] {
        assert!(
            !is_allowed_full_uri(url),
            "the host of {url} is what follows the last @"
        );
    }
}

#[test]
fn a_host_name_that_merely_begins_like_a_loopback_address_is_refused() {
    // `127.x` is a loopback address only when the whole host is an IPv4
    // literal; a name that starts with those digits resolves wherever its
    // owner points it.
    for url in [
        "http://127.0.0.1.attacker.example/creds",
        "http://127.attacker.example/creds",
        "http://127.0.0.1.attacker.example:8080/creds",
    ] {
        assert!(
            !is_allowed_full_uri(url),
            "{url} names a host under attacker.example, not a loopback address"
        );
    }
}

#[test]
fn a_scheme_other_than_http_or_https_or_none_at_all_is_refused() {
    for url in [
        "ftp://127.0.0.1/creds",
        "file:///etc/passwd",
        "ws://localhost/creds",
        "127.0.0.1/creds",
        "localhost:8080/creds",
        "//127.0.0.1/creds",
        "",
    ] {
        assert!(
            !is_allowed_full_uri(url),
            "{url:?} is refused by its scheme"
        );
    }
}

// --- the fetch, over the fake ---------------------------------------------------

#[test]
fn a_full_uri_answers_the_container_s_keys_in_one_request_and_names_the_source() {
    let identity = Identity::start();
    identity.set_container_credentials("ASIATASK", FAR);
    let uri = identity.container_uri();
    let session = walled(
        &identity,
        "container-full-uri",
        &[("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str())],
    );

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIATASK");
    assert_eq!(
        keys.session_token(),
        Some("container-token"),
        "the document's Token is the session token"
    );
    assert_eq!(
        keys.expires_at(),
        Some(UNIX_EPOCH + Duration::from_secs(FAR_SECONDS)),
        "the document's Expiration is when the set lapses"
    );
    assert_eq!(session.credential_source(), Some("container"));
    assert_eq!(
        shape(&identity),
        [format!("GET {CONTAINER_PATH}")],
        "one request, and the instance metadata service is never reached"
    );
    assert_eq!(
        identity.requests()[0].header("authorization"),
        None,
        "no token was named, so none is presented"
    );
}

#[test]
fn the_token_variable_is_presented_as_the_authorization_header() {
    let identity = Identity::start();
    identity.require_container_authorization(Some("task-token-1"));
    let uri = identity.container_uri();
    let session = walled(
        &identity,
        "container-token-variable",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "  task-token-1 "),
        ],
    );

    assert_eq!(found(&session).access_key_id(), "ASIACONTAINER");
    assert_eq!(session.credential_source(), Some("container"));
    assert_eq!(
        identity.request_count(),
        1,
        "the endpoint accepted the token at once"
    );
    assert_eq!(
        identity.requests()[0].header("authorization"),
        Some("task-token-1"),
        "the variable's token, trimmed"
    );
}

#[test]
fn the_token_file_is_read_trimmed_of_its_line_ending_and_presented() {
    let identity = Identity::start();
    identity.require_container_authorization(Some("task-token-2"));
    let file = token_file("container-token-file", "task-token-2\r\n");
    let (uri, location) = (identity.container_uri(), file.display().to_string());
    let session = walled(
        &identity,
        "container-token-file-session",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", location.as_str()),
        ],
    );

    assert_eq!(found(&session).access_key_id(), "ASIACONTAINER");
    assert_eq!(session.credential_source(), Some("container"));
    assert_eq!(identity.request_count(), 1);
    assert_eq!(
        identity.requests()[0].header("authorization"),
        Some("task-token-2"),
        "the file's token, without its CRLF"
    );
}

#[test]
fn a_token_holding_a_line_break_is_refused_before_any_request() {
    let identity = Identity::start();
    let uri = identity.container_uri();
    let session = walled(
        &identity,
        "container-token-newline",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            (
                "AWS_CONTAINER_AUTHORIZATION_TOKEN",
                "task-token\r\nX-Injected: header",
            ),
        ],
    )
    .with_metadata_disabled(true);

    let message = refused(&session);
    assert!(message.contains("container"), "{message}");
    assert!(message.contains("line break"), "{message}");
    assert!(
        !message.contains("task-token") && !message.contains("X-Injected"),
        "the refusal never renders the token: {message}"
    );
    assert_eq!(identity.request_count(), 0, "no header was ever sent");

    let file = token_file("container-token-file-newline", "task-token\nsecond-line\n");
    let location = file.display().to_string();
    let from_file = walled(
        &identity,
        "container-token-file-newline-session",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", location.as_str()),
        ],
    )
    .with_metadata_disabled(true);
    let message = refused(&from_file);
    assert!(message.contains("line break"), "{message}");
    assert_eq!(
        identity.request_count(),
        0,
        "a line break inside the file's token is refused the same way"
    );
}

#[test]
fn a_token_file_that_cannot_be_read_is_refused_by_name_before_any_request() {
    let identity = Identity::start();
    let missing = scratch("container-token-missing").join("no-such-token");
    let (uri, location) = (identity.container_uri(), missing.display().to_string());
    let session = walled(
        &identity,
        "container-token-missing-session",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", location.as_str()),
        ],
    )
    .with_metadata_disabled(true);

    let message = refused(&session);
    assert!(
        message.contains("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE"),
        "{message}"
    );
    assert!(message.contains(&location), "the path is named: {message}");
    assert_eq!(identity.request_count(), 0);
}

#[test]
fn a_full_uri_on_a_refused_host_is_a_recorded_failure_that_sends_nothing() {
    let identity = Identity::start();
    // The fake's own port under the unspecified address: were the rule to
    // let it through, the request would land on the fake and be counted
    // rather than leave the machine.
    let refused_uri = identity.container_uri().replace("127.0.0.1", "0.0.0.0");
    let pairs = [
        ("AWS_CONTAINER_CREDENTIALS_FULL_URI", refused_uri.as_str()),
        ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "task-token-3"),
    ];
    let session = walled(&identity, "container-refused-host", &pairs).with_metadata_disabled(true);

    let message = refused(&session);
    assert!(
        message.contains("AWS_CONTAINER_CREDENTIALS_FULL_URI"),
        "{message}"
    );
    assert!(
        message.contains(&refused_uri),
        "the URI is named: {message}"
    );
    assert!(
        !message.contains("task-token-3"),
        "the refusal never renders the token: {message}"
    );
    assert_eq!(identity.request_count(), 0, "the token went nowhere");

    // With the instance answering, the refusal is one source passed over.
    let walking = walled(&identity, "container-refused-host-walks-on", &pairs);
    assert_eq!(found(&walking).access_key_id(), INSTANCE_KEY);
    assert_eq!(walking.credential_source(), Some("instance metadata"));
    assert_eq!(
        shape(&identity),
        IMDS_WALK,
        "the container endpoint is never asked"
    );
}

#[test]
fn an_endpoint_that_fails_with_a_5xx_is_asked_again_up_to_three_times() {
    let identity = Identity::start();
    identity.fail_next(500, 2);
    let uri = identity.container_uri();
    let session = walled(
        &identity,
        "container-5xx",
        &[("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str())],
    );

    assert_eq!(found(&session).access_key_id(), "ASIACONTAINER");
    assert_eq!(session.credential_source(), Some("container"));
    assert_eq!(
        shape(&identity),
        vec![format!("GET {CONTAINER_PATH}"); 3],
        "two failures, then the answer"
    );
    let statuses: Vec<u16> = identity
        .requests()
        .iter()
        .map(|request| request.status)
        .collect();
    assert_eq!(statuses, [500_u16, 500, 200]);
}

#[test]
fn an_endpoint_that_fails_every_attempt_is_passed_over_for_the_instance() {
    let identity = Identity::start();
    identity.fail_next(500, 3);
    let uri = identity.container_uri();
    let session = walled(
        &identity,
        "container-5xx-exhausted",
        &[("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str())],
    );

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(session.credential_source(), Some("instance metadata"));
    let mut expected = vec![format!("GET {CONTAINER_PATH}"); 3];
    expected.extend(IMDS_WALK.map(str::to_owned));
    assert_eq!(
        shape(&identity),
        expected,
        "three attempts at the container, then the instance's walk"
    );
}

#[test]
fn a_4xx_is_a_failure_naming_the_status_and_is_not_asked_again() {
    let identity = Identity::start();
    identity.require_container_authorization(Some("task-token-right"));
    identity.set_imds_role(None);
    let uri = identity.container_uri();
    let session = walled(
        &identity,
        "container-401",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "task-token-wrong"),
        ],
    );

    let message = refused(&session);
    assert!(message.contains("container"), "{message}");
    assert!(
        message.contains("answered 401"),
        "the status is named: {message}"
    );
    assert!(
        !message.contains("task-token-wrong"),
        "the refusal never renders the token: {message}"
    );
    assert_eq!(
        shape(&identity),
        [
            format!("GET {CONTAINER_PATH}"),
            "PUT /latest/api/token".to_owned(),
            "GET /latest/meta-data/iam/security-credentials/".to_owned(),
        ],
        "one refused request, then the instance, which has no role"
    );
    assert_eq!(identity.requests()[0].status, 401);
}
