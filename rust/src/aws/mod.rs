//! Who this process is to Amazon Web Services, and where AWS is.
//!
//! Every AWS request signs with a credential set, is scoped to a region, and
//! reaches an endpoint. The AWS tools find all three in the same places, in
//! the same order - what the caller said, the process environment, the shared
//! files under `~/.aws`, the container and instance metadata services - and
//! this module is that resolution written once, for every consumer the crate
//! has: the S3 backend today, whatever signs an AWS request tomorrow.
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
pub(crate) mod metadata;
pub(crate) mod process;
pub(crate) mod profile;
pub(crate) mod session;
pub(crate) mod sigv4;
pub(crate) mod sso;
pub(crate) mod sts;

pub use credentials::Credentials;
pub use profile::Profile;
pub use session::{MfaPrompt, Session};
pub use sso::{DeviceAuthorization, Sso, SsoLogin};
pub use sts::{AssumedRole, CredentialSource};

/// The lowercase hex SHA-1 of `text`, which is how the AWS tools name every
/// file in the caches this module shares with them.
pub(crate) fn sha1_hex(text: &str) -> String {
    use std::fmt::Write as _;
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, text.as_bytes());
    let mut hex = String::with_capacity(40);
    for byte in digest.as_ref() {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
