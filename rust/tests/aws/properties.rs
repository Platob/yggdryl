//! `rust/src/aws/properties.rs`: the one reader of AWS identity properties,
//! in the names this crate, the AWS tools and PyIceberg each spell them by.

use std::time::{Duration, SystemTime};

use yggdryl::Error;
use yggdryl::aws::{CredentialSource, Session};

use crate::mod_::scratch;

/// A session that reads nothing but what a test states on it.
fn sealed() -> Session {
    Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(scratch("properties"))
        .with_metadata_disabled(true)
}

fn refusal(error: &Error) -> String {
    assert!(
        matches!(error, Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidInput),
        "{error:?}"
    );
    error.to_string()
}

#[test]
fn the_bare_names_the_aws_names_and_the_client_names_are_one_name() {
    for (region, access, secret, token, profile) in [
        (
            "region",
            "access_key_id",
            "secret_access_key",
            "session_token",
            "profile",
        ),
        (
            "AWS_REGION",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_PROFILE",
        ),
        (
            "client.region",
            "client.access-key-id",
            "client.secret-access-key",
            "client.session-token",
            "client.profile-name",
        ),
        (
            "Region",
            "access-key",
            "secret.key",
            "aws.session.token",
            "profile_name",
        ),
    ] {
        let session = sealed()
            .with_properties([
                // What a PyIceberg catalog states beside them is nobody's.
                (
                    "uri",
                    "https://s3tables.ap-southeast-1.amazonaws.com/iceberg",
                ),
                ("rest.sigv4-enabled", "true"),
                ("rest.signing-name", "s3tables"),
                (region, "ap-southeast-1"),
                (access, "AKIAONE"),
                (secret, "one-secret"),
                (token, "one-token"),
                (profile, "trading"),
            ])
            .expect("properties");
        assert_eq!(
            session.region().as_deref(),
            Some("ap-southeast-1"),
            "{region}"
        );
        assert_eq!(session.profile_name(), "trading", "{profile}");
        let keys = session
            .credentials(SystemTime::now())
            .expect("a walk")
            .expect("the stated set");
        assert_eq!(keys.access_key_id(), "AKIAONE", "{access}");
        assert_eq!(keys.session_token(), Some("one-token"), "{token}");
        assert_eq!(session.credential_source(), Some("explicit credentials"));
        for name in [region, access, secret, token, profile] {
            assert!(Session::is_property(name), "{name}");
        }
    }
}

#[test]
fn a_catalogs_bearer_token_and_the_stores_own_names_are_not_read_here() {
    // `token` authorizes a catalog, `s3.*` is the object store's reader's,
    // and a bag carries a great deal that is nobody's identity.
    let session = sealed()
        .with_properties([
            ("token", "a catalog's bearer token"),
            ("bearer_token", "a store's"),
            ("s3.region", "us-west-2"),
            ("s3.access-key-id", "AKIASTORE"),
            ("s3.secret-access-key", "store-secret"),
            ("endpoint_url", "http://localhost:9000"),
            ("warehouse", "s3://lake"),
            ("regoin", "eu-west-3"),
            ("region", "   "),
        ])
        .expect("unknown names are ignored, and an empty value states nothing");
    assert_eq!(session.region(), None);
    assert_eq!(
        session.endpoint_url("s3").expect("a readable endpoint"),
        None
    );
    assert_eq!(
        session.credentials(SystemTime::now()).expect("a walk"),
        None
    );
    for name in [
        "token",
        "bearer_token",
        "s3.region",
        "s3.access-key-id",
        "endpoint_url",
        "warehouse",
        "regoin",
        "catalog.token",
    ] {
        assert!(!Session::is_property(name), "{name}");
    }
    for name in Session::PROPERTY_NAMES {
        assert!(Session::is_property(name), "{name}");
    }
}

#[test]
fn half_a_credential_set_is_refused_naming_the_half_that_is_missing() {
    for (name, missing) in [
        ("client.access-key-id", "the secret_access_key is missing"),
        ("aws_secret_access_key", "the access_key_id is missing"),
        (
            "session_token",
            "the access_key_id and the secret_access_key are missing",
        ),
    ] {
        let error = sealed().with_properties([(name, "half")]).expect_err(name);
        assert!(refusal(&error).contains(missing), "{name}: {error}");
    }
}

#[test]
fn anonymous_states_that_nothing_signs_and_a_value_that_is_no_boolean_is_refused() {
    let session = sealed()
        .with_properties([("no_sign_request", "true")])
        .expect("properties");
    assert!(session.anonymous());
    assert_eq!(
        session.credentials(SystemTime::now()).expect("a walk"),
        None
    );

    let error = sealed()
        .with_properties([("AWS_USE_FIPS_ENDPOINT", "perhaps")])
        .expect_err("not a boolean");
    let message = refusal(&error);
    assert!(
        message.contains("true/false") && message.contains("AWS_USE_FIPS_ENDPOINT"),
        "{message}"
    );
}

#[test]
fn a_role_and_the_switches_are_assembled_as_the_object_store_reader_assembles_them() {
    let session = sealed()
        .with_properties([
            ("client.role-arn", "arn:aws:iam::123456789012:role/reader"),
            ("role_session_name", "audit"),
            ("external_id", "partner"),
            ("role_duration", "1800"),
            ("sts_region", "eu-west-1"),
            ("mfa_serial", "arn:aws:iam::123456789012:mfa/dev"),
            ("credential_source", "Environment"),
            ("use_fips_endpoint", "yes"),
            ("use_dualstack_endpoint", "0"),
            ("sts_regional_endpoints", "legacy"),
            ("ec2_metadata_disabled", "true"),
            ("metadata_service_timeout", "2.5"),
        ])
        .expect("properties");
    let role = session.assumed_role().expect("a role");
    assert_eq!(role.role_arn(), "arn:aws:iam::123456789012:role/reader");
    assert_eq!(role.session_name(), Some("audit"));
    assert_eq!(role.external_id(), Some("partner"));
    assert_eq!(role.duration(), Duration::from_secs(1800));
    assert_eq!(role.region(), Some("eu-west-1"));
    assert_eq!(
        role.credential_source(),
        Some(CredentialSource::Environment)
    );
    assert!(session.use_fips_endpoint());
    assert!(!session.use_dualstack_endpoint());
    assert!(!session.sts_regional_endpoints());

    // An STS endpoint stated without a role still says where STS is.
    let session = sealed()
        .with_properties([("sts_endpoint", "http://localhost:4566/")])
        .expect("properties");
    assert_eq!(
        session.sts_endpoint("eu-west-3").expect("an STS endpoint"),
        "http://localhost:4566"
    );
}

#[test]
fn a_sign_in_is_four_values_or_none_and_names_what_is_missing() {
    let session = sealed()
        .with_properties([
            ("sso_start_url", "https://corp.awsapps.com/start"),
            ("sso_region", "eu-west-1"),
            ("sso_account_id", "123456789012"),
            ("sso_role_name", "Reader"),
            ("sso_session", "corp"),
        ])
        .expect("properties");
    let sso = session.sso().expect("a sign-in");
    assert_eq!(sso.region(), "eu-west-1");

    let error = sealed()
        .with_properties([
            ("sso_start_url", "https://corp.awsapps.com/start"),
            ("sso_region", "eu-west-1"),
        ])
        .expect_err("half a sign-in");
    assert!(
        refusal(&error).contains("needs sso_account_id, sso_role_name"),
        "{error}"
    );
}

#[test]
fn properties_are_stated_over_the_session_and_a_later_pair_replaces_an_earlier_one() {
    let base = sealed().with_region("us-east-1").with_profile("base");
    let session = base
        .with_properties([("region", "eu-west-3"), ("AWS_REGION", "eu-central-1")])
        .expect("properties");
    assert_eq!(session.region().as_deref(), Some("eu-central-1"));
    assert_eq!(
        session.profile_name(),
        "base",
        "what the bag does not name stands"
    );
    assert_eq!(
        base.region().as_deref(),
        Some("us-east-1"),
        "the session it came from is untouched"
    );

    let fresh = Session::from_properties([("profile", "trading")]).expect("properties");
    assert_eq!(fresh.profile_name(), "trading");
    assert!(
        fresh.reads_environment(),
        "from_properties starts from Session::new"
    );
}
