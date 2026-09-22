//! `rust/src/object/aws/credentials.rs`: the credential chain's own readings.
//!
//! Walking the chain reaches the environment, two shared files and two
//! metadata services, none of which a test may touch. What it turns into a
//! credential set - one metadata document, one instant, one staleness rule,
//! and a `Debug` that never renders a secret - is what is pinned here.

use std::time::{Duration, SystemTime};

use yggdryl::internals::object_aws_credentials::{
    days_from_civil, is_stale, parse_iso8601_utc, parse_metadata_credentials,
};
use yggdryl::object::Credentials;

#[test]
fn the_secret_never_reaches_debug_output() {
    let keys = Credentials::new("AKIA", "s3cr3t").with_session_token("t0k3n");
    let rendered = format!("{keys:?}");
    assert!(rendered.contains("AKIA"), "{rendered}");
    assert!(!rendered.contains("s3cr3t"), "{rendered}");
    assert!(!rendered.contains("t0k3n"), "{rendered}");
}

#[test]
fn an_empty_session_token_is_no_token() {
    assert_eq!(
        Credentials::new("a", "b")
            .with_session_token("")
            .session_token(),
        None
    );
}

#[test]
fn a_set_is_stale_inside_the_refresh_margin_only() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let fresh = Credentials::new("a", "b").with_expiry(now + Duration::from_secs(3600));
    assert!(!is_stale(&fresh, now));
    let expiring = Credentials::new("a", "b").with_expiry(now + Duration::from_secs(60));
    assert!(is_stale(&expiring, now));
    assert!(!is_stale(&Credentials::new("a", "b"), now));
}

#[test]
fn metadata_documents_carry_token_and_expiry() {
    let body = br#"{"Code":"Success","AccessKeyId":"ASIA","SecretAccessKey":"secret","Token":"tok","Expiration":"2026-09-05T12:34:56Z"}"#;
    let keys = parse_metadata_credentials(body, "test").unwrap();
    assert_eq!(keys.access_key_id(), "ASIA");
    assert_eq!(keys.session_token(), Some("tok"));
    assert_eq!(keys.expires_at(), parse_iso8601_utc("2026-09-05T12:34:56Z"));

    let message = parse_metadata_credentials(b"{}", "test")
        .unwrap_err()
        .to_string();
    assert!(message.contains("AccessKeyId"), "{message}");
}

#[test]
fn iso8601_utc_instants_parse_and_the_rest_does_not() {
    let epoch = parse_iso8601_utc("1970-01-01T00:00:00Z").unwrap();
    assert_eq!(epoch, SystemTime::UNIX_EPOCH);
    let later = parse_iso8601_utc("2013-05-24T00:00:00Z").unwrap();
    assert_eq!(
        later.duration_since(SystemTime::UNIX_EPOCH).unwrap(),
        Duration::from_secs(1_369_353_600)
    );
    assert_eq!(
        parse_iso8601_utc("2013-05-24T00:00:00.123Z"),
        parse_iso8601_utc("2013-05-24T00:00:00Z")
    );
    assert_eq!(parse_iso8601_utc("2013-05-24T00:00:00+02:00"), None);
    assert_eq!(parse_iso8601_utc("2013-05-24"), None);
    assert_eq!(parse_iso8601_utc("2013-13-24T00:00:00Z"), None);
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    assert_eq!(days_from_civil(1969, 12, 31), -1);
}

mod protocol {
    use yggdryl::IOBase;
    use yggdryl::object::S3Options;

    use crate::mod_::{BUCKET, file_with, store};

    #[test]
    fn an_anonymous_client_signs_nothing() {
        let store = store();
        store.allow_anonymous(true);
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let handle = file_with(
            "lake/part.parquet",
            S3Options::default()
                .with_environment(false)
                .with_endpoint(store.endpoint())
                .with_path_style(true)
                .with_anonymous(true),
        );

        assert_eq!(handle.read_all_bytes().expect("a public object"), b"PAR1");
        let recorded = store.requests();
        assert!(
            !recorded
                .last()
                .expect("the read")
                .headers
                .iter()
                .any(|(name, _)| name == "authorization"),
            "an anonymous request carries no authorization"
        );
    }

    #[test]
    fn credentials_written_into_a_location_are_used_and_then_never_rendered() {
        let store = store();
        store.require_access_key(Some("AKIAINURL"));
        let mut handle = yggdryl::object::file_with(
            &format!("s3://AKIAINURL:s3cr3t@{BUCKET}/lake/part.parquet"),
            S3Options::default()
                .with_environment(false)
                .with_endpoint(store.endpoint())
                .with_region("us-east-1")
                .with_path_style(true),
        )
        .expect("a handle");
        handle.write_all_bytes(b"PAR1").expect("a signed write");

        // The keys signed the request, and the handle's own location has none.
        let rendered = handle.url().to_string();
        assert_eq!(rendered, "s3://trades/lake/part.parquet");
        assert!(!rendered.contains("s3cr3t"));
        assert!(!format!("{handle:?}").contains("s3cr3t"));
    }
}
