//! Where the bearer token that authorizes a Google request comes from.
//!
//! Google does not sign a request; it carries an OAuth 2.0 access token, and
//! the whole of this module is about obtaining one. Application Default
//! Credentials is a chain that stops at the first source with an answer, in the
//! order every Google client walks it: an explicit token, an explicit
//! credentials document, `GOOGLE_APPLICATION_CREDENTIALS`, the file `gcloud`
//! wrote, then the metadata server every instance has.
//!
//! A token expires, so it is obtained again shortly before it does rather than
//! per request, and nothing at all is read until the first request needs one.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;

use super::options::GoogleOptions;
use crate::{Error, Result, Scalar};

/// How long before a token lapses it is obtained again.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// The lifetime a signed assertion asks for; Google refuses more.
const ASSERTION_LIFETIME: u64 = 3600;
/// Where an assertion or a refresh token is exchanged, when a document names
/// no other.
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
/// The grant a signed service-account assertion uses.
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// Where an instance's own token is asked for.
const METADATA_HOST: &str = "metadata.google.internal";
/// The path under the metadata host that answers a token.
const METADATA_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
/// The header without which the metadata server refuses, which is what stops a
/// confused deputy from being asked for one.
const METADATA_FLAVOR: (&str, &str) = ("metadata-flavor", "Google");
/// Off a Google instance, resolving the metadata host can hang; a token is not
/// worth waiting on that long.
const METADATA_TIMEOUT: Duration = Duration::from_secs(2);
/// Where one identity is traded for another's.
const IAM_CREDENTIALS_HOST: &str = "https://iamcredentials.googleapis.com";
/// The largest token document read from any of these endpoints.
const MAX_ANSWER: u64 = 256 * 1024;

/// One access token, and when it stops being usable.
#[derive(Clone, Debug)]
pub(crate) struct Token {
    /// The bearer value, which is never rendered.
    value: String,
    /// When it lapses, when the answer said.
    expires_at: Option<SystemTime>,
}

impl Token {
    /// The bearer value.
    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    /// Whether this is close enough to lapsing to obtain another.
    fn is_stale(&self, now: SystemTime) -> bool {
        self.expires_at
            .is_some_and(|expiry| expiry <= now + REFRESH_MARGIN)
    }
}

/// A credentials document, in whichever of the shapes it was written.
#[derive(Clone)]
enum Credential {
    /// A service-account key: an email and an RSA private key to sign with.
    ServiceAccount {
        email: String,
        /// PKCS#8 DER, decoded from the document's PEM once.
        key: Vec<u8>,
        key_id: Option<String>,
        token_uri: String,
    },
    /// What `gcloud` writes: a refresh token and the client it belongs to.
    AuthorizedUser {
        client_id: String,
        client_secret: String,
        refresh_token: String,
        token_uri: String,
    },
    /// One identity used to obtain another's, which is what the document's
    /// `impersonated_service_account` type says.
    Impersonated {
        /// The `generateAccessToken` URL the document names.
        url: String,
        /// The identities the call is delegated through, in order.
        delegates: Vec<String>,
        /// The credential that authorizes the exchange.
        source: Box<Credential>,
    },
}

/// Where a token comes from, decided once and then asked repeatedly.
enum Source {
    /// A token the caller already holds; nothing is obtained or refreshed.
    Fixed(String),
    /// A credentials document, exchanged for a token.
    Document(Credential),
    /// The instance's own identity, asked of the metadata server.
    Metadata { host: String },
    /// Nothing authorizes these requests, which a public bucket allows.
    Anonymous,
}

/// The token this client authorizes with, obtained once and refreshed in time.
pub(crate) struct TokenCache {
    source: Source,
    scope: String,
    held: Mutex<Option<Token>>,
}

impl TokenCache {
    /// Decide where a token will come from, reading no network and no file.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a credentials document is named and unreadable,
    /// or is of a kind this client does not obtain a token for.
    pub(crate) fn new(options: &GoogleOptions, anonymous: bool, environment: bool) -> Result<Self> {
        let scope = options.scope().to_owned();
        let source = Self::resolve(options, anonymous, environment)?;
        Ok(Self {
            source,
            scope,
            held: Mutex::new(None),
        })
    }

    /// Walk the chain, stopping at the first source with an answer.
    fn resolve(options: &GoogleOptions, anonymous: bool, environment: bool) -> Result<Source> {
        if anonymous {
            return Ok(Source::Anonymous);
        }
        if let Some(token) = options.access_token() {
            return Ok(Source::Fixed(token.to_owned()));
        }
        let document = match (options.credentials_json(), options.credentials_file()) {
            (Some(json), _) => Some(json.to_owned()),
            (None, Some(path)) => Some(read_document(path)?),
            (None, None) if environment => well_known_document()?,
            (None, None) => None,
        };
        let credential = match document {
            Some(document) => Some(Credential::from_json(&document)?),
            None => None,
        };
        // Impersonation wraps whatever answered rather than replacing it: that
        // identity is what authorizes the exchange, and the token the exchange
        // returns is what reaches the store.
        let credential = match (options.impersonation(), credential) {
            (Some(target), Some(source)) => Some(Credential::Impersonated {
                url: impersonation_url(target),
                delegates: Vec::new(),
                source: Box::new(source),
            }),
            (_, held) => held,
        };
        Ok(match credential {
            Some(credential) => Source::Document(credential),
            None => Source::Metadata {
                host: options
                    .metadata_host()
                    .map(str::to_owned)
                    .or_else(|| environment.then(metadata_host).flatten())
                    .unwrap_or_else(|| METADATA_HOST.to_owned()),
            },
        })
    }

    /// The token to authorize with now, obtaining one if what is held lapses.
    ///
    /// `None` is anonymous, which is a valid way to reach a public bucket
    /// rather than a failure.
    ///
    /// # Errors
    ///
    /// Returns the refusal of whichever endpoint was asked.
    pub(crate) fn resolve_token(
        &self,
        agent: &ureq::Agent,
        now: SystemTime,
    ) -> Result<Option<Token>> {
        if matches!(self.source, Source::Anonymous) {
            return Ok(None);
        }
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(token) = held.as_ref() {
            if !token.is_stale(now) {
                return Ok(Some(token.clone()));
            }
        }
        let fresh = self.obtain(agent, now)?;
        *held = Some(fresh.clone());
        drop::<MutexGuard<'_, _>>(held);
        Ok(Some(fresh))
    }

    /// Ask this cache's source for a token.
    fn obtain(&self, agent: &ureq::Agent, now: SystemTime) -> Result<Token> {
        match &self.source {
            Source::Anonymous => unreachable!("an anonymous client asks for no token"),
            Source::Fixed(value) => Ok(Token {
                value: value.clone(),
                expires_at: None,
            }),
            Source::Metadata { host } => from_metadata(agent, host, now),
            Source::Document(credential) => credential.token(agent, &self.scope, now),
        }
    }
}

impl Credential {
    /// Read a credentials document, dispatching on its own `type`.
    fn from_json(document: &str) -> Result<Self> {
        let value = crate::text::json::from_utf8(document).map_err(|error| {
            refusal(&format!("expected a Google credentials document: {error}"))
        })?;
        Self::from_scalar(&value)
    }

    /// The same, over a document already parsed - which impersonation needs,
    /// because it nests one credential inside another.
    fn from_scalar(value: &Scalar) -> Result<Self> {
        let text = |name: &str| value.get_key_str(name).and_then(Scalar::as_str);
        let required = |name: &'static str| {
            text(name).ok_or_else(|| {
                refusal(&format!("expected {name} in a Google credentials document"))
            })
        };
        match text("type") {
            Some("service_account") => Ok(Self::ServiceAccount {
                email: required("client_email")?.to_owned(),
                key: pkcs8_of(required("private_key")?)?,
                key_id: text("private_key_id").map(str::to_owned),
                token_uri: text("token_uri").unwrap_or(DEFAULT_TOKEN_URI).to_owned(),
            }),
            Some("authorized_user") => Ok(Self::AuthorizedUser {
                client_id: required("client_id")?.to_owned(),
                client_secret: required("client_secret")?.to_owned(),
                refresh_token: required("refresh_token")?.to_owned(),
                token_uri: text("token_uri").unwrap_or(DEFAULT_TOKEN_URI).to_owned(),
            }),
            Some("impersonated_service_account") => {
                let source = value.get_key_str("source_credentials").ok_or_else(|| {
                    refusal("expected source_credentials in an impersonated credentials document")
                })?;
                Ok(Self::Impersonated {
                    url: required("service_account_impersonation_url")?.to_owned(),
                    delegates: value
                        .get_key_str("delegates")
                        .map(|list| {
                            list.sequence_iter()
                                .filter_map(Scalar::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                    source: Box::new(Self::from_scalar(source)?),
                })
            }
            // Named, understood, and not something this client obtains a token
            // for. Saying so beats a request that fails unauthorized later.
            Some(other) => Err(refusal(&format!(
                "a Google credentials document of type {other} is not one this client \
                 obtains a token from; name a service account, an authorized user, or \
                 an access token"
            ))),
            None => Err(refusal("expected a type in a Google credentials document")),
        }
    }

    /// Trade this credential for an access token.
    fn token(&self, agent: &ureq::Agent, scope: &str, now: SystemTime) -> Result<Token> {
        match self {
            Self::ServiceAccount {
                email,
                key,
                key_id,
                token_uri,
            } => {
                let assertion = assertion(email, key, key_id.as_deref(), token_uri, scope, now)?;
                let body = form(&[("grant_type", JWT_BEARER_GRANT), ("assertion", &assertion)]);
                exchange(agent, token_uri, &body, now)
            }
            Self::AuthorizedUser {
                client_id,
                client_secret,
                refresh_token,
                token_uri,
            } => {
                // No scope is sent: a refresh token holds the scopes it was
                // granted, and naming a wider one is refused outright.
                let body = form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", client_id),
                    ("client_secret", client_secret),
                    ("refresh_token", refresh_token),
                ]);
                exchange(agent, token_uri, &body, now)
            }
            Self::Impersonated {
                url,
                delegates,
                source,
            } => {
                let held = source.token(agent, scope, now)?;
                // The body is rendered by the core JSON codec, so nothing
                // here escapes a string for itself.
                let body = crate::text::json::into_utf8(&Scalar::from_record([
                    (
                        "delegates",
                        Scalar::from_sequence(
                            delegates.iter().map(|name| Scalar::from(name.as_str())),
                        ),
                    ),
                    ("scope", Scalar::from_sequence([Scalar::from(scope)])),
                    (
                        "lifetime",
                        Scalar::from(format!("{ASSERTION_LIFETIME}s").as_str()),
                    ),
                ])?)?;
                let answer = post(
                    agent,
                    url,
                    "application/json",
                    body.as_bytes(),
                    &[("authorization", &format!("Bearer {}", held.value))],
                )?;
                // The exchange answers an RFC 3339 instant rather than a
                // lifetime, which is the one place these two differ.
                let value = crate::text::json::from_bytes(&answer)
                    .map_err(|error| refusal(&format!("expected an access token: {error}")))?;
                let token = value
                    .get_key_str("accessToken")
                    .and_then(Scalar::as_str)
                    .ok_or_else(|| refusal("expected accessToken in the impersonation answer"))?;
                Ok(Token {
                    value: token.to_owned(),
                    expires_at: value
                        .get_key_str("expireTime")
                        .and_then(Scalar::as_str)
                        .and_then(instant_of),
                })
            }
        }
    }
}

/// The JWT a service-account key signs to ask for a token.
///
/// Header and claims are rendered once and signed as the exact bytes that go
/// on the wire, so no re-serialization can change what was signed.
fn assertion(
    email: &str,
    key: &[u8],
    key_id: Option<&str>,
    audience: &str,
    scope: &str,
    now: SystemTime,
) -> Result<String> {
    let issued = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| refusal("expected a clock reading at or after the Unix epoch"))?
        .as_secs();
    let mut header = vec![("alg", Scalar::from("RS256")), ("typ", Scalar::from("JWT"))];
    if let Some(key_id) = key_id {
        header.push(("kid", Scalar::from(key_id)));
    }
    let header = crate::text::json::into_utf8(&Scalar::from_record(header)?)?;
    let claims = crate::text::json::into_utf8(&Scalar::from_record([
        ("iss", Scalar::from(email)),
        ("scope", Scalar::from(scope)),
        ("aud", Scalar::from(audience)),
        (
            "iat",
            Scalar::from(i64::try_from(issued).unwrap_or(i64::MAX)),
        ),
        (
            "exp",
            Scalar::from(i64::try_from(issued + ASSERTION_LIFETIME).unwrap_or(i64::MAX)),
        ),
    ])?)?;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let signing_input = format!("{}.{}", engine.encode(header), engine.encode(claims));
    let signature = sign_rs256(key, signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", engine.encode(signature)))
}

/// RSASSA-PKCS1-v1_5 over SHA-256, with the PKCS#8 key `key`.
fn sign_rs256(key: &[u8], message: &[u8]) -> Result<Vec<u8>> {
    let pair = ring::signature::RsaKeyPair::from_pkcs8(key)
        .map_err(|error| refusal(&format!("expected an RSA private key: {error}")))?;
    let mut signature = vec![0; pair.public().modulus_len()];
    pair.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        message,
        &mut signature,
    )
    .map_err(|error| refusal(&format!("could not sign the assertion: {error}")))?;
    Ok(signature)
}

/// The DER a PEM private key wraps.
fn pkcs8_of(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .flat_map(str::chars)
        .filter(|character| !character.is_whitespace())
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|error| refusal(&format!("expected a PEM-wrapped private key: {error}")))
}

/// Exchange a form body at a token endpoint.
fn exchange(agent: &ureq::Agent, url: &str, body: &str, now: SystemTime) -> Result<Token> {
    let answer = post(
        agent,
        url,
        "application/x-www-form-urlencoded",
        body.as_bytes(),
        &[],
    )?;
    read_token(&answer, now)
}

/// The instance's own token, from the metadata server.
fn from_metadata(agent: &ureq::Agent, host: &str, now: SystemTime) -> Result<Token> {
    // Plain HTTP by design: the link-local address is the authority, and the
    // header is what a confused deputy cannot forge on the instance's behalf.
    let url = format!("http://{host}{METADATA_PATH}");
    let mut response = agent
        .get(&url)
        .header(METADATA_FLAVOR.0, METADATA_FLAVOR.1)
        .config()
        .timeout_global(Some(METADATA_TIMEOUT))
        .build()
        .call()
        .map_err(|error| {
            refusal(&format!(
                "the Google metadata server at {host} answered no token: {error}"
            ))
        })?;
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|error| refusal(&format!("could not read the metadata answer: {error}")))?;
    read_token(&body, now)
}

/// Read `{"access_token": ..., "expires_in": ...}`.
fn read_token(body: &[u8], now: SystemTime) -> Result<Token> {
    let value = crate::text::json::from_bytes(body)
        .map_err(|error| refusal(&format!("expected an access token: {error}")))?;
    let token = value
        .get_key_str("access_token")
        .and_then(Scalar::as_str)
        .ok_or_else(|| refusal("expected access_token in the token answer"))?;
    let lifetime = value
        .get_key_str("expires_in")
        .and_then(|value| value.as_str().and_then(|text| text.parse::<u64>().ok()));
    Ok(Token {
        value: token.to_owned(),
        expires_at: lifetime.map(|seconds| now + Duration::from_secs(seconds)),
    })
}

/// One `POST`, reading a bounded answer and turning a refusal into an error.
fn post(
    agent: &ureq::Agent,
    url: &str,
    content_type: &str,
    body: &[u8],
    headers: &[(&str, &str)],
) -> Result<Vec<u8>> {
    let mut request = agent.post(url).header("content-type", content_type);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request
        .send(body)
        .map_err(|error| refusal(&format!("the token endpoint {url} refused: {error}")))?;
    let status = response.status().as_u16();
    let answer = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|error| refusal(&format!("could not read the token answer: {error}")))?;
    if status >= 300 {
        let described = String::from_utf8_lossy(&answer);
        let described = described.lines().next().unwrap_or_default();
        return Err(Error::remote(
            "gs",
            "GetAccessToken",
            status,
            "TokenError",
            described,
            url,
        ));
    }
    Ok(answer)
}

/// The impersonation endpoint for `target`, when a caller named an email
/// rather than the whole URL.
fn impersonation_url(target: &str) -> String {
    if target.starts_with("https://") {
        return target.to_owned();
    }
    format!(
        "{IAM_CREDENTIALS_HOST}/v1/projects/-/serviceAccounts/{}:generateAccessToken",
        crate::uri::percent_encode_segment(target)
    )
}

/// A form body, percent-encoded as one.
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                super::super::sigv4::encode_query_component(name),
                super::super::sigv4::encode_query_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Read one RFC 3339 instant in UTC, which is how an impersonated token states
/// its expiry.
fn instant_of(text: &str) -> Option<SystemTime> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let mut time = time.splitn(3, ':');
    let hour: u64 = time.next()?.parse().ok()?;
    let minute: u64 = time.next()?.parse().ok()?;
    let second: f64 = time.next()?.parse().ok()?;
    let days = super::super::aws::credentials::days_from_civil(year, month, day);
    let seconds = days.checked_mul(86_400)? + (hour * 3600 + minute * 60) as i64 + second as i64;
    u64::try_from(seconds)
        .ok()
        .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds))
}

/// The credentials document the environment names, when it names one.
fn well_known_document() -> Result<Option<String>> {
    if let Some(path) = variable("GOOGLE_APPLICATION_CREDENTIALS") {
        return read_document(std::path::Path::new(&path)).map(Some);
    }
    let Some(path) = well_known_path() else {
        return Ok(None);
    };
    // The file `gcloud` writes is a default rather than an instruction, so its
    // absence is the next step in the chain rather than a refusal.
    match std::fs::read_to_string(&path) {
        Ok(document) => Ok(Some(document)),
        Err(_) => Ok(None),
    }
}

/// Where `gcloud auth application-default login` writes its document.
fn well_known_path() -> Option<std::path::PathBuf> {
    if let Some(configured) = variable("CLOUDSDK_CONFIG") {
        return Some(
            std::path::PathBuf::from(configured).join("application_default_credentials.json"),
        );
    }
    if cfg!(windows) {
        return variable("APPDATA").map(|base| {
            std::path::PathBuf::from(base)
                .join("gcloud")
                .join("application_default_credentials.json")
        });
    }
    crate::holder::local::Folder::config().ok().map(|config| {
        config
            .path()
            .unwrap_or_default()
            .join("gcloud")
            .join("application_default_credentials.json")
    })
}

/// The metadata host the environment names, when it names one.
fn metadata_host() -> Option<String> {
    variable("GCE_METADATA_HOST").or_else(|| variable("GCE_METADATA_ROOT"))
}

/// Read a credentials document from disk, naming the file in any refusal.
fn read_document(path: &std::path::Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|error| {
        refusal(&format!(
            "could not read the Google credentials file {}: {error}",
            path.display()
        ))
    })
}

/// One environment variable, empty read as unset.
fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Refuse something about obtaining a token.
fn refusal(message: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.to_owned(),
    ))
}

/// The token is a secret, so it is named rather than rendered.
impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ServiceAccount { .. } => "ServiceAccount(<redacted>)",
            Self::AuthorizedUser { .. } => "AuthorizedUser(<redacted>)",
            Self::Impersonated { .. } => "Impersonated(<redacted>)",
        })
    }
}
