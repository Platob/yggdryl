//! `rust/src/aws/credentials.rs`: one credential set, and the document it is
//! read out of.
//!
//! What a set is - a pair, a token that makes it temporary, an account, an
//! expiry that the session's lease reads through `Expiring`, and a `Debug`
//! that never renders a secret - is pinned over sets a test builds; what the
//! metadata services, the container endpoint and a `credential_process` answer
//! is pinned over documents a test spells. How the instant spellings read on
//! their own is `rust/tests/auth/lease.rs`'s; here they are pinned as a
//! document's `Expiration`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::aws::Credentials;
use yggdryl::internals::auth_lease::{Expiring, iso8601, lapses_within};
use yggdryl::internals::aws_credentials::parse_document;

/// `2026-09-05T12:34:56Z`, the instant every document here lapses at.
const EXPIRY_SECONDS: u64 = 1_788_611_696;

/// The session refreshes a set this long before it lapses.
const WINDOW: Duration = Duration::from_secs(15 * 60);

fn expiry() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(EXPIRY_SECONDS)
}

/// A document holding a pair and `expiration` as its `Expiration`.
fn expiring(expiration: &str) -> Vec<u8> {
    format!(
        r#"{{"AccessKeyId":"ASIAEXPIRING","SecretAccessKey":"expiring-secret","Expiration":"{expiration}"}}"#
    )
    .into_bytes()
}

/// The text of a refusal, which is invalid data.
fn refusal(answer: yggdryl::Result<Credentials>) -> String {
    let error = answer.expect_err("a refusal");
    assert!(
        matches!(&error, yggdryl::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData),
        "a document's refusal is invalid data: {error:?}"
    );
    error.to_string()
}

mod set {
    use super::*;

    #[test]
    fn a_long_lived_pair_names_no_token_no_expiry_and_no_account() {
        let keys = Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        assert_eq!(keys.access_key_id(), "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(keys.session_token(), None);
        assert_eq!(keys.expires_at(), None);
        assert_eq!(keys.account_id(), None);
        assert!(
            !keys.is_temporary(),
            "a created pair is not a temporary set"
        );
    }

    #[test]
    fn a_session_token_or_an_expiry_makes_a_set_temporary() {
        let token = Credentials::new("ASIATOKEN", "secret").with_session_token("t0k3n");
        assert_eq!(token.session_token(), Some("t0k3n"));
        assert!(token.is_temporary(), "a token is issued");

        let lapsing = Credentials::new("ASIALAPSING", "secret").with_expiry(expiry());
        assert_eq!(lapsing.expires_at(), Some(expiry()));
        assert_eq!(lapsing.session_token(), None);
        assert!(lapsing.is_temporary(), "an expiry is issued");
    }

    #[test]
    fn an_empty_session_token_is_no_token_and_clears_one() {
        let empty = Credentials::new("AKIAEMPTY", "secret").with_session_token("");
        assert_eq!(empty.session_token(), None, "an empty token is none");
        assert!(!empty.is_temporary(), "and does not make the set temporary");
        assert_eq!(
            empty,
            Credentials::new("AKIAEMPTY", "secret"),
            "the set is the pair alone"
        );
        let cleared = Credentials::new("ASIA", "secret")
            .with_session_token("t0k3n")
            .with_session_token("");
        assert_eq!(
            cleared.session_token(),
            None,
            "a later empty token clears it"
        );
    }

    #[test]
    fn an_account_id_is_carried_and_an_empty_one_is_none() {
        let keys = Credentials::new("AKIA", "secret").with_account_id("123456789012");
        assert_eq!(keys.account_id(), Some("123456789012"));
        assert!(
            !keys.is_temporary(),
            "an account does not make a set temporary"
        );
        assert_eq!(
            keys.with_account_id("").account_id(),
            None,
            "an empty account is none, and clears one"
        );
    }

    #[test]
    fn equality_reads_the_secret_and_the_token() {
        assert_eq!(
            Credentials::new("AKIA", "one"),
            Credentials::new("AKIA", "one")
        );
        assert_ne!(
            Credentials::new("AKIA", "one"),
            Credentials::new("AKIA", "two"),
            "a set is its secret too"
        );
        assert_ne!(
            Credentials::new("ASIA", "one").with_session_token("a"),
            Credentials::new("ASIA", "one").with_session_token("b"),
            "and its token"
        );
    }

    #[test]
    fn the_secret_and_the_token_never_reach_debug_output() {
        let keys = Credentials::new("AKIAVISIBLE", "s3cr3t-key")
            .with_session_token("t0k3n-value")
            .with_account_id("123456789012");
        for rendered in [format!("{keys:?}"), format!("{keys:#?}")] {
            assert!(
                rendered.contains("AKIAVISIBLE"),
                "the key id is public: {rendered}"
            );
            assert!(
                rendered.contains("123456789012"),
                "the account is public: {rendered}"
            );
            assert!(
                !rendered.contains("s3cr3t-key"),
                "the secret is not: {rendered}"
            );
            assert!(
                !rendered.contains("t0k3n-value"),
                "the token is not: {rendered}"
            );
            assert!(
                rendered.contains("<redacted>"),
                "each stands redacted: {rendered}"
            );
        }
    }

    #[test]
    fn a_set_lapses_through_its_expiry_stale_within_the_window_and_expired_at_its_instant() {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let hour = Credentials::new("ASIA", "secret")
            .with_session_token("t0k3n")
            .with_expiry(now + Duration::from_secs(3600));
        assert_eq!(
            <Credentials as Expiring>::expires_at(&hour),
            hour.expires_at(),
            "the lease reads the set's own expiry"
        );
        assert!(!lapses_within(&hour, now, WINDOW), "an hour out is fresh");

        let edge = Credentials::new("ASIA", "secret").with_expiry(now + WINDOW);
        assert!(
            lapses_within(&edge, now, WINDOW),
            "fifteen minutes out is stale"
        );
        let past_edge =
            Credentials::new("ASIA", "secret").with_expiry(now + WINDOW + Duration::from_secs(1));
        assert!(
            !lapses_within(&past_edge, now, WINDOW),
            "a second beyond the window is fresh"
        );

        let soon = Credentials::new("ASIA", "secret").with_expiry(now + Duration::from_secs(600));
        assert!(
            lapses_within(&soon, now, WINDOW),
            "ten minutes out is stale"
        );
        assert!(
            !lapses_within(&soon, now, Duration::ZERO),
            "and stale is not expired"
        );
        let at = now + Duration::from_secs(600);
        assert!(
            !lapses_within(&soon, at - Duration::from_secs(1), Duration::ZERO),
            "a second before its instant it stands"
        );
        assert!(
            lapses_within(&soon, at, Duration::ZERO),
            "at its instant it has lapsed"
        );

        let forever = Credentials::new("AKIA", "secret");
        assert_eq!(<Credentials as Expiring>::expires_at(&forever), None);
        assert!(
            !lapses_within(&forever, now, WINDOW),
            "a long-lived pair never lapses"
        );
    }
}

mod document {
    use super::*;

    #[test]
    fn a_metadata_document_carries_its_token_and_expiry_and_ignores_the_rest() {
        let body = br#"{
            "Code": "Success",
            "LastUpdated": "2026-09-05T06:34:56Z",
            "Type": "AWS-HMAC",
            "AccessKeyId": "ASIAMETADATA",
            "SecretAccessKey": "metadata-secret",
            "Token": "metadata-token",
            "Expiration": "2026-09-05T12:34:56Z"
        }"#;
        let keys = parse_document(body, "instance metadata").expect("a metadata document");
        assert_eq!(
            keys,
            Credentials::new("ASIAMETADATA", "metadata-secret")
                .with_session_token("metadata-token")
                .with_expiry(expiry())
        );
        assert_eq!(keys.account_id(), None);
    }

    #[test]
    fn a_process_document_states_version_1_its_session_token_and_its_account() {
        let body = br#"{
            "Version": 1,
            "AccessKeyId": "ASIAPROCESS",
            "SecretAccessKey": "process-secret",
            "SessionToken": "process-token",
            "Expiration": "2026-09-05T12:34:56Z",
            "AccountId": "123456789012"
        }"#;
        assert_eq!(
            parse_document(body, "credential_process").expect("a process document"),
            Credentials::new("ASIAPROCESS", "process-secret")
                .with_session_token("process-token")
                .with_account_id("123456789012")
                .with_expiry(expiry())
        );

        let long_lived = parse_document(
            br#"{"Version":1,"AccessKeyId":"AKIAPROCESS","SecretAccessKey":"process-secret"}"#,
            "credential_process",
        )
        .expect("a long-lived process document");
        assert_eq!(
            long_lived,
            Credentials::new("AKIAPROCESS", "process-secret")
        );
        assert!(
            !long_lived.is_temporary(),
            "no token and no expiry is long-lived"
        );
    }

    #[test]
    fn token_wins_over_session_token_and_an_empty_value_is_none() {
        let both = parse_document(
            br#"{"AccessKeyId":"ASIA","SecretAccessKey":"s","Token":"token","SessionToken":"session-token"}"#,
            "container endpoint",
        )
        .expect("a document with both names");
        assert_eq!(both.session_token(), Some("token"), "Token is read first");

        let empty = parse_document(
            br#"{"AccessKeyId":"ASIA","SecretAccessKey":"s","Token":"","SessionToken":"session-token","AccountId":""}"#,
            "container endpoint",
        )
        .expect("a document with an empty token");
        assert_eq!(
            empty.session_token(),
            Some("session-token"),
            "an empty Token is absent, so SessionToken answers"
        );
        assert_eq!(empty.account_id(), None, "an empty AccountId is absent");
    }

    #[test]
    fn any_version_but_1_is_refused_and_an_absent_one_is_not() {
        for (version, spelled) in [
            ("2", "2"),
            ("\"1\"", "\"1\""),
            ("1.5", "1.5"),
            ("null", "null"),
        ] {
            let body =
                format!(r#"{{"Version":{version},"AccessKeyId":"ASIA","SecretAccessKey":"s"}}"#);
            let message = refusal(parse_document(body.as_bytes(), "credential_process"));
            assert!(message.contains("expected Version 1"), "{message}");
            assert!(
                message.contains("credential_process"),
                "the source is named: {message}"
            );
            assert!(
                message.contains(spelled),
                "the version found is named: {message}"
            );
        }
        assert!(
            parse_document(
                br#"{"AccessKeyId":"ASIA","SecretAccessKey":"s"}"#,
                "instance metadata"
            )
            .is_ok(),
            "the services state no version"
        );
    }

    #[test]
    fn a_document_without_its_pair_is_refused_naming_the_source() {
        for body in [
            &br"{}"[..],
            br#"{"AccessKeyId":"ASIA"}"#,
            br#"{"SecretAccessKey":"s"}"#,
            br#"{"AccessKeyId":"ASIA","SecretAccessKey":""}"#,
            br#"{"AccessKeyId":1,"SecretAccessKey":"s"}"#,
            br"[]",
        ] {
            let message = refusal(parse_document(body, "container endpoint"));
            assert!(message.contains("AccessKeyId"), "{message}");
            assert!(message.contains("SecretAccessKey"), "{message}");
            assert!(
                message.contains("container endpoint"),
                "the source is named: {message}"
            );
        }
    }

    #[test]
    fn a_body_that_is_not_json_is_refused_naming_the_source() {
        for body in [&b"not json"[..], b"", b"{\"AccessKeyId\":"] {
            let message = refusal(parse_document(body, "container endpoint"));
            assert!(
                message.contains("expected a JSON credential document from container endpoint"),
                "{message}"
            );
        }
    }

    #[test]
    fn an_expiration_reads_in_every_spelling_the_tools_write() {
        for spelling in [
            "2026-09-05T12:34:56Z",
            "2026-09-05T14:34:56+02:00",
            "2026-09-05T10:34:56-02:00",
            "2026-09-05T12:34:56UTC",
            "2026-09-05T12:34:56",
        ] {
            let keys = parse_document(&expiring(spelling), "test").expect("a document");
            assert_eq!(keys.expires_at(), Some(expiry()), "{spelling}");
            assert!(
                keys.is_temporary(),
                "an expiry is a temporary set: {spelling}"
            );
        }
        assert_eq!(
            parse_document(&expiring("2026-09-05T12:34:56.250Z"), "test")
                .expect("a document")
                .expires_at(),
            Some(expiry() + Duration::from_millis(250)),
            "a fraction is kept"
        );
        assert_eq!(
            parse_document(&expiring("2026-09-05"), "test")
                .expect("a document")
                .expires_at(),
            Some(UNIX_EPOCH + Duration::from_secs(1_788_566_400)),
            "a bare date lapses at its midnight, UTC"
        );
        assert_eq!(
            parse_document(&expiring(&iso8601(expiry())), "test")
                .expect("a document")
                .expires_at(),
            Some(expiry()),
            "the spelling the crate writes reads back"
        );
    }

    #[test]
    fn an_expiration_nothing_reads_leaves_the_set_long_lived_rather_than_refused() {
        for spelling in ["soon", "", "12:34:56"] {
            let keys = parse_document(&expiring(spelling), "test")
                .expect("an unreadable expiry is not a refusal");
            assert_eq!(keys.expires_at(), None, "{spelling:?}");
            assert_eq!(keys.access_key_id(), "ASIAEXPIRING");
        }
        let numeric = parse_document(
            br#"{"AccessKeyId":"ASIA","SecretAccessKey":"s","Expiration":1788611696}"#,
            "test",
        )
        .expect("a numeric expiry is not a refusal");
        assert_eq!(numeric.expires_at(), None, "an Expiration is text");
    }
}
