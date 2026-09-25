//! IAM Identity Center, which the AWS tools still call SSO.
//!
//! A person signs in once, in a browser, and the tools keep the token that
//! sign-in produced under `~/.aws/sso/cache`, named by the SHA-1 of the
//! session name (or, for the older profile shape, of the start URL). Any
//! process on the machine then trades that token for a role's keys at the
//! access portal, and refreshes it through the OIDC service when the sign-in
//! registered a client that can. This module reads and writes that cache the
//! way the AWS CLI does, so a sign-in made by `aws sso login` serves this
//! crate and one made here serves the CLI, and it performs the sign-in
//! itself, the OAuth 2.0 device flow, when a session was told how to show
//! the person where to go.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::credentials::Credentials;
use crate::auth::{Secret, instant, instant_from_millis, iso8601, write_private};
use crate::{Error, Result};

/// The scope a sign-in registers for when the profile names none.
const DEFAULT_SCOPE: &str = "sso:account:access";
/// The name a client registered by this crate carries.
const CLIENT_NAME: &str = "yggdryl";
/// The OAuth 2.0 grant a device sign-in completes with.
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The grant a refresh token is traded under.
const REFRESH_GRANT: &str = "refresh_token";
/// A token lapsing within this is refreshed when it can be.
const REFRESH_WINDOW: Duration = Duration::from_secs(15 * 60);
/// The interval between polls when the service names none.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
/// How much longer to wait between polls when the service asks to slow down.
const SLOW_DOWN: Duration = Duration::from_secs(5);
/// The largest document any of these endpoints answers.
const MAX_ANSWER: u64 = 256 * 1024;
/// The bound on one request to the portal or the OIDC service.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The longest lifetime a service's `expiresIn` is believed; a bigger one
/// is a document nobody meant.
const MAX_LIFETIME: Duration = Duration::from_secs(90 * 86_400);
/// Attempts at one request before its failure is the answer.
const ATTEMPTS: u32 = 3;
const RETRY_PAUSE: Duration = Duration::from_millis(200);

/// One IAM Identity Center sign-in and the role it is traded for.
///
/// What a profile states as `sso_session` (with its `[sso-session]` section),
/// `sso_start_url`, `sso_region`, `sso_account_id` and `sso_role_name`, so a
/// caller can state the same thing without a file.
///
/// ```
/// use yggdryl::aws::{Session, Sso};
///
/// let sso = Sso::new("https://trading.awsapps.com/start", "eu-west-1", "123456789012", "LakeReader")
///     .with_session_name("trading");
/// let session = Session::new().with_sso(sso);
/// assert_eq!(session.sso().map(Sso::role_name), Some("LakeReader"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sso {
    start_url: String,
    region: String,
    account_id: String,
    role_name: String,
    session_name: Option<String>,
    scopes: Vec<String>,
}

impl Sso {
    /// The sign-in at `start_url`, served from `region`, traded for
    /// `role_name` in `account_id`.
    pub fn new(
        start_url: impl Into<String>,
        region: impl Into<String>,
        account_id: impl Into<String>,
        role_name: impl Into<String>,
    ) -> Self {
        Self {
            start_url: start_url.into().trim().to_owned(),
            region: region.into().trim().to_owned(),
            account_id: account_id.into().trim().to_owned(),
            role_name: role_name.into().trim().to_owned(),
            session_name: None,
            scopes: Vec::new(),
        }
    }

    /// Name the `[sso-session]` the sign-in belongs to, which is what the
    /// cached token is filed under and what makes it refreshable.
    #[must_use]
    pub fn with_session_name(mut self, name: impl Into<String>) -> Self {
        let name: String = name.into();
        self.session_name = (!name.trim().is_empty()).then(|| name.trim().to_owned());
        self
    }

    /// Register the sign-in for `scopes` rather than for `sso:account:access`.
    #[must_use]
    pub fn with_scopes<S: Into<String>>(mut self, scopes: impl IntoIterator<Item = S>) -> Self {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// The start URL the sign-in begins at.
    pub fn start_url(&self) -> &str {
        &self.start_url
    }

    /// The region the sign-in is served from.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// The account the role is in.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// The role the sign-in is traded for.
    pub fn role_name(&self) -> &str {
        &self.role_name
    }

    /// The `[sso-session]` name, when the sign-in belongs to one.
    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }

    /// The scopes the sign-in registers for.
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    /// The cache file the sign-in's token is filed under.
    pub(crate) fn token_cache_key(&self) -> String {
        super::sha1_hex(self.session_name.as_deref().unwrap_or(&self.start_url))
    }

    /// The CLI cache file the role's keys are filed under: the SHA-1 of the
    /// compact JSON the AWS tools key theirs by - the account, the role, and
    /// the session name where the sign-in belongs to one, else the start URL.
    pub(crate) fn credentials_cache_key(&self) -> String {
        let mut fields = vec![
            ("accountId", self.account_id.as_str()),
            ("roleName", self.role_name.as_str()),
        ];
        match &self.session_name {
            Some(session) => fields.push(("sessionName", session)),
            None => fields.push(("startUrl", &self.start_url)),
        }
        let rendered: Vec<String> = fields
            .iter()
            .map(|(key, value)| {
                format!(
                    "\"{key}\":{}",
                    serde_json::to_string(value).unwrap_or_default()
                )
            })
            .collect();
        super::sha1_hex(&format!("{{{}}}", rendered.join(",")))
    }

    /// The OIDC service the sign-in is registered and refreshed at.
    pub(crate) fn oidc_endpoint(&self) -> String {
        format!("https://oidc.{}.amazonaws.com", self.region)
    }

    /// The access portal the token is traded at.
    pub(crate) fn portal_endpoint(&self) -> String {
        format!("https://portal.sso.{}.amazonaws.com", self.region)
    }
}

/// What a session does when a sign-in is needed and none is cached.
///
/// The device flow needs a person: somebody has to open a URL and confirm a
/// code. A library cannot know where that person is, so a session is told,
/// and the default is to refuse - the chain then walks on to its other
/// sources and the refusal names `aws sso login` as the way out.
#[derive(Clone, Default)]
pub enum SsoLogin {
    /// Never sign in; a lapsed sign-in is a source that failed.
    #[default]
    Never,
    /// Print where to go and which code to confirm on standard error, then
    /// wait, which is what `aws sso login --no-browser` does.
    Stderr,
    /// Hand the authorization to `handler`, which shows it however the
    /// process shows things, then wait.
    Handler(Arc<dyn Fn(&DeviceAuthorization) + Send + Sync>),
}

impl std::fmt::Debug for SsoLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Never => "Never",
            Self::Stderr => "Stderr",
            Self::Handler(_) => "Handler",
        })
    }
}

/// Where a person completes a device sign-in.
///
/// ```
/// use yggdryl::aws::DeviceAuthorization;
///
/// # fn show(authorization: &DeviceAuthorization) {
/// // What a handler shows: the page to open and the code to confirm there.
/// println!("{authorization}");
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct DeviceAuthorization {
    verification_uri: String,
    verification_uri_complete: Option<String>,
    user_code: String,
    expires_in: Duration,
    interval: Duration,
    device_code: Secret,
}

impl DeviceAuthorization {
    /// The page to open.
    pub fn verification_uri(&self) -> &str {
        &self.verification_uri
    }

    /// The page to open with the code already filled in, when the service
    /// offered one.
    pub fn verification_uri_complete(&self) -> Option<&str> {
        self.verification_uri_complete.as_deref()
    }

    /// The code the person confirms on that page.
    pub fn user_code(&self) -> &str {
        &self.user_code
    }

    /// How long the authorization stays open.
    pub const fn expires_in(&self) -> Duration {
        self.expires_in
    }

    /// How long to wait between polls.
    pub const fn interval(&self) -> Duration {
        self.interval
    }
}

impl std::fmt::Display for DeviceAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Open {} and confirm the code {}",
            self.verification_uri_complete
                .as_deref()
                .unwrap_or(&self.verification_uri),
            self.user_code
        )
    }
}

/// One sign-in's token, as the cache files it.
#[derive(Clone, Debug)]
pub struct Token {
    pub access_token: Secret,
    pub expires_at: SystemTime,
    pub refresh_token: Option<Secret>,
    pub client_id: Option<String>,
    pub client_secret: Option<Secret>,
    pub registration_expires_at: Option<SystemTime>,
    pub region: Option<String>,
    pub start_url: Option<String>,
}

impl Token {
    /// The token the cache file at `path` holds, when it holds one.
    pub fn read(path: &Path) -> Option<Self> {
        let text = std::fs::read(path).ok()?;
        Self::parse(&text)
    }

    /// The token a cache document states.
    pub fn parse(text: &[u8]) -> Option<Self> {
        let document: serde_json::Value = serde_json::from_slice(text).ok()?;
        let field = |name: &str| {
            document
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            access_token: Secret::new(field("accessToken")?),
            expires_at: instant(&field("expiresAt")?)?,
            refresh_token: field("refreshToken").map(Secret::new),
            client_id: field("clientId"),
            client_secret: field("clientSecret").map(Secret::new),
            registration_expires_at: field("registrationExpiresAt").as_deref().and_then(instant),
            region: field("region"),
            start_url: field("startUrl"),
        })
    }

    /// The cache document the AWS tools read this token back from.
    pub fn render(&self) -> String {
        let mut document = serde_json::Map::new();
        let mut put = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                document.insert(key.to_owned(), serde_json::Value::String(value));
            }
        };
        put("startUrl", self.start_url.clone());
        put("region", self.region.clone());
        put("accessToken", Some(self.access_token.expose().to_owned()));
        put("expiresAt", Some(iso8601(self.expires_at)));
        put("clientId", self.client_id.clone());
        put(
            "clientSecret",
            self.client_secret
                .as_ref()
                .map(|secret| secret.expose().to_owned()),
        );
        put(
            "registrationExpiresAt",
            self.registration_expires_at.map(iso8601),
        );
        put(
            "refreshToken",
            self.refresh_token
                .as_ref()
                .map(|token| token.expose().to_owned()),
        );
        serde_json::Value::Object(document).to_string()
    }

    /// Write the token to `path` as a file only its owner reads, creating
    /// the cache directory the same way.
    ///
    /// # Errors
    ///
    /// The file system's refusal.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        write_private(path, self.render().as_bytes())
    }

    /// Whether the token lapses within the refresh window.
    pub(crate) fn is_stale(&self, now: SystemTime) -> bool {
        now.checked_add(REFRESH_WINDOW)
            .is_none_or(|edge| edge >= self.expires_at)
    }

    /// Whether the token has lapsed.
    pub(crate) fn is_expired(&self, now: SystemTime) -> bool {
        now >= self.expires_at
    }

    /// Whether the sign-in registered a client that can refresh the token,
    /// and that registration still stands.
    pub(crate) fn can_refresh(&self, now: SystemTime) -> bool {
        self.refresh_token.is_some()
            && self.client_id.is_some()
            && self.client_secret.is_some()
            && self
                .registration_expires_at
                .is_none_or(|expiry| now < expiry)
    }
}

/// Trade the refresh token for a fresh access token.
///
/// # Errors
///
/// The OIDC service's refusal, the transport's failure, or an answer without
/// a token.
pub(crate) fn refresh(
    agent: &ureq::Agent,
    oidc: &str,
    token: &Token,
    now: SystemTime,
) -> Result<Token> {
    let (Some(client_id), Some(client_secret), Some(refresh_token)) =
        (&token.client_id, &token.client_secret, &token.refresh_token)
    else {
        return Err(refusal(
            "the cached sign-in registered no client that could refresh it",
        ));
    };
    let answer = post_json(
        agent,
        &format!("{oidc}/token"),
        &serde_json::json!({
            "clientId": client_id,
            "clientSecret": client_secret.expose(),
            "grantType": REFRESH_GRANT,
            "refreshToken": refresh_token.expose(),
        }),
        "CreateToken",
        token.start_url.as_deref().unwrap_or(oidc),
    )?;
    Ok(Token {
        access_token: Secret::new(required_text(&answer, "accessToken", "CreateToken")?),
        expires_at: lapse(now, seconds(&answer, "expiresIn")),
        refresh_token: text(&answer, "refreshToken")
            .map(Secret::new)
            .or_else(|| token.refresh_token.clone()),
        client_id: token.client_id.clone(),
        client_secret: token.client_secret.clone(),
        registration_expires_at: token.registration_expires_at,
        region: token.region.clone(),
        start_url: token.start_url.clone(),
    })
}

/// Sign in through the device flow: register a client, open a device
/// authorization, hand it to `prompt`, and poll until the person confirms.
///
/// # Errors
///
/// A `prompt` of [`SsoLogin::Never`], the service's refusal at any of its
/// three steps, an authorization that expired or was denied, or the
/// transport's failure.
pub(crate) fn login(
    agent: &ureq::Agent,
    oidc: &str,
    sso: &Sso,
    prompt: &SsoLogin,
    now: SystemTime,
) -> Result<Token> {
    if matches!(prompt, SsoLogin::Never) {
        return Err(refusal(format!(
            "the IAM Identity Center sign-in for {} has lapsed or was never made: run \
             `aws sso login`, or tell the session how to show the sign-in with \
             Session::with_sso_login",
            sso.start_url
        )));
    }
    let scopes: Vec<&str> = if sso.scopes.is_empty() {
        vec![DEFAULT_SCOPE]
    } else {
        sso.scopes.iter().map(String::as_str).collect()
    };
    let registered = post_json(
        agent,
        &format!("{oidc}/client/register"),
        &serde_json::json!({
            "clientName": CLIENT_NAME,
            "clientType": "public",
            "scopes": scopes,
        }),
        "RegisterClient",
        &sso.start_url,
    )?;
    let client_id = required_text(&registered, "clientId", "RegisterClient")?;
    let client_secret = required_text(&registered, "clientSecret", "RegisterClient")?;
    let registration_expires_at = registered
        .get("clientSecretExpiresAt")
        .and_then(serde_json::Value::as_i64)
        .and_then(|seconds| {
            UNIX_EPOCH.checked_add(Duration::from_secs(seconds.max(0).unsigned_abs()))
        });

    let started = post_json(
        agent,
        &format!("{oidc}/device_authorization"),
        &serde_json::json!({
            "clientId": client_id,
            "clientSecret": client_secret,
            "startUrl": sso.start_url,
        }),
        "StartDeviceAuthorization",
        &sso.start_url,
    )?;
    let authorization = DeviceAuthorization {
        verification_uri: required_text(&started, "verificationUri", "StartDeviceAuthorization")?,
        verification_uri_complete: text(&started, "verificationUriComplete"),
        user_code: required_text(&started, "userCode", "StartDeviceAuthorization")?,
        expires_in: seconds(&started, "expiresIn").unwrap_or(Duration::from_secs(600)),
        interval: seconds(&started, "interval").unwrap_or(DEFAULT_INTERVAL),
        device_code: Secret::new(required_text(
            &started,
            "deviceCode",
            "StartDeviceAuthorization",
        )?),
    };
    match prompt {
        SsoLogin::Never => unreachable!("refused above"),
        SsoLogin::Stderr => eprintln!("{authorization}"),
        SsoLogin::Handler(handler) => handler(&authorization),
    }

    let deadline = now + authorization.expires_in;
    let mut interval = authorization.interval;
    loop {
        std::thread::sleep(interval);
        let answer = post(
            agent,
            &format!("{oidc}/token"),
            &serde_json::json!({
                "clientId": client_id,
                "clientSecret": client_secret,
                "grantType": DEVICE_GRANT,
                "deviceCode": authorization.device_code.expose(),
            }),
            &sso.start_url,
        )?;
        let (status, document) = answer;
        if status == 200 {
            return Ok(Token {
                access_token: Secret::new(required_text(&document, "accessToken", "CreateToken")?),
                expires_at: lapse(SystemTime::now(), seconds(&document, "expiresIn")),
                refresh_token: text(&document, "refreshToken").map(Secret::new),
                client_id: Some(client_id.clone()),
                client_secret: Some(Secret::new(client_secret.clone())),
                registration_expires_at,
                region: Some(sso.region.clone()),
                start_url: Some(sso.start_url.clone()),
            });
        }
        let error = text(&document, "error").unwrap_or_default();
        match error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += SLOW_DOWN,
            _ => {
                return Err(Error::remote(
                    "sso-oidc",
                    "CreateToken",
                    status,
                    if error.is_empty() {
                        "CreateTokenFailed".to_owned()
                    } else {
                        error
                    },
                    text(&document, "error_description").unwrap_or_default(),
                    &sso.start_url,
                ));
            }
        }
        if SystemTime::now() >= deadline {
            return Err(refusal(format!(
                "the sign-in at {} was not confirmed before it expired",
                authorization.verification_uri
            )));
        }
    }
}

/// Trade the sign-in's token for the role's keys at the access portal.
///
/// # Errors
///
/// The portal's refusal - a lapsed token answers 401 - the transport's
/// failure, or an answer without a credential set.
pub(crate) fn role_credentials(
    agent: &ureq::Agent,
    portal: &str,
    sso: &Sso,
    token: &Token,
) -> Result<Credentials> {
    let url = format!(
        "{portal}/federation/credentials?role_name={}&account_id={}",
        super::sigv4::encode_query_component(&sso.role_name),
        super::sigv4::encode_query_component(&sso.account_id)
    );
    let mut answer = agent
        .get(&url)
        .config()
        .timeout_global(Some(TIMEOUT))
        .build()
        .header("x-amz-sso_bearer_token", token.access_token.expose())
        .call()
        .map_err(|error| transport_failure(portal, &error))?;
    let status = answer.status().as_u16();
    let body = answer
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|error| transport_failure(portal, &error))?;
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    if status != 200 {
        return Err(Error::remote(
            "sso",
            "GetRoleCredentials",
            status,
            text(&document, "error")
                .or_else(|| text(&document, "code"))
                .unwrap_or_else(|| "GetRoleCredentialsFailed".to_owned()),
            text(&document, "error_description")
                .or_else(|| text(&document, "message"))
                .unwrap_or_default(),
            &sso.start_url,
        ));
    }
    let Some(found) = document.get("roleCredentials") else {
        return Err(Error::remote(
            "sso",
            "GetRoleCredentials",
            status,
            "MalformedAnswer",
            "the answer carried no roleCredentials",
            &sso.start_url,
        ));
    };
    let mut credentials = Credentials::new(
        required_text(found, "accessKeyId", "GetRoleCredentials")?,
        required_text(found, "secretAccessKey", "GetRoleCredentials")?,
    )
    .with_session_token(text(found, "sessionToken").unwrap_or_default())
    .with_account_id(sso.account_id.clone());
    if let Some(expiry) = found
        .get("expiration")
        .and_then(serde_json::Value::as_i64)
        .and_then(instant_from_millis)
    {
        credentials = credentials.with_expiry(expiry);
    }
    Ok(credentials)
}

/// Send one JSON request and read a JSON answer, refusing a non-2xx status.
fn post_json(
    agent: &ureq::Agent,
    url: &str,
    body: &serde_json::Value,
    operation: &'static str,
    path: &str,
) -> Result<serde_json::Value> {
    let (status, document) = post(agent, url, body, path)?;
    if status >= 300 {
        return Err(Error::remote(
            "sso-oidc",
            operation,
            status,
            text(&document, "error").unwrap_or_else(|| format!("{operation}Failed")),
            text(&document, "error_description").unwrap_or_default(),
            path,
        ));
    }
    Ok(document)
}

/// Send one JSON request and read the status and the JSON it answered.
fn post(
    agent: &ureq::Agent,
    url: &str,
    body: &serde_json::Value,
    path: &str,
) -> Result<(u16, serde_json::Value)> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let answered = agent
            .post(url)
            .config()
            .timeout_global(Some(TIMEOUT))
            .build()
            .header("Content-Type", "application/json")
            .send(body.to_string().as_bytes())
            .and_then(|mut answer| {
                let status = answer.status().as_u16();
                answer
                    .body_mut()
                    .with_config()
                    .limit(MAX_ANSWER)
                    .read_to_vec()
                    .map(|bytes| (status, bytes))
            });
        match answered {
            Ok((status, _)) if (status >= 500 || status == 429) && attempt < ATTEMPTS => {
                std::thread::sleep(RETRY_PAUSE);
            }
            Ok((status, bytes)) => {
                return Ok((status, serde_json::from_slice(&bytes).unwrap_or_default()));
            }
            Err(_) if attempt < ATTEMPTS => std::thread::sleep(RETRY_PAUSE),
            Err(error) => return Err(transport_failure(path, &error)),
        }
    }
}

fn text(document: &serde_json::Value, key: &str) -> Option<String> {
    document
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_text(document: &serde_json::Value, key: &str, operation: &str) -> Result<String> {
    text(document, key).ok_or_else(|| refusal(format!("{operation} answered without {key}")))
}

fn seconds(document: &serde_json::Value, key: &str) -> Option<Duration> {
    document
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map(Duration::from_secs)
}

/// When a token obtained at `now` with `lifetime` lapses: an hour when the
/// service said nothing, and never past the longest lifetime believed.
fn lapse(now: SystemTime, lifetime: Option<Duration>) -> SystemTime {
    let lifetime = lifetime
        .unwrap_or(Duration::from_secs(3600))
        .min(MAX_LIFETIME);
    now.checked_add(lifetime).unwrap_or(now)
}

fn transport_failure(endpoint: &str, error: &impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        format!("could not reach IAM Identity Center through {endpoint}: {error}"),
    ))
}

fn refusal(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message.into(),
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/sso.rs` pins and a caller cannot reach.
    //!
    //! The sign-in is driven over a socket by the suite; what is pinned here
    //! is the cache the CLI shares - its file names and its document - and
    //! the windows a token moves through. Each item forwards.
    pub use super::Token;

    use std::time::SystemTime;

    use crate::aws::Sso;

    /// The cache file the sign-in's token is filed under.
    pub fn token_cache_key(sso: &Sso) -> String {
        sso.token_cache_key()
    }

    /// The CLI cache file the role's keys are filed under.
    pub fn credentials_cache_key(sso: &Sso) -> String {
        sso.credentials_cache_key()
    }

    /// Whether `token` lapses within the refresh window.
    pub fn is_stale(token: &Token, now: SystemTime) -> bool {
        token.is_stale(now)
    }

    /// Whether `token` can be refreshed at `now`.
    pub fn can_refresh(token: &Token, now: SystemTime) -> bool {
        token.can_refresh(now)
    }
}
