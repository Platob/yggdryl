//! Reading a property map written in somebody else's vocabulary.
//!
//! A caller reaching S3 through this crate starts from a PyIceberg catalog's
//! properties, from PyArrow's `S3FileSystem` arguments, or from the AWS
//! environment variable names - three vocabularies for one set of knobs. What
//! these check is that each of the three lands on the same knob, that a value
//! that will not parse is heard here rather than at the store, and that a
//! knob this client cannot honor is refused rather than dropped.

use std::time::Duration;

use crate::holder::s3::{AssumedRole, Encryption, S3Options};

#[test]
fn a_pyiceberg_catalogs_properties_reach_every_knob_they_name() {
    let options = S3Options::from_properties([
        // Most of a catalog's properties have nothing to do with a store.
        ("type", "rest"),
        ("uri", "https://catalog.example.io"),
        ("warehouse", "s3://trades/lake"),
        ("token", "a-catalog-bearer-token-that-is-not-a-session"),
        // These do.
        ("s3.endpoint", "http://localhost:9000"),
        ("s3.access-key-id", "minioadmin"),
        ("s3.secret-access-key", "minioadmin-secret"),
        ("s3.session-token", "a-session-token"),
        ("s3.region", "eu-west-1"),
        ("s3.force-virtual-addressing", "false"),
        ("s3.connect-timeout", "2.5"),
        ("s3.request-timeout", "30"),
        ("s3.proxy-uri", "http://proxy.internal:3128"),
    ])
    .expect("readable properties");

    assert_eq!(options.endpoint(), Some("http://localhost:9000"));
    assert_eq!(options.region(), Some("eu-west-1"));
    assert_eq!(options.path_style(), Some(true));
    assert_eq!(options.connect_timeout(), Duration::from_millis(2500));
    assert_eq!(options.timeout(), Duration::from_secs(30));
    assert_eq!(options.proxy(), Some("http://proxy.internal:3128"));
    let credentials = options.credentials().expect("a credential set");
    assert_eq!(credentials.access_key_id(), "minioadmin");
    assert_eq!(credentials.session_token(), Some("a-session-token"));
}

#[test]
fn pyarrows_arguments_reach_the_same_knobs_by_their_own_names() {
    let options = S3Options::from_properties([
        ("endpoint_override", "localhost:9000"),
        ("scheme", "http"),
        ("access_key", "minioadmin"),
        ("secret_key", "minioadmin-secret"),
        ("region", "eu-west-1"),
        ("force_virtual_addressing", "true"),
        ("request_timeout", "15"),
        ("connect_timeout", "1"),
        ("allow_bucket_creation", "false"),
        ("allow_bucket_deletion", "false"),
        // Ours, and nothing PyArrow ever passes; it is simply ignored.
        ("background_writes", "true"),
    ])
    .expect("readable properties");

    // A scheme and a bare endpoint are one endpoint.
    assert_eq!(options.endpoint(), Some("http://localhost:9000"));
    assert_eq!(options.path_style(), Some(false), "virtual is not path");
    assert_eq!(options.timeout(), Duration::from_secs(15));
    assert!(!options.bucket_creation());
    assert!(!options.bucket_deletion());
}

#[test]
fn the_aws_environment_names_are_the_same_knobs_again() {
    let options = S3Options::from_properties([
        ("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE"),
        ("AWS_SECRET_ACCESS_KEY", "wJalrXUtnFEMI"),
        ("AWS_REGION", "us-east-1"),
        ("AWS_ENDPOINT_URL", "https://s3.example.io"),
        ("AWS_S3_FORCE_PATH_STYLE", "true"),
        ("AWS_PROFILE", "trading"),
    ])
    .expect("readable properties");

    assert_eq!(options.region(), Some("us-east-1"));
    assert_eq!(options.endpoint(), Some("https://s3.example.io"));
    assert_eq!(options.path_style(), Some(true), "two prefixes are peeled");
    assert_eq!(options.profile(), Some("trading"));
    assert_eq!(
        options.credentials().map(|keys| keys.access_key_id()),
        Some("AKIAIOSFODNN7EXAMPLE")
    );
}

#[test]
fn a_catalogs_bearer_token_is_never_read_as_a_session_token() {
    // `token` is a REST catalog's OAuth token, and reading it as an AWS
    // session token would sign every request with a credential set that is
    // half right - which fails at the store, far from the cause.
    let options = S3Options::from_properties([
        ("token", "an-oauth-bearer-token"),
        ("s3.access-key-id", "minioadmin"),
        ("s3.secret-access-key", "minioadmin-secret"),
    ])
    .expect("readable properties");
    assert_eq!(
        options.credentials().and_then(|keys| keys.session_token()),
        None
    );
}

#[test]
fn a_role_is_assembled_from_the_properties_that_describe_it() {
    let options = S3Options::from_properties([
        ("s3.role-arn", "arn:aws:iam::123456789012:role/lake-reader"),
        ("s3.role-session-name", "power-desk"),
        ("external_id", "desk-42"),
        ("role_duration", "7200"),
        ("sts_endpoint", "https://sts.eu-west-1.amazonaws.com"),
    ])
    .expect("readable properties");

    let role = options.assumed_role().expect("a role");
    assert_eq!(
        role.role_arn(),
        "arn:aws:iam::123456789012:role/lake-reader"
    );
    assert_eq!(role.session_name(), "power-desk");
    assert_eq!(role.external_id(), Some("desk-42"));
    assert_eq!(role.duration(), Duration::from_secs(7200));
    assert_eq!(role.endpoint(), Some("https://sts.eu-west-1.amazonaws.com"));

    // A duration outside what STS issues is clamped rather than refused.
    let options = S3Options::from_properties([("role_arn", "arn:x"), ("role_duration", "60")])
        .expect("readable properties");
    assert_eq!(
        options.assumed_role().map(AssumedRole::duration),
        Some(Duration::from_secs(900))
    );
}

#[test]
fn the_sse_properties_name_each_of_the_three_kinds() {
    let managed = S3Options::from_properties([("s3.sse.type", "AES256")]).expect("properties");
    assert!(matches!(managed.encryption(), Encryption::Managed));

    let kms = S3Options::from_properties([
        ("s3.sse.type", "aws:kms"),
        ("s3.sse.key", "arn:aws:kms:eu-west-1:1234:key/abcd"),
        ("sse.context", r#"{"desk":"power"}"#),
        ("sse.bucket-key-enabled", "true"),
    ])
    .expect("properties");
    let Encryption::Kms(key) = kms.encryption() else {
        panic!("expected SSE-KMS, got {:?}", kms.encryption());
    };
    assert_eq!(key.key_id(), Some("arn:aws:kms:eu-west-1:1234:key/abcd"));
    assert_eq!(key.context(), Some(r#"{"desk":"power"}"#));
    assert_eq!(key.bucket_key(), Some(true));

    // `AES256` is what SSE-S3 is called and what a customer key's algorithm
    // is called; a key alongside it is what tells the two apart.
    let customer = S3Options::from_properties([
        ("s3.sse.type", "AES256"),
        ("s3.sse.key", "AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dw="),
        ("s3.sse.md5", "N+iD0sgzzGlyEXJzCNck5w=="),
    ])
    .expect("properties");
    assert!(matches!(customer.encryption(), Encryption::Customer(_)));

    // And nothing said is nothing done.
    assert!(
        S3Options::from_properties([("s3.sse.type", "none")])
            .expect("properties")
            .encryption()
            .is_default()
    );
}

#[test]
fn a_value_that_will_not_parse_is_heard_here_rather_than_at_the_store() {
    for (name, value, expected) in [
        ("s3.request-timeout", "soon", "seconds"),
        ("anonymous", "perhaps", "boolean"),
        ("max_attempts", "many", "whole number"),
        ("part_size", "big", "byte count"),
        ("s3.sse.type", "rot13", "sse type"),
    ] {
        let refused = S3Options::from_properties([(name, value)]).expect_err("a refusal");
        assert!(
            refused.to_string().contains(expected),
            "{name}={value}: {refused}"
        );
    }

    // A customer key whose MD5 does not match it is the same story.
    let refused = S3Options::from_properties([
        ("s3.sse.type", "sse-c"),
        ("s3.sse.key", "AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dw="),
        ("s3.sse.md5", "AAAAAAAAAAAAAAAAAAAAAA=="),
    ])
    .expect_err("a refusal");
    assert!(refused.to_string().contains("MD5"), "{refused}");
}

#[test]
fn a_knob_this_client_cannot_honor_is_refused_rather_than_dropped() {
    let refused = S3Options::from_properties([("s3.signer.uri", "https://signer.example.io")])
        .expect_err("a refusal");
    assert!(refused.to_string().contains("does not do"), "{refused}");

    // Whereas a knob about something that is not this store is simply not
    // this store's business.
    let options = S3Options::from_properties([
        ("adls.account-name", "trades"),
        ("gcs.project-id", "trades"),
        ("py-io-impl", "pyiceberg.io.pyarrow.PyArrowFileIO"),
    ])
    .expect("readable properties");
    assert_eq!(options.endpoint(), None);
    assert_eq!(options.region(), None);
}

#[test]
fn sizes_may_carry_the_unit_a_configuration_file_writes_them_with() {
    let options = S3Options::from_properties([
        ("part_size", "8MiB"),
        ("multipart_threshold", "32 MB"),
        ("list_page_size", "500"),
        ("num_retries", "4"),
    ])
    .expect("readable properties");

    assert_eq!(options.part_size(), 8 * 1024 * 1024);
    assert_eq!(options.multipart_threshold(), 32 * 1024 * 1024);
    assert_eq!(options.list_page_size(), 500);
    assert_eq!(options.max_attempts(), 5, "four retries is five attempts");
}
