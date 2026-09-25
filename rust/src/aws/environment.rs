//! The variables the AWS tools read for themselves.
//!
//! A [`Session`](super::Session) reads these with the precedence those tools
//! give them: a profile `AWS_PROFILE` names does not displace the
//! environment's own keys the way an explicit one does, and
//! `AWS_ENDPOINT_URL_S3` beats `AWS_ENDPOINT_URL`. The S3 backend sweeps the
//! environment for its own knobs under `AWS_` and leaves these to the
//! session.

/// Whether `name` is a variable the session reads for itself.
pub(crate) fn is_native(name: &str) -> bool {
    NATIVE.contains(&name) || name.starts_with("AWS_ENDPOINT_URL_")
}

const NATIVE: [&str; 37] = [
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
    "AWS_REQUEST_CHECKSUM_CALCULATION",
    "AWS_RESPONSE_CHECKSUM_VALIDATION",
];

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/environment.rs` pins and a caller cannot reach.

    /// Whether `name` is a variable the session reads for itself.
    pub fn is_native(name: &str) -> bool {
        super::is_native(name)
    }
}
