//! Where the keys that sign a request come from.
//!
//! One credential set is a value; finding one is a chain that stops at the
//! first source with an answer, in the order the AWS tools use: explicit,
//! environment, the shared files, the container endpoint, the instance
//! metadata service. Nothing is read before the first request, and a set that
//! expires is refreshed shortly before it does.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::{Error, Result};

/// Refresh an expiring set this long before it lapses, so a request signed at
/// the edge is never rejected mid-flight.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Where the ECS/EKS container agent serves credentials from.
const CONTAINER_HOST: &str = "http://169.254.170.2";
/// Where the EC2 instance metadata service lives when nothing else says.
const METADATA_HOST: &str = "http://169.254.169.254";
/// How long an IMDSv2 session token is asked to last.
const METADATA_TOKEN_TTL: &str = "21600";
/// The two metadata endpoints are on-link addresses: when they do not answer
/// at once, this is not an EC2 instance, and waiting longer helps nobody.
const METADATA_TIMEOUT: Duration = Duration::from_secs(1);

/// An access key, its secret, and the session token a temporary set carries.
///
/// The secret never appears in `Debug` output or in an error.
///
/// ```
/// use yggdryl::holder::s3::Credentials;
///
/// let keys = Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
/// assert_eq!(keys.access_key_id(), "AKIAIOSFODNN7EXAMPLE");
/// assert_eq!(keys.session_token(), None);
/// assert!(!format!("{keys:?}").contains("wJalrXUtnFEMI"));
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    /// When a temporary set stops being accepted; a long-lived set has none.
    expires_at: Option<SystemTime>,
}

impl Credentials {
    /// A long-lived key pair.
    pub fn new(access_key_id: impl Into<String>, secret_access_key: impl Into<String>) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
            expires_at: None,
        }
    }

    /// Carry the session token a temporary set signs with.
    #[must_use]
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        let token: String = token.into();
        self.session_token = (!token.is_empty()).then_some(token);
        self
    }

    /// Record when a temporary set lapses.
    #[must_use]
    pub const fn with_expiry(mut self, expires_at: SystemTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// The access key id.
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// The secret, for the signer alone.
    pub(super) fn secret_access_key(&self) -> &str {
        &self.secret_access_key
    }

    /// The session token, when the set is temporary.
    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }

    /// When the set lapses, when it does.
    pub const fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }

    /// Whether the set should be replaced before signing at `now`.
    fn is_stale(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|expiry| {
            now.checked_add(REFRESH_MARGIN)
                .is_none_or(|edge| edge >= expiry)
        })
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// What the client consults when it needs a credential set.
pub(super) enum CredentialSource {
    /// Requests go unsigned.
    Anonymous,
    /// One set, given explicitly or read from the URL.
    Fixed(Credentials),
    /// The environment, the shared files, then the two metadata services.
    Chain {
        /// The profile the shared files are read for.
        profile: Option<String>,
    },
    /// A role, assumed with whatever the source under it answers.
    Role {
        /// The role to trade for.
        role: Box<super::sts::AssumedRole>,
        /// Where the keys that sign the exchange come from.
        base: Box<CredentialSource>,
        /// The region STS is reached in when the role does not name one.
        region: String,
    },
}

/// The chain plus the set it last found, refreshed before expiry.
pub(super) struct CredentialCache {
    source: CredentialSource,
    cached: Mutex<Option<Credentials>>,
}

impl CredentialCache {
    pub(super) const fn new(source: CredentialSource) -> Self {
        Self {
            source,
            cached: Mutex::new(None),
        }
    }

    /// The set to sign with at `now`, or `None` for an anonymous request.
    ///
    /// The first call walks the chain; later calls answer from the cache until
    /// a temporary set nears its expiry, and a chain that finds nothing is
    /// anonymous rather than an error, because a public bucket is a valid
    /// destination.
    ///
    /// # Errors
    ///
    /// Returns a metadata service's transport failure only when it answered
    /// and then failed; a service that is not there is silence, not failure.
    pub(super) fn resolve(
        &self,
        agent: &ureq::Agent,
        now: SystemTime,
    ) -> Result<Option<Credentials>> {
        match &self.source {
            CredentialSource::Anonymous => Ok(None),
            CredentialSource::Fixed(credentials) => Ok(Some(credentials.clone())),
            CredentialSource::Chain { .. } | CredentialSource::Role { .. } => {
                let mut cached = self.cached.lock().map_err(|_| poisoned())?;
                if let Some(credentials) = cached.as_ref() {
                    if !credentials.is_stale(now) {
                        return Ok(Some(credentials.clone()));
                    }
                }
                let found = resolve_source(&self.source, agent, now)?;
                cached.clone_from(&found);
                Ok(found)
            }
        }
    }
}

/// Walk one source, with no cache of its own.
///
/// A role's base is walked again whenever the session is refreshed, which is
/// once an hour rather than once a request - and is what keeps a base that
/// expires too, like the instance metadata service's, from going stale.
fn resolve_source(
    source: &CredentialSource,
    agent: &ureq::Agent,
    now: SystemTime,
) -> Result<Option<Credentials>> {
    match source {
        CredentialSource::Anonymous => Ok(None),
        CredentialSource::Fixed(credentials) => Ok(Some(credentials.clone())),
        CredentialSource::Chain { profile } => from_chain(agent, profile.as_deref()),
        CredentialSource::Role { role, base, region } => {
            let Some(base) = resolve_source(base, agent, now)? else {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "expected keys to assume the role {} with, and found none",
                        role.role_arn()
                    ),
                )));
            };
            super::sts::assume(agent, &base, role, region, now).map(Some)
        }
    }
}

/// Walk the chain once.
fn from_chain(agent: &ureq::Agent, profile: Option<&str>) -> Result<Option<Credentials>> {
    if let Some(credentials) = from_environment() {
        return Ok(Some(credentials));
    }
    if let Some(credentials) = super::profile::credentials(profile) {
        return Ok(Some(credentials));
    }
    if let Some(credentials) = from_container(agent)? {
        return Ok(Some(credentials));
    }
    from_instance_metadata(agent)
}

/// `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, with `AWS_SESSION_TOKEN`.
fn from_environment() -> Option<Credentials> {
    let access_key_id = variable("AWS_ACCESS_KEY_ID")?;
    let secret_access_key = variable("AWS_SECRET_ACCESS_KEY")?;
    let mut credentials = Credentials::new(access_key_id, secret_access_key);
    if let Some(token) = variable("AWS_SESSION_TOKEN") {
        credentials = credentials.with_session_token(token);
    }
    Some(credentials)
}

/// A non-empty environment variable.
pub(super) fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The ECS/EKS container credential endpoint, when the task exposes one.
fn from_container(agent: &ureq::Agent) -> Result<Option<Credentials>> {
    let url = match (
        variable("AWS_CONTAINER_CREDENTIALS_FULL_URI"),
        variable("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI"),
    ) {
        (Some(full), _) => full,
        (None, Some(relative)) => format!("{CONTAINER_HOST}{relative}"),
        (None, None) => return Ok(None),
    };
    let mut request = agent
        .get(&url)
        .config()
        .timeout_global(Some(METADATA_TIMEOUT))
        .build();
    if let Some(token) = variable("AWS_CONTAINER_AUTHORIZATION_TOKEN") {
        request = request.header("Authorization", token);
    }
    let mut response = request
        .call()
        .map_err(|error| metadata_failure("container credentials", error))?;
    if response.status() != 200 {
        return Err(Error::remote(
            "s3",
            "ContainerCredentials",
            response.status().as_u16(),
            "CredentialsUnavailable",
            format!(
                "the container credential endpoint answered {}",
                response.status()
            ),
            url,
        ));
    }
    let body = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| metadata_failure("container credentials", error))?;
    parse_metadata_credentials(&body, "container credentials").map(Some)
}

/// The EC2 instance metadata service, over IMDSv2.
///
/// Silence - a connection that fails or times out - means this is not an EC2
/// instance, and the chain ends without credentials. Anything the service does
/// say that is not a credential set is a failure worth reporting.
fn from_instance_metadata(agent: &ureq::Agent) -> Result<Option<Credentials>> {
    if variable("AWS_EC2_METADATA_DISABLED").is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return Ok(None);
    }
    let host = variable("AWS_EC2_METADATA_SERVICE_ENDPOINT")
        .map(|host| host.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| METADATA_HOST.to_owned());

    let token = agent
        .put(format!("{host}/latest/api/token"))
        .config()
        .timeout_global(Some(METADATA_TIMEOUT))
        .build()
        .header("x-aws-ec2-metadata-token-ttl-seconds", METADATA_TOKEN_TTL)
        .send_empty();
    let Ok(mut token) = token else {
        // Not reachable: not an EC2 instance.
        return Ok(None);
    };
    if token.status() != 200 {
        return Ok(None);
    }
    let token = token
        .body_mut()
        .read_to_string()
        .map_err(|error| metadata_failure("instance metadata", error))?;
    let token = token.trim().to_owned();

    let mut role = agent
        .get(format!("{host}/latest/meta-data/iam/security-credentials/"))
        .config()
        .timeout_global(Some(METADATA_TIMEOUT))
        .build()
        .header("x-aws-ec2-metadata-token", &token)
        .call()
        .map_err(|error| metadata_failure("instance metadata", error))?;
    if role.status() != 200 {
        // An instance without a role has no credentials to give.
        return Ok(None);
    }
    let role = role
        .body_mut()
        .read_to_string()
        .map_err(|error| metadata_failure("instance metadata", error))?;
    let Some(role) = role.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return Ok(None);
    };

    let mut response = agent
        .get(format!(
            "{host}/latest/meta-data/iam/security-credentials/{role}"
        ))
        .config()
        .timeout_global(Some(METADATA_TIMEOUT))
        .build()
        .header("x-aws-ec2-metadata-token", &token)
        .call()
        .map_err(|error| metadata_failure("instance metadata", error))?;
    if response.status() != 200 {
        return Ok(None);
    }
    let body = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| metadata_failure("instance metadata", error))?;
    parse_metadata_credentials(&body, "instance metadata").map(Some)
}

/// The JSON document both metadata services answer with.
fn parse_metadata_credentials(body: &[u8], source: &str) -> Result<Credentials> {
    let document: serde_json::Value = serde_json::from_slice(body).map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected a JSON credential document from {source}, got {error}"),
        ))
    })?;
    let text = |name: &str| {
        document
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let (Some(access_key_id), Some(secret_access_key)) =
        (text("AccessKeyId"), text("SecretAccessKey"))
    else {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected AccessKeyId and SecretAccessKey from {source}, got neither"),
        )));
    };
    let mut credentials = Credentials::new(access_key_id, secret_access_key);
    if let Some(token) = text("Token") {
        credentials = credentials.with_session_token(token);
    }
    if let Some(expiry) = text("Expiration").as_deref().and_then(parse_iso8601_utc) {
        credentials = credentials.with_expiry(expiry);
    }
    Ok(credentials)
}

/// Parse `YYYY-MM-DDThh:mm:ssZ`, the shape both metadata services use.
///
/// Fractional seconds are accepted and ignored. Anything else - an offset,
/// a date alone - is not a time this reads, and answers `None` so the set is
/// treated as long-lived rather than expiring at a guessed instant.
pub(super) fn parse_iso8601_utc(text: &str) -> Option<SystemTime> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let (year, month, day): (i64, u32, u32) = (
        date.next()?.parse().ok()?,
        date.next()?.parse().ok()?,
        date.next()?.parse().ok()?,
    );
    if date.next().is_some() {
        return None;
    }
    let time = time.split('.').next()?;
    let mut time = time.split(':');
    let (hour, minute, second): (u64, u64, u64) = (
        time.next()?.parse().ok()?,
        time.next()?.parse().ok()?,
        time.next()?.parse().ok()?,
    );
    if time.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = u64::try_from(days).ok()?.checked_mul(86_400)?;
    let seconds = seconds
        .checked_add(hour * 3600)?
        .checked_add(minute * 60)?
        .checked_add(second)?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A metadata service answered and then failed.
fn metadata_failure(source: &str, error: ureq::Error) -> Error {
    Error::Io(std::io::Error::other(format!(
        "reading {source} failed: {error}"
    )))
}

/// Report a poisoned cache lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the credential cache lock was poisoned by a panicking writer",
    ))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{Credentials, days_from_civil, parse_iso8601_utc, parse_metadata_credentials};

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
        assert!(!fresh.is_stale(now));
        let expiring = Credentials::new("a", "b").with_expiry(now + Duration::from_secs(60));
        assert!(expiring.is_stale(now));
        assert!(!Credentials::new("a", "b").is_stale(now));
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
}
