//! Trading a role for a credential set, through AWS STS.
//!
//! A process that reaches a bucket in another account, or under a policy of
//! its own, does not hold keys for it: it holds keys that are allowed to
//! *assume a role* that does. One signed request trades the first for the
//! second, and the answer expires, so it is asked for again shortly before it
//! does rather than per request.

use std::time::{Duration, SystemTime};

use super::credentials::Credentials;
use super::sign::{self, Signer};
use crate::{Error, Result};

/// The STS API version every request names.
const VERSION: &str = "2011-04-15";
/// How long a session is asked to last when nothing else is said.
const DEFAULT_DURATION: Duration = Duration::from_secs(3600);
/// The shortest and longest session STS will issue.
const MIN_DURATION: Duration = Duration::from_secs(900);
const MAX_DURATION: Duration = Duration::from_secs(12 * 3600);
/// The session name used when the caller does not choose one.
const DEFAULT_SESSION_NAME: &str = "yggdryl";

/// A role to assume, and how to ask for it.
///
/// Naming one replaces the credential chain's answer rather than adding to it:
/// the chain still finds the keys that sign the *exchange*, and what signs the
/// bucket requests is what STS hands back.
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::holder::s3::{AssumedRole, S3Options};
///
/// let role = AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
///     .with_session_name("power-desk")
///     .with_duration(Duration::from_secs(3600));
/// let options = S3Options::default().with_assumed_role(role);
/// assert_eq!(
///     options.assumed_role().map(AssumedRole::role_arn),
///     Some("arn:aws:iam::123456789012:role/lake-reader")
/// );
/// ```
#[derive(Clone, Debug)]
pub struct AssumedRole {
    role_arn: String,
    session_name: String,
    external_id: Option<String>,
    duration: Duration,
    region: Option<String>,
    endpoint: Option<String>,
}

impl AssumedRole {
    /// Assume the role `role_arn` names.
    pub fn new(role_arn: impl Into<String>) -> Self {
        Self {
            role_arn: role_arn.into(),
            session_name: DEFAULT_SESSION_NAME.to_owned(),
            external_id: None,
            duration: DEFAULT_DURATION,
            region: None,
            endpoint: None,
        }
    }

    /// Name the session, which is what appears in the role's audit trail.
    #[must_use]
    pub fn with_session_name(mut self, session_name: impl Into<String>) -> Self {
        let session_name: String = session_name.into();
        if !session_name.trim().is_empty() {
            self.session_name = session_name.trim().to_owned();
        }
        self
    }

    /// Present the external id the role's trust policy requires.
    #[must_use]
    pub fn with_external_id(mut self, external_id: impl Into<String>) -> Self {
        let external_id: String = external_id.into();
        self.external_id = (!external_id.trim().is_empty()).then(|| external_id.trim().to_owned());
        self
    }

    /// Ask for a session of `duration`, clamped to STS's 15 minute floor and
    /// 12 hour ceiling.
    #[must_use]
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration.clamp(MIN_DURATION, MAX_DURATION);
        self
    }

    /// Reach STS in `region` rather than in the bucket's.
    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Reach STS at `endpoint` rather than at `https://sts.{region}.amazonaws.com`.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let endpoint: String = endpoint.into();
        let endpoint = endpoint.trim().trim_end_matches('/');
        self.endpoint = (!endpoint.is_empty()).then(|| endpoint.to_owned());
        self
    }

    /// The role this asks for.
    pub fn role_arn(&self) -> &str {
        &self.role_arn
    }

    /// The session name the audit trail records.
    pub fn session_name(&self) -> &str {
        &self.session_name
    }

    /// The external id, when the trust policy needs one.
    pub fn external_id(&self) -> Option<&str> {
        self.external_id.as_deref()
    }

    /// How long a session is asked to last.
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// The STS region, when it differs from the bucket's.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// The STS endpoint, when one was named.
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// Where the exchange is sent, and under which region it is signed.
    fn destination(&self, fallback_region: &str) -> (String, String) {
        let region = self
            .region
            .clone()
            .unwrap_or_else(|| fallback_region.to_owned());
        let endpoint = self
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://sts.{region}.amazonaws.com"));
        (endpoint, region)
    }
}

/// Trade `base` for the credentials of `role`, with one signed request.
///
/// # Errors
///
/// Returns STS's refusal, the transport's failure, or an answer that does not
/// carry a credential set.
pub(super) fn assume(
    agent: &ureq::Agent,
    base: &Credentials,
    role: &AssumedRole,
    fallback_region: &str,
    now: SystemTime,
) -> Result<Credentials> {
    let (endpoint, region) = role.destination(fallback_region);
    let mut query = vec![
        ("Action".to_owned(), "AssumeRole".to_owned()),
        ("Version".to_owned(), VERSION.to_owned()),
        ("RoleArn".to_owned(), role.role_arn.clone()),
        ("RoleSessionName".to_owned(), role.session_name.clone()),
        (
            "DurationSeconds".to_owned(),
            role.duration.as_secs().to_string(),
        ),
    ];
    if let Some(external_id) = &role.external_id {
        query.push(("ExternalId".to_owned(), external_id.clone()));
    }
    let (scheme, host) = split_endpoint(&endpoint)?;
    let signer = Signer::for_service(
        "sts",
        base.access_key_id(),
        base.secret_access_key(),
        base.session_token().map(str::to_owned),
        &region,
    );
    // The exchange carries no body, so the empty payload hash is the request's.
    let signed = signer.sign(
        "GET",
        &host,
        "/",
        &query,
        &[],
        sign::EMPTY_PAYLOAD_SHA256,
        now,
    );
    let url = format!("{scheme}://{host}/?{}", sign::canonical_query(&query));
    let mut request = agent.get(&url);
    for (name, value) in &signed {
        request = request.header(name, value);
    }
    let mut answer = request
        .call()
        .map_err(|error| transport_failure(&endpoint, &error))?;
    let status = answer.status().as_u16();
    let body = answer
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|error| transport_failure(&endpoint, &error))?;
    if status >= 300 {
        let (code, message) = super::xml::parse_error(&body).map_or_else(
            || ("AssumeRoleFailed".to_owned(), String::new()),
            |error| (error.code, error.message),
        );
        return Err(Error::remote(
            "sts",
            "AssumeRole",
            status,
            code,
            message,
            &role.role_arn,
        ));
    }
    parse(&body, now).ok_or_else(|| {
        Error::remote(
            "sts",
            "AssumeRole",
            status,
            "MalformedAnswer",
            "the answer carried no credential set",
            &role.role_arn,
        )
    })
}

/// The largest STS answer read into memory; one credential set is a kilobyte.
const MAX_ANSWER: u64 = 256 * 1024;

/// Read `<AssumeRoleResponse><AssumeRoleResult><Credentials>`.
fn parse(body: &[u8], now: SystemTime) -> Option<Credentials> {
    let found = super::xml::parse_assumed_credentials(body).ok()?;
    let mut credentials = Credentials::new(found.access_key_id, found.secret_access_key)
        .with_session_token(found.session_token);
    credentials = match found
        .expiration
        .as_deref()
        .and_then(super::credentials::parse_iso8601_utc)
    {
        Some(expiry) => credentials.with_expiry(expiry),
        // An answer that states no expiry is still temporary; a conservative
        // one is better than treating a session as permanent.
        None => credentials.with_expiry(now + DEFAULT_DURATION),
    };
    Some(credentials)
}

/// The scheme and the host, with its port, of an endpoint URL.
fn split_endpoint(endpoint: &str) -> Result<(String, String)> {
    let (scheme, rest) = endpoint.split_once("://").unwrap_or(("https", endpoint));
    let host = rest.split('/').next().unwrap_or(rest);
    if host.is_empty() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("expected an STS endpoint naming a host, got {endpoint}"),
        )));
    }
    Ok((scheme.to_owned(), host.to_owned()))
}

/// Report a failure to reach STS at all.
fn transport_failure(endpoint: &str, error: &impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        format!("could not assume a role through {endpoint}: {error}"),
    ))
}
