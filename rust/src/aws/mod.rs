//! Who this process is to Amazon Web Services, and where AWS is.
//!
//! Every AWS request signs with a credential set, is scoped to a region, and
//! reaches an endpoint. The AWS tools find all three in the same places, in
//! the same order - what the caller said, the process environment, the shared
//! files under `~/.aws`, the container and instance metadata services - and
//! this module is that resolution written once, for every consumer the crate
//! has: the S3 backend, and any request the HTTP client sends, which
//! [`Request::with_sigv4`](crate::http::Request::with_sigv4) signs for the
//! service it names.
//!
//! [`Session`] is the door. It carries what a caller states explicitly and
//! resolves the rest lazily, once, on the first request that needs it:
//!
//! ```
//! use yggdryl::aws::Session;
//!
//! // Nothing is read until a credential set, a region or an endpoint is asked
//! // for, and everything read is cached on the session.
//! let session = Session::new().with_profile("trading").with_region("eu-west-3");
//! assert_eq!(session.profile_name(), "trading");
//! assert_eq!(session.region().as_deref(), Some("eu-west-3"));
//! ```
//!
//! # Coverage
//!
//! The credential chain walks the sources botocore walks, in botocore's
//! order: an explicit set, the environment (`AWS_ACCESS_KEY_ID` and its
//! companions, with `AWS_CREDENTIAL_EXPIRATION`), a profile that assumes a
//! role - through `source_profile`, `credential_source` or a web identity
//! token - IAM Identity Center (SSO) through its cached sign-in, the shared
//! credentials file, a `credential_process`, the configuration file, the
//! legacy boto files, the container credential endpoint, and the instance
//! metadata service. Region and endpoint resolve through `AWS_REGION`,
//! `AWS_ENDPOINT_URL`, `AWS_ENDPOINT_URL_<SERVICE>`, the profile's own
//! `region`, `endpoint_url` and `[services]` section, and the FIPS, dual-stack
//! and STS endpoint switches the files name.
//!
//! # Best effort, then a named refusal
//!
//! Where botocore fails on the first source that is configured and broken -
//! a profile nobody wrote, an SSO sign-in that lapsed, a `credential_process`
//! that exited non-zero - the chain here records why and walks on, so a
//! process whose instance has a role still signs when its operator's laptop
//! profile does not apply to it. Only when every source has been asked does
//! it refuse, and the refusal names each source and what it said. A
//! temporary set is replaced by the request that finds it near its expiry,
//! and a refresh that fails keeps the set in hand until that set has
//! actually lapsed rather than failing a request signed with a still valid
//! one.
//!
//! What is not AWS's own - a secret that never renders, a value that lapses
//! and is obtained again before it does, an environment or a stand-in for
//! one, the bookkeeping of a walk - is the crate's `auth` module, shared with
//! the Google and Azure dialects of the S3 backend.
//!
//! The module is behind the non-default `aws` feature, which the `s3` feature
//! implies.

pub(crate) mod container;
pub(crate) mod credentials;
#[cfg(feature = "s3")]
pub(crate) mod environment;
pub(crate) mod login;
pub(crate) mod metadata;
pub(crate) mod process;
pub(crate) mod profile;
pub(crate) mod properties;
pub(crate) mod request;
pub(crate) mod session;
pub(crate) mod sigv4;
pub(crate) mod sso;
pub(crate) mod sts;

pub use credentials::Credentials;
pub use profile::Profile;
pub use session::{MfaPrompt, Session};
pub use sso::{DeviceAuthorization, Sso, SsoLogin};
pub use sts::{AssumedRole, CredentialSource};

/// One answer an identity service gave, read whole: its status, the error
/// type it names in `x-amzn-ErrorType`, and its body. Every identity call
/// goes out through the session's [`crate::http::Session`], so retries,
/// timeouts, proxies and the CA bundle are the HTTP client's.
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) error_type: Option<String>,
    pub(crate) body: std::sync::Arc<[u8]>,
}

impl Answer {
    /// Send `request` and read its answer whole.
    ///
    /// # Errors
    ///
    /// The HTTP client's: a transport failure no retry could mend
    /// ([`crate::http::is_unanswered`] tells one where nothing answered),
    /// or a body past the session's bound.
    pub(crate) fn of(request: &crate::http::Request) -> crate::Result<Self> {
        let response = request.send()?;
        let error_type = error_type(response.headers()).map(str::to_owned);
        Ok(Self {
            status: response.status().code(),
            error_type,
            body: response.bytes()?,
        })
    }
}

/// The error type an answer names in `x-amzn-ErrorType`: the header's text
/// before any `:` a service appends its documentation URL after.
fn error_type(headers: &crate::http::Headers) -> Option<&str> {
    headers
        .get("x-amzn-errortype")
        .and_then(|value| value.split(':').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The error code a refusing answer names, wherever the AWS protocols let a
/// service state one: the `x-amzn-ErrorType` header, the `__type` or `code`
/// of a JSON body - a `__type` read past its `#`, which a namespace comes
/// before - or the `<Code>` of an XML `<Error>`.
pub(crate) fn error_code(headers: &crate::http::Headers, body: &[u8]) -> Option<String> {
    if let Some(named) = error_type(headers) {
        return Some(named.to_owned());
    }
    match body.iter().find(|byte| !byte.is_ascii_whitespace())? {
        b'{' => {
            let document: serde_json::Value = serde_json::from_slice(body).ok()?;
            ["__type", "code", "Code"]
                .iter()
                .find_map(|member| document.get(member)?.as_str())
                .map(|named| named.rsplit('#').next().unwrap_or(named).to_owned())
                .filter(|named| !named.is_empty())
        }
        b'<' => sts::parse_error(body)
            .map(|(code, _message)| code)
            .filter(|code| !code.is_empty()),
        _ => None,
    }
}

/// The lowercase hex SHA-1 of `text`, which is how the AWS tools name every
/// file in the caches this module shares with them.
pub(crate) fn sha1_hex(text: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, text.as_bytes());
    crate::bytes::hex_text(digest.as_ref())
}
