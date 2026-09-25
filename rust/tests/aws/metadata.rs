//! `rust/src/aws/metadata.rs`: the EC2 instance metadata service, which
//! serves an instance its own role and its own region.
//!
//! Every walk runs over the identity fake, reached through a sealed session
//! that states nothing else, or over a loopback port that answers nothing -
//! released, silent, or hanging up - so "not an instance" is exercised
//! without a link-local address. The requests a walk makes are the contract,
//! so they are counted: the service is asked for an `IMDSv2` token, then the
//! role's name, then the role's keys, and nothing more.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use yggdryl::aws::{Credentials, Session};

use crate::identity::{IMDS_TOKEN, Identity};
use crate::mod_::{scratch, sealed};

/// What one walk that ends at the instance metadata service asks it, in order.
const IMDS_WALK: [&str; 3] = [
    "PUT /latest/api/token",
    "GET /latest/meta-data/iam/security-credentials/",
    "GET /latest/meta-data/iam/security-credentials/instance-role",
];

/// The keys the fake's instance role answers.
const INSTANCE_KEY: &str = "ASIAINSTANCEROLE";

/// The set the session signs with now, which the test expects to exist.
fn found(session: &Session) -> Credentials {
    session
        .credentials(SystemTime::now())
        .expect("a walk that answers")
        .expect("a credential set rather than unsigned requests")
}

/// What the session answers now, which the test expects not to be a refusal.
fn answered(session: &Session) -> Option<Credentials> {
    session
        .credentials(SystemTime::now())
        .expect("an absent service is not a failure")
}

/// Every request the fake handled, as `METHOD path`.
fn shape(identity: &Identity) -> Vec<String> {
    identity
        .requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

/// A session that reads `pairs` as its whole environment, keeps its files
/// under a directory of its own, and reaches no endpoint but the metadata
/// service at `endpoint`.
fn toward(endpoint: &str, name: &str, pairs: &[(&str, &str)]) -> Session {
    Session::new()
        .with_variables(pairs.iter().copied())
        .with_directory(scratch(name))
        .with_metadata_endpoint(endpoint)
}

/// An endpoint on a loopback port nobody listens on: bound, read, released.
fn released_endpoint() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback listener");
    let address = listener
        .local_addr()
        .expect("a bound listener has an address");
    drop(listener);
    format!("http://{address}")
}

/// A loopback listener that accepts every connection and hangs up on it at
/// once, counting them: a service that is there and never answers, whose
/// every attempt is one connection. Dropping it stops the listener.
struct Hangup {
    address: String,
    accepted: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Hangup {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback listener");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address")
            .to_string();
        let accepted = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let (counted, stop) = (Arc::clone(&accepted), Arc::clone(&stopping));
        let thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = connection {
                    // Counted before the hang-up, so the client that sees it
                    // has been counted.
                    counted.fetch_add(1, Ordering::SeqCst);
                    drop(stream);
                }
            }
        });
        Self {
            address,
            accepted,
            stopping,
            thread: Some(thread),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The connections hung up on so far.
    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for Hangup {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(&self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How many connections one walk toward a hanging-up service made, for a
/// session reading `pairs` and, when given, stating `attempts`.
fn attempts_made(
    name: &str,
    pairs: &[(&str, &str)],
    config: Option<&str>,
    attempts: Option<u32>,
) -> usize {
    let service = Hangup::start();
    // The bound is generous so that only a hang-up ends an attempt.
    let mut session =
        toward(&service.endpoint(), name, pairs).with_metadata_timeout(Duration::from_secs(5));
    if let Some(config) = config {
        session = session.with_config_text(config);
    }
    if let Some(attempts) = attempts {
        session = session.with_metadata_attempts(attempts);
    }
    assert_eq!(
        answered(&session),
        None,
        "a service that never answers is no instance"
    );
    service.accepted()
}

// --- the role's keys ------------------------------------------------------------

#[test]
fn an_instance_answers_its_role_s_keys_through_a_token_the_listing_and_the_role() {
    let identity = Identity::start();
    let session = sealed(&identity, "imds-role");

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), INSTANCE_KEY);
    assert_eq!(keys.session_token(), Some("token-of-instance-role"));
    let expiry = keys.expires_at().expect("an instance role's keys lapse");
    assert!(
        expiry > SystemTime::now() + Duration::from_secs(5 * 3600),
        "the document's Expiration, six hours out, is the set's"
    );
    assert_eq!(session.credential_source(), Some("instance metadata"));

    assert_eq!(
        shape(&identity),
        IMDS_WALK,
        "a token, the listing, the role's keys - and nothing else"
    );
    let requests = identity.requests();
    assert_eq!(
        requests[0].header("x-aws-ec2-metadata-token-ttl-seconds"),
        Some("21600"),
        "the token is asked for six hours"
    );
    assert_eq!(
        requests[0].header("x-aws-ec2-metadata-token"),
        None,
        "the token request carries no token"
    );
    for read in &requests[1..] {
        assert_eq!(
            read.header("x-aws-ec2-metadata-token"),
            Some(IMDS_TOKEN),
            "every read presents the session token: {read:?}"
        );
    }
}

#[test]
fn a_service_that_refuses_reads_without_the_token_answers_because_every_read_presents_it() {
    let identity = Identity::start();
    identity.imds_v2_only(true);
    let session = sealed(&identity, "imds-v2-only");

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(session.credential_source(), Some("instance metadata"));
    assert_eq!(shape(&identity), IMDS_WALK);
    assert!(
        identity
            .requests()
            .iter()
            .all(|request| request.status == 200),
        "no read was refused: {:?}",
        identity.requests()
    );
}

#[test]
fn an_instance_without_a_role_is_absent_and_the_walk_answers_unsigned() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let session = sealed(&identity, "imds-no-role");

    assert_eq!(answered(&session), None);
    assert_eq!(session.credential_source(), None);
    assert_eq!(
        shape(&identity),
        [
            "PUT /latest/api/token",
            "GET /latest/meta-data/iam/security-credentials/",
        ],
        "the listing's 404 ends the walk before a role is asked for"
    );
}

#[test]
fn a_service_that_will_not_issue_a_token_is_read_without_one_unless_v1_is_disabled() {
    let identity = Identity::start();
    identity.fail_next(403, 1);
    let session = sealed(&identity, "imds-v1");

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(shape(&identity), IMDS_WALK);
    let requests = identity.requests();
    assert_eq!(requests[0].status, 403);
    for read in &requests[1..] {
        assert_eq!(
            read.header("x-aws-ec2-metadata-token"),
            None,
            "without a token the reads go out as IMDSv1: {read:?}"
        );
    }

    let identity = Identity::start();
    identity.fail_next(403, 1);
    let session = sealed(&identity, "imds-v1-disabled")
        .with_variables([("AWS_EC2_METADATA_V1_DISABLED", "true")]);
    assert_eq!(answered(&session), None);
    assert_eq!(
        shape(&identity),
        ["PUT /latest/api/token"],
        "with IMDSv1 disabled no read goes out without the token"
    );
}

// --- disabled -----------------------------------------------------------------

#[test]
fn a_disabled_service_is_asked_nothing_whichever_knob_disabled_it() {
    let identity = Identity::start();
    let cases = [
        (
            "by the session",
            sealed(&identity, "imds-off-session").with_metadata_disabled(true),
        ),
        (
            "by AWS_EC2_METADATA_DISABLED",
            sealed(&identity, "imds-off-variable")
                .with_variables([("AWS_EC2_METADATA_DISABLED", "true")]),
        ),
        (
            "by the profile's ec2_metadata_disabled",
            sealed(&identity, "imds-off-profile")
                .with_config_text("[default]\nec2_metadata_disabled = true\n"),
        ),
        (
            "by a session that consults no environment",
            sealed(&identity, "imds-off-environment").with_environment(false),
        ),
    ];
    for (how, session) in cases {
        assert_eq!(answered(&session), None, "disabled {how}");
        assert_eq!(session.credential_source(), None, "disabled {how}");
        assert_eq!(session.instance_region(), None, "disabled {how}");
        assert_eq!(
            identity.request_count(),
            0,
            "disabled {how} means no request at all"
        );
    }
}

#[test]
fn a_session_that_enables_the_service_overrides_the_variable_that_disables_it() {
    let identity = Identity::start();
    let session = sealed(&identity, "imds-on-over-variable")
        .with_variables([("AWS_EC2_METADATA_DISABLED", "true")])
        .with_metadata_disabled(false);

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(shape(&identity), IMDS_WALK);
}

// --- the region -----------------------------------------------------------------

#[test]
fn the_instance_region_is_the_identity_document_s_read_under_the_token() {
    let identity = Identity::start();
    identity.set_imds_region("ap-northeast-2");
    let session = sealed(&identity, "imds-region");

    assert_eq!(session.instance_region().as_deref(), Some("ap-northeast-2"));
    assert_eq!(
        shape(&identity),
        [
            "PUT /latest/api/token",
            "GET /latest/dynamic/instance-identity/document",
        ]
    );
    assert_eq!(
        identity.requests()[1].header("x-aws-ec2-metadata-token"),
        Some(IMDS_TOKEN)
    );
}

// --- no instance ----------------------------------------------------------------

#[test]
fn an_address_nobody_listens_on_is_no_instance_answered_at_once() {
    let session = toward(&released_endpoint(), "imds-released", &[])
        .with_metadata_timeout(Duration::from_secs(1));

    let started = Instant::now();
    assert_eq!(answered(&session), None, "no service is not a failure");
    assert_eq!(session.instance_region(), None);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "a refused connection is answered at once, not waited on: {elapsed:?}"
    );
}

#[test]
fn a_service_that_accepts_and_never_answers_is_no_instance_after_the_timeout() {
    // Bound and never accepted from: the kernel completes the connection,
    // and the request waits for an answer that never comes.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback listener");
    let address = listener
        .local_addr()
        .expect("a bound listener has an address");
    let session = toward(&format!("http://{address}"), "imds-silent", &[])
        .with_metadata_timeout(Duration::from_secs(1));

    let started = Instant::now();
    assert_eq!(
        answered(&session),
        None,
        "silence is read as not an instance"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "one attempt bounded by the one-second timeout: {elapsed:?}"
    );
    drop(listener);
}

// --- where the service is ---------------------------------------------------------

#[test]
fn aws_ec2_metadata_service_endpoint_points_the_session_at_the_service_trailing_slash_and_all() {
    let identity = Identity::start();
    let endpoint = format!("{}/", identity.endpoint());
    let session = Session::new()
        .with_variables([("AWS_EC2_METADATA_SERVICE_ENDPOINT", endpoint.as_str())])
        .with_directory(scratch("imds-endpoint-variable"))
        .with_endpoint_url(identity.endpoint());

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(
        shape(&identity),
        IMDS_WALK,
        "the trailing slash is dropped, so every path is the service's own"
    );
}

#[test]
fn the_profile_s_ec2_metadata_service_endpoint_points_the_session_at_the_service() {
    let identity = Identity::start();
    let config = format!(
        "[default]\nec2_metadata_service_endpoint = {}\n",
        identity.endpoint()
    );
    let session = Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(scratch("imds-endpoint-profile"))
        .with_endpoint_url(identity.endpoint())
        .with_config_text(config);

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(shape(&identity), IMDS_WALK);
}

#[test]
fn an_endpoint_stated_on_the_session_beats_the_variable() {
    let identity = Identity::start();
    let released = released_endpoint();
    let session = toward(
        &identity.endpoint(),
        "imds-endpoint-stated",
        &[("AWS_EC2_METADATA_SERVICE_ENDPOINT", released.as_str())],
    );

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(shape(&identity), IMDS_WALK);
}

// --- how often it is asked ------------------------------------------------------

#[test]
fn the_attempts_are_the_stated_count_then_the_variable_then_the_profile_then_one() {
    let variable = [("AWS_METADATA_SERVICE_NUM_ATTEMPTS", "3")];
    let profile = "[default]\nmetadata_service_num_attempts = 2\n";

    assert_eq!(
        attempts_made("imds-attempts-default", &[], None, None),
        1,
        "one attempt when nothing names a count"
    );
    assert_eq!(
        attempts_made("imds-attempts-variable", &variable, None, None),
        3,
        "AWS_METADATA_SERVICE_NUM_ATTEMPTS"
    );
    assert_eq!(
        attempts_made("imds-attempts-profile", &[], Some(profile), None),
        2,
        "the profile's metadata_service_num_attempts"
    );
    assert_eq!(
        attempts_made("imds-attempts-over-profile", &variable, Some(profile), None),
        3,
        "the variable beats the profile"
    );
    assert_eq!(
        attempts_made("imds-attempts-stated", &variable, Some(profile), Some(4)),
        4,
        "the stated count beats both"
    );
}

#[test]
fn a_count_that_is_not_a_positive_number_is_one_attempt() {
    for count in ["0", "many", "-2"] {
        assert_eq!(
            attempts_made(
                "imds-attempts-malformed",
                &[("AWS_METADATA_SERVICE_NUM_ATTEMPTS", count)],
                None,
                None,
            ),
            1,
            "AWS_METADATA_SERVICE_NUM_ATTEMPTS={count}"
        );
    }
    assert_eq!(
        attempts_made("imds-attempts-stated-zero", &[], None, Some(0)),
        1,
        "a stated zero is one attempt"
    );
}

#[test]
fn under_v2_only_a_token_the_service_fails_to_issue_with_one_attempt_leaves_the_source_absent() {
    let identity = Identity::start();
    identity.imds_v2_only(true);
    identity.fail_next(503, 1);
    let session = sealed(&identity, "imds-5xx-one-attempt");

    assert_eq!(answered(&session), None, "a failing service is no instance");
    let requests = identity.requests();
    assert_eq!(
        (requests[0].method.as_str(), requests[0].status),
        ("PUT", 503)
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "PUT")
            .count(),
        1,
        "one attempt asks for the token once"
    );
}

#[test]
fn a_token_the_service_fails_to_issue_with_a_5xx_is_asked_for_again_within_the_attempts() {
    let identity = Identity::start();
    identity.imds_v2_only(true);
    identity.fail_next(503, 1);
    let session = sealed(&identity, "imds-5xx-two-attempts")
        .with_variables([("AWS_METADATA_SERVICE_NUM_ATTEMPTS", "2")]);

    assert_eq!(
        found(&session).access_key_id(),
        INSTANCE_KEY,
        "the second attempt is issued a token, and the reads it signs answer"
    );
    assert_eq!(
        shape(&identity),
        [
            "PUT /latest/api/token",
            "PUT /latest/api/token",
            "GET /latest/meta-data/iam/security-credentials/",
            "GET /latest/meta-data/iam/security-credentials/instance-role",
        ]
    );
    let statuses: Vec<u16> = identity
        .requests()
        .iter()
        .map(|request| request.status)
        .collect();
    assert_eq!(statuses, [503_u16, 200, 200, 200]);
}
