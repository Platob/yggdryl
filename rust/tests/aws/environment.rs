//! `rust/src/aws/environment.rs`: the variables a session reads for itself,
//! and how a session reads a variable at all.
//!
//! A variable is read trimmed, an empty or blank one is unset, the pairs a
//! session was handed replace the process environment whole, and a session
//! told to consult no environment reads no variable, not even one it was
//! handed. Nothing here sets a variable: the process's own environment is
//! only read, through a session built with `Session::new`, and a session that
//! resolves a region or a profile keeps its shared files in a directory of the
//! test's own so no machine's `~/.aws` is read.

use yggdryl::aws::Session;

use crate::mod_::scratch;

#[test]
fn a_variable_is_read_trimmed_and_an_empty_or_blank_one_is_unset() {
    let session = Session::new()
        .with_variables([
            ("AWS_REGION", "  eu-west-1 \n"),
            ("AWS_PROFILE", ""),
            ("AWS_DEFAULT_PROFILE", " \t "),
        ])
        .with_directory(scratch("environment-trimmed"));

    assert_eq!(session.variable("AWS_REGION").as_deref(), Some("eu-west-1"));
    assert_eq!(session.variable("AWS_PROFILE"), None, "empty is unset");
    assert_eq!(
        session.variable("AWS_DEFAULT_PROFILE"),
        None,
        "blank is unset"
    );
    assert_eq!(
        session.profile_name(),
        "default",
        "neither unset profile variable names a profile"
    );
    assert_eq!(session.region().as_deref(), Some("eu-west-1"));

    let blank_region = Session::new()
        .with_variables([
            ("AWS_REGION", "   "),
            ("AWS_DEFAULT_REGION", "\tap-south-1\t"),
        ])
        .with_directory(scratch("environment-blank-region"));
    assert_eq!(
        blank_region.region().as_deref(),
        Some("ap-south-1"),
        "a blank AWS_REGION gives way to AWS_DEFAULT_REGION"
    );
}

#[test]
fn the_pairs_a_session_is_handed_replace_the_process_environment_whole() {
    let empty = Session::new().with_variables::<&str, &str>([]);
    assert!(empty.reads_environment());
    assert_eq!(
        empty.variable("PATH"),
        None,
        "the process's PATH is not read through an environment handed over"
    );
    assert_eq!(empty.variable("HOME"), None);

    let replaced = Session::new()
        .with_variables([("AWS_REGION", "eu-west-1")])
        .with_variables([("AWS_PROFILE", "trading")]);
    assert_eq!(
        replaced.variable("AWS_REGION"),
        None,
        "a second set of pairs replaces the first rather than adding to it"
    );
    assert_eq!(replaced.variable("AWS_PROFILE").as_deref(), Some("trading"));

    let repeated =
        Session::new().with_variables([("AWS_PROFILE", "first"), ("AWS_PROFILE", "last")]);
    assert_eq!(
        repeated.variable("AWS_PROFILE").as_deref(),
        Some("last"),
        "of two pairs naming one variable, the later one is read"
    );

    let exact = Session::new().with_variables([("aws_profile", "lowercase")]);
    assert_eq!(
        exact.variable("AWS_PROFILE"),
        None,
        "a handed-over name is read exactly as spelled"
    );
}

#[test]
fn a_session_that_consults_no_environment_reads_no_variable_even_one_it_was_handed() {
    let session = Session::new()
        .with_variables([
            ("AWS_REGION", "eu-west-1"),
            ("AWS_PROFILE", "trading"),
            ("AWS_ENDPOINT_URL", "http://localhost:9000"),
            ("AWS_USE_FIPS_ENDPOINT", "true"),
        ])
        .with_directory(scratch("environment-off"))
        .with_environment(false);

    assert!(!session.reads_environment());
    for name in [
        "AWS_REGION",
        "AWS_PROFILE",
        "AWS_ENDPOINT_URL",
        "AWS_USE_FIPS_ENDPOINT",
        "PATH",
    ] {
        assert_eq!(session.variable(name), None, "{name} is not read");
    }
    assert_eq!(session.profile_name(), "default");
    assert_eq!(session.region(), None);
    assert_eq!(session.endpoint_url("s3"), None);
    assert!(!session.use_fips_endpoint());

    let consulting = session.with_environment(true);
    assert!(consulting.reads_environment());
    assert_eq!(
        consulting.variable("AWS_REGION").as_deref(),
        Some("eu-west-1"),
        "turned back on, the pairs it was handed are read again"
    );
    assert_eq!(
        Session::new().with_environment(false).variable("PATH"),
        None,
        "nor is the process environment read"
    );
}

#[test]
fn a_session_over_the_process_environment_reads_it() {
    let session = Session::new();
    assert!(session.reads_environment());
    assert!(
        session.variable("PATH").is_some(),
        "every process a test runs in has a PATH"
    );
    assert_eq!(
        session.variable("YGGDRYL_AWS_SURELY_UNSET_VARIABLE_7C1E"),
        None
    );
}

#[test]
fn a_boolean_variable_is_true_in_the_spellings_the_tools_accept_and_false_otherwise() {
    for value in ["true", "TRUE", " True ", "1", "yes", "on"] {
        let session = Session::new()
            .with_variables([("AWS_USE_FIPS_ENDPOINT", value)])
            .with_directory(scratch("environment-flag-true"));
        assert!(session.use_fips_endpoint(), "{value:?} spells true");
    }
    for value in ["false", "0", "no", "off", "maybe", ""] {
        let session = Session::new()
            .with_variables([("AWS_USE_FIPS_ENDPOINT", value)])
            .with_directory(scratch("environment-flag-false"));
        assert!(
            !session.use_fips_endpoint(),
            "{value:?} does not spell true"
        );
    }
}

#[cfg(all(feature = "internals", feature = "s3"))]
mod internal {
    use yggdryl::internals::aws_environment::is_native;

    /// Every variable the AWS tools read for themselves, which the S3
    /// backend's sweep leaves to the session.
    const READ_BY_THE_SESSION: [&str; 36] = [
        "AWS_PROFILE",
        "AWS_DEFAULT_PROFILE",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_SECURITY_TOKEN",
        "AWS_CREDENTIAL_EXPIRATION",
        "AWS_ACCOUNT_ID",
        "AWS_CREDENTIAL_FILE",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
        "AWS_CONFIG_FILE",
        "AWS_SHARED_CREDENTIALS_FILE",
        "AWS_ENDPOINT_URL",
        "AWS_IGNORE_CONFIGURED_ENDPOINT_URLS",
        "AWS_ROLE_ARN",
        "AWS_ROLE_SESSION_NAME",
        "AWS_WEB_IDENTITY_TOKEN_FILE",
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI",
        "AWS_CONTAINER_AUTHORIZATION_TOKEN",
        "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
        "AWS_EC2_METADATA_DISABLED",
        "AWS_EC2_METADATA_SERVICE_ENDPOINT",
        "AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE",
        "AWS_IMDS_USE_IPV6",
        "AWS_EC2_METADATA_V1_DISABLED",
        "AWS_METADATA_SERVICE_TIMEOUT",
        "AWS_METADATA_SERVICE_NUM_ATTEMPTS",
        "AWS_STS_REGIONAL_ENDPOINTS",
        "AWS_USE_FIPS_ENDPOINT",
        "AWS_USE_DUALSTACK_ENDPOINT",
        "AWS_CA_BUNDLE",
        "AWS_MAX_ATTEMPTS",
        "AWS_RETRY_MODE",
        "AWS_ENDPOINT_URL_S3",
    ];

    #[test]
    fn every_variable_the_session_reads_is_left_to_it() {
        for name in READ_BY_THE_SESSION {
            assert!(is_native(name), "{name} is the session's to read");
        }
    }

    #[test]
    fn every_service_s_endpoint_variable_is_the_session_s() {
        for name in [
            "AWS_ENDPOINT_URL_S3",
            "AWS_ENDPOINT_URL_STS",
            "AWS_ENDPOINT_URL_SSO_OIDC",
            "AWS_ENDPOINT_URL_DYNAMODB",
        ] {
            assert!(is_native(name), "{name} names one service's endpoint");
        }
    }

    #[test]
    fn the_crate_s_own_knobs_and_foreign_variables_are_not_the_session_s() {
        for name in [
            "AWS_SSE_TYPE",
            "AWS_REQUESTER_PAYS",
            "YGGDRYL_ROLE_ARN",
            "AWS_ENDPOINT_URLX",
            "PATH",
        ] {
            assert!(
                !is_native(name),
                "{name} is not a variable the session reads"
            );
        }
    }
}
