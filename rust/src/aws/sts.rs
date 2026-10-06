//! Trading a role for a credential set, through AWS STS.
//!
//! A process that reaches a bucket in another account, or under a policy of
//! its own, does not hold keys for it: it holds keys that are allowed to
//! *assume a role* that does, or a web identity token a role trusts. One
//! request trades the first for the second, and the answer expires, so it is
//! asked for again shortly before it does rather than per request - and it
//! is written to the cache the AWS CLI keeps under `~/.aws/cli/cache`, so a
//! session the CLI already obtained, an MFA prompt already answered, is
//! reused rather than asked for again.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::credentials::{Credentials, Refusal};
use super::sigv4::{self, Signer};
use crate::auth::{instant, iso8601, write_private};
use crate::xml::scanner::{parse_document, parse_root};
use crate::{Error, Result, Url};

/// The STS API version every request names.
const VERSION: &str = "2011-06-15";
/// How long a session is asked to last when nothing else is said.
const DEFAULT_DURATION: Duration = Duration::from_secs(3600);
/// The shortest and longest session STS will issue.
const MIN_DURATION: Duration = Duration::from_secs(900);
const MAX_DURATION: Duration = Duration::from_secs(12 * 3600);
/// What a generated session name starts with; the AWS tools spell theirs
/// `botocore-session-{seconds}`, and a name this crate generated says so.
const SESSION_NAME_PREFIX: &str = "yggdryl-session";
/// A cached session lapsing within this is not worth reading back.
const CACHE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// The bound on one exchange.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The media type of an exchange's body: the query API's parameters as a
/// form, so nothing an exchange presents - a web identity token, an MFA code -
/// is ever part of a URL a transport failure, a log or a proxy repeats.
const FORM: &str = "application/x-www-form-urlencoded; charset=utf-8";
/// Attempts at one exchange before its failure is the answer: a throttle or
/// a server-side failure is tried again, after a short pause.
const ATTEMPTS: u32 = 3;

/// Where the keys that sign a role exchange come from, when a profile names
/// a source rather than another profile.
///
/// `credential_source` in `~/.aws/config` takes these three values and no
/// other, and a profile names either it or `source_profile`, never both.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CredentialSource {
    /// The `AWS_ACCESS_KEY_ID` pair in the environment.
    Environment,
    /// The instance metadata service.
    Ec2InstanceMetadata,
    /// The container credential endpoint.
    EcsContainer,
}

impl CredentialSource {
    /// The spelling the configuration file uses.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Environment => "Environment",
            Self::Ec2InstanceMetadata => "Ec2InstanceMetadata",
            Self::EcsContainer => "EcsContainer",
        }
    }
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for CredentialSource {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "environment" => Ok(Self::Environment),
            "ec2instancemetadata" => Ok(Self::Ec2InstanceMetadata),
            "ecscontainer" => Ok(Self::EcsContainer),
            _ => Err(Error::Parse {
                target: "credential source",
                position: 0,
                reason: format!(
                    "expected Environment, Ec2InstanceMetadata or EcsContainer, got {value:?}"
                )
                .into(),
            }),
        }
    }
}

/// A role to assume, and how to ask for it.
///
/// Naming one replaces the credential chain's answer rather than adding to it:
/// the chain still finds the keys that sign the *exchange* - or a web identity
/// token replaces them - and what signs the requests after it is what STS
/// hands back.
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::aws::{AssumedRole, Session};
///
/// let role = AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
///     .with_session_name("power-desk")
///     .with_duration(Duration::from_secs(3600));
/// let session = Session::new().with_assumed_role(role);
/// assert_eq!(
///     session.assumed_role().map(AssumedRole::role_arn),
///     Some("arn:aws:iam::123456789012:role/lake-reader")
/// );
/// ```
#[derive(Clone, Debug)]
pub struct AssumedRole {
    role_arn: String,
    session_name: Option<String>,
    external_id: Option<String>,
    duration: Option<Duration>,
    region: Option<String>,
    endpoint: Option<String>,
    mfa_serial: Option<String>,
    source_profile: Option<String>,
    credential_source: Option<CredentialSource>,
    web_identity_token_file: Option<PathBuf>,
}

impl AssumedRole {
    /// Assume the role `role_arn` names.
    pub fn new(role_arn: impl Into<String>) -> Self {
        Self {
            role_arn: role_arn.into(),
            session_name: None,
            external_id: None,
            duration: None,
            region: None,
            endpoint: None,
            mfa_serial: None,
            source_profile: None,
            credential_source: None,
            web_identity_token_file: None,
        }
    }

    /// Name the session, which is what appears in the role's audit trail.
    ///
    /// Unset, a name is generated per exchange, and the CLI cache is keyed
    /// without it - as the AWS tools key theirs - so the two agree on which
    /// cached session is this role's.
    #[must_use]
    pub fn with_session_name(mut self, session_name: impl Into<String>) -> Self {
        let session_name: String = session_name.into();
        let session_name = session_name.trim();
        self.session_name = (!session_name.is_empty()).then(|| session_name.to_owned());
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
        self.duration = Some(duration.clamp(MIN_DURATION, MAX_DURATION));
        self
    }

    /// Reach STS in `region` rather than in the session's.
    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        let region: String = region.into();
        self.region = (!region.trim().is_empty()).then(|| region.trim().to_owned());
        self
    }

    /// Reach STS at `endpoint` rather than at the region's published host.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let endpoint: String = endpoint.into();
        let endpoint = endpoint.trim().trim_end_matches('/');
        self.endpoint = (!endpoint.is_empty()).then(|| endpoint.to_owned());
        self
    }

    /// Present a code from the MFA device `serial` names, which the session
    /// asks its MFA prompt for at each exchange.
    #[must_use]
    pub fn with_mfa_serial(mut self, serial: impl Into<String>) -> Self {
        let serial: String = serial.into();
        self.mfa_serial = (!serial.trim().is_empty()).then(|| serial.trim().to_owned());
        self
    }

    /// Sign the exchange with what the profile `name` answers, the way
    /// `source_profile` in `~/.aws/config` does.
    #[must_use]
    pub fn with_source_profile(mut self, name: impl Into<String>) -> Self {
        let name: String = name.into();
        self.source_profile = (!name.trim().is_empty()).then(|| name.trim().to_owned());
        self
    }

    /// Sign the exchange with what `source` answers, the way
    /// `credential_source` in `~/.aws/config` does.
    #[must_use]
    pub const fn with_credential_source(mut self, source: CredentialSource) -> Self {
        self.credential_source = Some(source);
        self
    }

    /// Present the web identity token the file at `path` holds instead of
    /// signing the exchange, the way `web_identity_token_file` and
    /// `AWS_WEB_IDENTITY_TOKEN_FILE` do.
    ///
    /// The file is read at every exchange, because the platform that writes
    /// it rotates it.
    #[must_use]
    pub fn with_web_identity_token_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.web_identity_token_file = Some(path.into());
        self
    }

    /// The role this asks for.
    pub fn role_arn(&self) -> &str {
        &self.role_arn
    }

    /// The session name the audit trail records, when one was chosen.
    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }

    /// The external id, when the trust policy needs one.
    pub fn external_id(&self) -> Option<&str> {
        self.external_id.as_deref()
    }

    /// How long a session is asked to last: what was chosen, or one hour.
    pub fn duration(&self) -> Duration {
        self.duration.unwrap_or(DEFAULT_DURATION)
    }

    /// The STS region, when it differs from the session's.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// The STS endpoint, when one was named.
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// The MFA device whose code each exchange presents, when one was named.
    pub fn mfa_serial(&self) -> Option<&str> {
        self.mfa_serial.as_deref()
    }

    /// The profile whose keys sign the exchange, when one was named.
    pub fn source_profile(&self) -> Option<&str> {
        self.source_profile.as_deref()
    }

    /// The source whose keys sign the exchange, when one was named.
    pub const fn credential_source(&self) -> Option<CredentialSource> {
        self.credential_source
    }

    /// The file holding the web identity token, when one was named.
    pub fn web_identity_token_file(&self) -> Option<&Path> {
        self.web_identity_token_file.as_deref()
    }

    /// The session name one exchange at `now` uses.
    pub(crate) fn session_name_at(&self, now: SystemTime) -> String {
        self.session_name.clone().unwrap_or_else(|| {
            format!(
                "{SESSION_NAME_PREFIX}-{}",
                now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            )
        })
    }

    /// The CLI cache file this role's sessions are kept under.
    ///
    /// The AWS tools key it by the SHA-1 of the exchange's arguments as JSON,
    /// keys sorted, spelled with Python's default separators, and without a
    /// session name that was generated - so a session the CLI cached is found
    /// by this crate, and the other way round.
    pub(crate) fn cache_key(&self) -> String {
        let mut fields: Vec<(&str, String)> = Vec::new();
        if let Some(duration) = self.duration {
            fields.push(("DurationSeconds", duration.as_secs().to_string()));
        }
        if let Some(external_id) = &self.external_id {
            fields.push(("ExternalId", json_string(external_id)));
        }
        fields.push(("RoleArn", json_string(&self.role_arn)));
        if let Some(name) = &self.session_name {
            fields.push(("RoleSessionName", json_string(name)));
        }
        if let Some(serial) = &self.mfa_serial {
            fields.push(("SerialNumber", json_string(serial)));
        }
        let rendered: Vec<String> = fields
            .iter()
            .map(|(key, value)| format!("\"{key}\": {value}"))
            .collect();
        super::sha1_hex(&format!("{{{}}}", rendered.join(", ")))
    }
}

/// `text` as a JSON string literal.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("{text:?}"))
}

/// Trade `base` for the credentials of `role`, with one signed `POST` to
/// `endpoint` whose form body carries every parameter, signed for `region`.
///
/// # Errors
///
/// Returns STS's refusal - `Error::Remote` with its code and message, located
/// at the role, the endpoint and the signing region, and, for a key STS does
/// not recognize in an opt-in region, saying that region may be what refuses
/// it - the transport's failure naming the endpoint alone, or an answer that
/// does not carry a credential set.
// The argument list is the exchange's parts, each decided by a different
// owner - the role, the session, the prompt, the endpoint rule; a struct
// would only rename them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assume(
    http: &crate::http::Session,
    base: &Credentials,
    role: &AssumedRole,
    session_name: &str,
    token_code: Option<&str>,
    endpoint: &str,
    region: &str,
    now: SystemTime,
) -> Result<Credentials> {
    let mut form = vec![
        ("Action".to_owned(), "AssumeRole".to_owned()),
        ("Version".to_owned(), VERSION.to_owned()),
        ("RoleArn".to_owned(), role.role_arn.clone()),
        ("RoleSessionName".to_owned(), session_name.to_owned()),
        (
            "DurationSeconds".to_owned(),
            role.duration().as_secs().to_string(),
        ),
    ];
    if let Some(external_id) = &role.external_id {
        form.push(("ExternalId".to_owned(), external_id.clone()));
    }
    if let (Some(serial), Some(code)) = (&role.mfa_serial, token_code) {
        form.push(("SerialNumber".to_owned(), serial.clone()));
        form.push(("TokenCode".to_owned(), code.to_owned()));
    }
    let (scheme, host, path) = split_endpoint(endpoint)?;
    let body = sigv4::canonical_query(&form);
    let content_type = [("content-type".to_owned(), FORM.to_owned())];
    let signer = Signer::for_service(
        "sts",
        base.access_key_id(),
        base.secret_access_key(),
        base.session_token().map(str::to_owned),
        region,
    );
    // The signature covers the body that is sent, byte for byte, and the
    // media type it is read as, as botocore signs a query-API POST.
    let mut headers = signer.sign(
        "POST",
        &host,
        &path,
        &[],
        &content_type,
        &sigv4::sha256_hex(body.as_bytes()),
        now,
    );
    headers.extend(content_type);
    exchange(
        http,
        &format!("{scheme}://{host}{path}"),
        body,
        &headers,
        "AssumeRole",
        endpoint,
        &role.role_arn,
        Some(region),
        now,
    )
}

/// Trade the web identity `token` for the credentials of `role`, with one
/// unsigned `POST` to `endpoint` whose form body carries the token beside
/// every other parameter - never the URL, so no transport failure, report
/// or log line can repeat the token.
///
/// # Errors
///
/// As [`assume`], the refusal located at the role and the endpoint.
pub(crate) fn assume_with_web_identity(
    http: &crate::http::Session,
    role: &AssumedRole,
    token: &str,
    session_name: &str,
    endpoint: &str,
    now: SystemTime,
) -> Result<Credentials> {
    let form = vec![
        ("Action".to_owned(), "AssumeRoleWithWebIdentity".to_owned()),
        ("Version".to_owned(), VERSION.to_owned()),
        ("RoleArn".to_owned(), role.role_arn.clone()),
        ("RoleSessionName".to_owned(), session_name.to_owned()),
        ("WebIdentityToken".to_owned(), token.to_owned()),
        (
            "DurationSeconds".to_owned(),
            role.duration().as_secs().to_string(),
        ),
    ];
    let (scheme, host, path) = split_endpoint(endpoint)?;
    exchange(
        http,
        &format!("{scheme}://{host}{path}"),
        sigv4::canonical_query(&form),
        &[("content-type".to_owned(), FORM.to_owned())],
        "AssumeRoleWithWebIdentity",
        endpoint,
        &role.role_arn,
        None,
        now,
    )
}

/// Send one exchange - `body` posted to `url` - and read the credential set
/// it answers.
///
/// A throttle STS states in its body - a `400` naming `Throttling` - or an
/// identity provider it could not reach is tried again under the HTTP
/// client's rules, beside the statuses it retries of its own accord: an
/// exchange changes nothing at STS, so it is idempotent whatever its method.
///
/// A refusal is `Error::Remote` with STS's code and message, located at
/// `{role_arn} at {endpoint}` and, where the exchange was signed, the
/// `(region {region})` it was signed for; a key STS does not recognize
/// (`Refusal::Unrecognized`) in an opt-in region
/// (`ArnPartition::is_opt_in`) says beside STS's message that
/// such a region refuses every key until the account enables it. A transport
/// failure names `endpoint` alone.
// The argument list is the request's parts and the refusal's location, each
// decided by the exchange that calls it; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
fn exchange(
    http: &crate::http::Session,
    url: &str,
    body: String,
    headers: &[(String, String)],
    action: &'static str,
    endpoint: &str,
    role_arn: &str,
    region: Option<&str>,
    now: SystemTime,
) -> Result<Credentials> {
    let mut request = http
        .post(url, body)?
        .with_header("accept", "application/xml")?
        .with_idempotent(true)
        .with_timeout(TIMEOUT)
        .with_max_attempts(ATTEMPTS)
        .with_retry_on(|_, _, body| {
            parse_error(body).is_some_and(|(code, _)| {
                matches!(
                    code.as_str(),
                    "Throttling"
                        | "ThrottlingException"
                        | "RequestLimitExceeded"
                        | "IDPCommunicationError"
                )
            })
        });
    for (name, value) in headers {
        request = request.with_header(name, value)?;
    }
    let answer =
        super::Answer::of(&request).map_err(|error| transport_failure(endpoint, &error))?;
    let (status, body) = (answer.status, answer.body);
    let location = match region {
        Some(region) => format!("{role_arn} at {endpoint} (region {region})"),
        None => format!("{role_arn} at {endpoint}"),
    };
    if status >= 300 {
        let (code, mut message) =
            parse_error(&body).unwrap_or_else(|| (format!("{action}Failed"), String::new()));
        if let Some(region) = region
            && Refusal::from_code(&code) == Some(Refusal::Unrecognized)
            && crate::uri::ArnPartition::is_opt_in(region)
        {
            // `Error::remote` keeps one line of the message, so the hint is
            // appended on it rather than below it.
            message = format!(
                "{} ({region} is an opt-in region: AWS refuses every key there with {code} until \
                 the account enables the region, so a valid key is refused this way by a region \
                 the account has not enabled)",
                message.lines().next().unwrap_or_default()
            );
        }
        return Err(Error::remote(
            "sts", action, status, code, message, location,
        ));
    }
    parse(&body, action, now).ok_or_else(|| {
        Error::remote(
            "sts",
            action,
            status,
            "MalformedAnswer",
            "the answer carried no credential set",
            location,
        )
    })
}

/// Read `<{action}Response><{action}Result><Credentials>`.
fn parse(body: &[u8], action: &str, now: SystemTime) -> Option<Credentials> {
    let root = parse_root(body, &format!("{action}Response")).ok()?;
    let result = root.child(&format!("{action}Result"))?;
    let found = result.child("Credentials")?;
    let mut credentials = Credentials::new(
        found.child_text("AccessKeyId")?,
        found.child_text("SecretAccessKey")?,
    )
    .with_session_token(found.child_text("SessionToken").unwrap_or_default());
    credentials = match found.child_text("Expiration").and_then(instant) {
        Some(expiry) => credentials.with_expiry(expiry),
        // An answer that states no expiry is still temporary; a conservative
        // one is better than treating a session as permanent.
        None => credentials.with_expiry(now + DEFAULT_DURATION),
    };
    if let Some(account) = result
        .child("AssumedRoleUser")
        .and_then(|user| user.child_text("Arn"))
        .and_then(|arn| crate::Arn::from_str(arn).ok())
        .and_then(|arn| arn.account().map(str::to_owned))
    {
        credentials = credentials.with_account_id(account);
    }
    Some(credentials)
}

/// Read `<ErrorResponse><Error><Code>..</Code><Message>..</Message>`, or
/// the bare `<Error>` S3 answers with.
pub(crate) fn parse_error(body: &[u8]) -> Option<(String, String)> {
    let root = parse_document(body).ok()?;
    let error = match root.name() {
        "ErrorResponse" => root.child("Error")?,
        "Error" => &root,
        _ => return None,
    };
    Some((
        error.child_text("Code").unwrap_or_default().to_owned(),
        error.child_text("Message").unwrap_or_default().to_owned(),
    ))
}

/// The scheme, the host with its port, and the path of an endpoint URL.
///
/// The endpoint is read once, as the URL it is, so what is signed and what
/// is dialed are one reading: a bare host or `host:port` is reached over
/// `https`, `http` and `https` are the only schemes an STS endpoint has, and
/// user information and a query are no part of where it is. Its path - a
/// gateway mounting STS below one - is where every exchange is sent and what
/// it signs, as botocore sends it: `/` where the endpoint has none, a
/// trailing `/` dropped as the session drops it.
fn split_endpoint(endpoint: &str) -> Result<(String, String, String)> {
    let refuse = |reason: &str| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("expected an STS endpoint naming a host, got {endpoint}: {reason}"),
        ))
    };
    let named = if endpoint.contains("://") {
        endpoint.to_owned()
    } else {
        format!("https://{endpoint}")
    };
    let url = Url::from_str(&named).map_err(|error| refuse(&error.to_string()))?;
    if !url.scheme().is_http() {
        return Err(refuse(&format!("{} is not http or https", url.scheme())));
    }
    let host = url.authority().host_port();
    if host.is_empty() {
        return Err(refuse("no host"));
    }
    let path = match url.path().as_str().trim_end_matches('/') {
        "" => "/",
        path => path,
    };
    Ok((
        url.scheme().as_str().to_owned(),
        host.to_owned(),
        path.to_owned(),
    ))
}

/// Report a failure to reach STS at all, naming the endpoint and the kind
/// of failure the transport met - a refused connection, a timeout - and
/// nothing of the request: the transport's own text names the URL it was
/// sending, which is no part of what this error may repeat.
fn transport_failure(endpoint: &str, error: &Error) -> Error {
    let kind = match error {
        Error::Io(error) => error.kind(),
        _ => std::io::ErrorKind::Other,
    };
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        format!("could not reach STS at {endpoint}: {kind}"),
    ))
}

/// The session the CLI cache holds under `key`, when it holds one that
/// lasts.
pub(crate) fn read_cache(directory: &Path, key: &str, now: SystemTime) -> Option<Credentials> {
    let text = std::fs::read(directory.join(format!("{key}.json"))).ok()?;
    let document: serde_json::Value = serde_json::from_slice(&text).ok()?;
    let held = document.get("Credentials")?;
    let text = |name: &str| held.get(name).and_then(serde_json::Value::as_str);
    let expiry = instant(text("Expiration")?)?;
    if expiry <= now + CACHE_WINDOW {
        return None;
    }
    let mut found = Credentials::new(text("AccessKeyId")?, text("SecretAccessKey")?)
        .with_session_token(text("SessionToken").unwrap_or_default())
        .with_expiry(expiry);
    if let Some(account) = text("AccountId") {
        found = found.with_account_id(account);
    }
    Some(found)
}

/// Write `credentials` to the CLI cache under `key`, in the shape the AWS
/// tools read back - the account beside the keys when the source said, as
/// they file it. A cache that cannot be written is a cache nothing reads,
/// not a failure of the exchange.
pub(crate) fn write_cache(directory: &Path, key: &str, credentials: &Credentials) {
    let Some(expiry) = credentials.expires_at() else {
        return;
    };
    let mut document = serde_json::json!({
        "Credentials": {
            "AccessKeyId": credentials.access_key_id(),
            "SecretAccessKey": credentials.secret_access_key(),
            "SessionToken": credentials.session_token().unwrap_or_default(),
            "Expiration": iso8601(expiry).replace('Z', "UTC"),
        }
    });
    if let Some(account) = credentials.account_id() {
        document["Credentials"]["AccountId"] = account.into();
    }
    let _ = write_private(
        &directory.join(format!("{key}.json")),
        document.to_string().as_bytes(),
    );
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/sts.rs` pins and a caller cannot reach.
    //!
    //! The exchange is driven over a socket by the suite; what is pinned here
    //! is the reading of STS's answers and the cache the CLI shares. Each
    //! item forwards.
    use std::path::Path;
    use std::time::SystemTime;

    use crate::aws::{AssumedRole, Credentials};

    /// Read `<{action}Response><{action}Result><Credentials>`.
    pub fn parse(body: &[u8], action: &str, now: SystemTime) -> Option<Credentials> {
        super::parse(body, action, now)
    }

    /// Read an STS `<ErrorResponse>` as its code and message.
    pub fn parse_error(body: &[u8]) -> Option<(String, String)> {
        super::parse_error(body)
    }

    /// The CLI cache file this role's sessions are kept under.
    pub fn cache_key(role: &AssumedRole) -> String {
        role.cache_key()
    }

    /// The session the CLI cache holds under `key`, when it lasts.
    pub fn read_cache(directory: &Path, key: &str, now: SystemTime) -> Option<Credentials> {
        super::read_cache(directory, key, now)
    }

    /// Write `credentials` to the CLI cache under `key`.
    pub fn write_cache(directory: &Path, key: &str, credentials: &Credentials) {
        super::write_cache(directory, key, credentials);
    }

    /// The session name one exchange at `now` uses.
    pub fn session_name_at(role: &AssumedRole, now: SystemTime) -> String {
        role.session_name_at(now)
    }

    /// The scheme, the host with its port, and the path an endpoint is
    /// signed and dialed as.
    ///
    /// # Errors
    ///
    /// A refusal naming the endpoint when it is no URL naming a host over
    /// `http` or `https`.
    pub fn split_endpoint(endpoint: &str) -> crate::Result<(String, String, String)> {
        super::split_endpoint(endpoint)
    }
}
