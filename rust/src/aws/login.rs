//! The AWS Console sign-in, which `aws login` files and this module
//! refreshes.
//!
//! `aws login` (AWS CLI v2.32 and later) signs a developer in with the
//! identity they use in the AWS Management Console, and files what the
//! sign-in produced under `~/.aws/login/cache` - or the directory
//! `AWS_LOGIN_CACHE_DIRECTORY` names - in a file named by the SHA-256 of the
//! session's ARN: a credential set that lasts fifteen minutes, the refresh
//! token that obtains the next one, and the P-256 key that refresh token is
//! bound to. A profile names the sign-in with `login_session`.
//!
//! Any process on the machine signs with the cached set while it lasts and,
//! five minutes before it lapses, trades the refresh token for the next set
//! at the AWS Sign-In service: an OAuth 2.0 `refresh_token` grant that is not
//! signed, carrying instead a `DPoP` proof (RFC 9449) - a compact JWS over
//! ES256, by that key, naming the request it rides on - so a refresh token
//! copied off the machine is worth nothing without the key beside it. The
//! refreshed set and the rotated refresh token are written back over the
//! cached document, every field the refresh does not replace kept as it was
//! read, so the CLI and this crate share one sign-in.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::rand::{SecureRandom as _, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};
use serde_json::{Map, Value};

use super::credentials::Credentials;
use crate::auth::lease::lapses_within;
use crate::auth::{Secret, instant, iso8601, write_private};
use crate::{Error, Result};

/// A cached set lapsing within this is refreshed: the window the AWS tools
/// keep for a sign-in whose sets last fifteen minutes.
pub(crate) const REFRESH_WINDOW: Duration = Duration::from_secs(5 * 60);
/// The service and the operation a refusal names.
const SERVICE: &str = "signin";
const OPERATION: &str = "CreateOAuth2Token";
/// The OAuth 2.0 grant a refresh is.
const GRANT: &str = "refresh_token";
/// Where the operation is answered under the service's endpoint.
const TOKEN_PATH: &str = "/v1/token";
/// The largest cache document or answer read; a sign-in is a few kilobytes.
const MAX_DOCUMENT: u64 = 256 * 1024;
/// The bound on one request to the service.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The longest lifetime an answer's `expiresIn` is believed; the service
/// states at most fifteen minutes, and a bigger one is a document nobody
/// meant.
const MAX_LIFETIME: Duration = Duration::from_secs(12 * 3600);
/// Attempts at one refresh before its failure is the answer: a refresh is
/// idempotent while its token stands, so a throttle, a server failure or a
/// transport failure is tried again, after a short pause.
const ATTEMPTS: u32 = 3;

/// The DER tags the SEC1 key is read by.
const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
const OBJECT: u8 = 0x06;
const PARAMETERS: u8 = 0xa0;
const PUBLIC_KEY: u8 = 0xa1;
/// `prime256v1`, 1.2.840.10045.3.1.7: the one curve a proof is signed on.
const P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
/// The length of a P-256 scalar, and of an uncompressed P-256 point.
const SCALAR_LEN: usize = 32;
const POINT_LEN: usize = 1 + 2 * SCALAR_LEN;

/// The Sign-In service a sign-in made in `region` is refreshed at, as the
/// service's endpoint rules spell it for an operation that is not the
/// control plane's: `{region}.signin.<partition host>`; under FIPS
/// `signin-fips.{region}.<dns suffix>`, but `signin-fips.amazonaws-us-gov.com`
/// for `us-gov-west-1` and `{region}.signin-fips.amazonaws-us-gov.com` for
/// the rest of GovCloud; dual-stack `signin.{region}.<dual-stack suffix>`,
/// and both `signin-fips.{region}.<dual-stack suffix>` - the suffixes the
/// region's [`ArnPartition`](crate::ArnPartition) states.
///
/// # Errors
///
/// A region that is not one host label: it chooses the host a refresh
/// token is posted to, so nothing else may.
pub(crate) fn endpoint(
    region: &str,
    fips: bool,
    dualstack: bool,
) -> std::result::Result<String, String> {
    use crate::ArnPartition;

    let region = region.trim();
    let label = !region.is_empty()
        && region.len() <= 63
        && !region.starts_with('-')
        && !region.ends_with('-')
        && region
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !label {
        return Err(format!(
            "the region {region:?} is not one host label, so it names no Sign-In service"
        ));
    }
    let partition = ArnPartition::from_region(region);
    Ok(match (fips, dualstack) {
        (false, false) => {
            // The Sign-In service keeps hosts of its own beside each
            // partition's DNS suffix.
            let host = match partition {
                ArnPartition::Aws => "signin.aws.amazon.com",
                ArnPartition::AwsCn => "signin.amazonaws.cn",
                ArnPartition::AwsUsGov => "signin.amazonaws-us-gov.com",
                ArnPartition::AwsIso => "signin.c2shome.ic.gov",
                ArnPartition::AwsIsoB => "signin.sc2shome.sgov.gov",
                ArnPartition::AwsIsoE => "signin.csphome.adc-e.uk",
                ArnPartition::AwsIsoF => "signin.csphome.hci.ic.gov",
                ArnPartition::AwsEusc => "signin.amazonaws-eusc.eu",
            };
            format!("https://{region}.{host}")
        }
        (true, false) if region == "us-gov-west-1" => {
            "https://signin-fips.amazonaws-us-gov.com".to_owned()
        }
        (true, false) if partition == ArnPartition::AwsUsGov => {
            format!("https://{region}.signin-fips.amazonaws-us-gov.com")
        }
        (true, false) => format!("https://signin-fips.{region}.{}", partition.dns_suffix()),
        (true, true) => format!(
            "https://signin-fips.{region}.{}",
            partition.dualstack_dns_suffix()
        ),
        (false, true) => format!(
            "https://signin.{region}.{}",
            partition.dualstack_dns_suffix()
        ),
    })
}

/// Where `aws login` files the sign-in `login_session` names: the lowercase
/// hex SHA-256 of the session, its surrounding blanks trimmed as the AWS
/// tools trim a profile's value.
pub(crate) fn cache_path(cache_directory: &Path, login_session: &str) -> PathBuf {
    cache_directory.join(format!(
        "{}.json",
        super::sigv4::sha256_hex(login_session.trim().as_bytes())
    ))
}

/// The credential set the sign-in `login_session` names, as it stands at
/// `now`: the cached set, with its expiry and its account, while it lasts
/// beyond [`REFRESH_WINDOW`] and `is_refused` does not say a store refused
/// its key; else the set a refresh at `endpoint` obtains, filed back in the
/// cache under `cache_directory` before it is answered.
///
/// A refresh that fails while the cached set still stands answers that set
/// and says why in the log, so no request is refused over a set the
/// service still accepts; the session's lease decides when to try again.
///
/// # Errors
///
/// A refusal naming the session, the cause, and `aws login --profile
/// <profile>` as the way out: no sign-in is filed, its document is not one,
/// or the set has lapsed and the refresh failed - the service refused it,
/// could not be reached, or the key the proof is signed with is not a P-256
/// key pair.
pub(crate) fn credentials(
    http: &crate::http::Session,
    endpoint: &str,
    cache_directory: &Path,
    login_session: &str,
    profile: &str,
    is_refused: &dyn Fn(&str) -> bool,
    now: SystemTime,
) -> Result<Credentials> {
    let path = cache_path(cache_directory, login_session);
    let signin = Signin {
        session: login_session.trim(),
        profile,
        path: &path,
    };
    let held = Token::read(signin)?;
    // A set a store refused is refreshed whatever its expiry says; one
    // another process already replaced in the file is not.
    let refused = is_refused(held.credentials.access_key_id());
    if !refused && !lapses_within(&held.credentials, now, REFRESH_WINDOW) {
        return Ok(held.credentials);
    }
    match refresh(http, endpoint, &held, signin, now) {
        Ok(fresh) => {
            if let Err(error) = write_private(&path, fresh.render().as_bytes()) {
                log::warn!(
                    "the refreshed AWS Console sign-in {} could not be filed at {}: {error}",
                    signin.session,
                    path.display()
                );
            }
            log::debug!(
                "refreshed the AWS Console sign-in {}, which now stands until {}",
                signin.session,
                fresh
                    .credentials
                    .expires_at()
                    .map(iso8601)
                    .unwrap_or_default()
            );
            Ok(fresh.credentials)
        }
        Err(error) if lapses_within(&held.credentials, now, Duration::ZERO) => Err(error),
        Err(error) => {
            log::warn!(
                "keeping the AWS Console sign-in {} in hand, which stands until {}: refreshing it failed: {error}",
                signin.session,
                held.credentials
                    .expires_at()
                    .map(iso8601)
                    .unwrap_or_default()
            );
            Ok(held.credentials)
        }
    }
}

/// Which sign-in a refusal is about, and the way out it names.
#[derive(Clone, Copy)]
struct Signin<'a> {
    /// The `login_session` the sign-in is filed under.
    session: &'a str,
    /// The profile `aws login` signs in again for.
    profile: &'a str,
    /// The cache file that holds it.
    path: &'a Path,
}

impl Signin<'_> {
    /// What to run to sign in again.
    fn way_out(self) -> String {
        format!("sign in again with `aws login --profile {}`", self.profile)
    }

    /// A refusal of the sign-in for `cause`, which reads after its name.
    fn refusal(self, cause: impl std::fmt::Display) -> Error {
        refusal(format!(
            "the AWS Console sign-in {} {cause}: {}",
            self.session,
            self.way_out()
        ))
    }

    /// A refusal of the document filed for the sign-in, for `cause`.
    fn defect(self, cause: impl std::fmt::Display) -> Error {
        self.refusal(format_args!("filed at {} {cause}", self.path.display()))
    }
}

/// One sign-in as the cache files it.
///
/// The document is kept whole, so a refresh writes back every field it does
/// not replace - the key, the client, the identity token, any field a later
/// CLI adds - exactly as it was read. `Debug` renders no secret: the
/// document, which repeats them, is not rendered at all.
#[derive(Clone)]
pub struct Token {
    document: Map<String, Value>,
    credentials: Credentials,
    refresh_token: Secret,
    client_id: String,
    dpop_key: Secret,
}

impl Token {
    /// The cached set, with its expiry and its account.
    #[cfg(feature = "internals")]
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// The client the sign-in was made for.
    #[cfg(feature = "internals")]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The cache document the AWS tools read this sign-in back from.
    pub fn render(&self) -> String {
        Value::Object(self.document.clone()).to_string()
    }

    /// The sign-in its cache file holds.
    fn read(signin: Signin<'_>) -> Result<Self> {
        let file = match std::fs::File::open(signin.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    signin.refusal(format_args!("is not filed at {}", signin.path.display()))
                );
            }
            Err(error) => return Err(signin.defect(format_args!("could not be read: {error}"))),
        };
        let mut bytes = Vec::new();
        file.take(MAX_DOCUMENT + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| signin.defect(format_args!("could not be read: {error}")))?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_DOCUMENT) {
            return Err(signin.defect(format_args!("is larger than {MAX_DOCUMENT} bytes")));
        }
        Self::parse(&bytes, signin)
    }

    /// The sign-in a cache document states: `accessToken` with its keys,
    /// session token and expiry, `refreshToken`, `dpopKey` and `clientId`
    /// required, the account taken when the set names one.
    fn parse(bytes: &[u8], signin: Signin<'_>) -> Result<Self> {
        // A JSON syntax error states a position, never the text it stopped
        // at, so it renders none of the secrets the document holds.
        let document = match serde_json::from_slice(bytes) {
            Ok(Value::Object(document)) => document,
            Ok(_) => return Err(signin.defect("is not a JSON object")),
            Err(error) => return Err(signin.defect(format_args!("is not JSON ({error})"))),
        };
        let missing = |field: &str| signin.defect(format_args!("lacks {field}"));
        let access = document
            .get("accessToken")
            .and_then(Value::as_object)
            .ok_or_else(|| missing("accessToken"))?;
        let refresh_token =
            text(document.get("refreshToken")).ok_or_else(|| missing("refreshToken"))?;
        let dpop_key = text(document.get("dpopKey")).ok_or_else(|| missing("dpopKey"))?;
        let client_id = text(document.get("clientId")).ok_or_else(|| missing("clientId"))?;
        let access_key_id =
            text(access.get("accessKeyId")).ok_or_else(|| missing("accessToken.accessKeyId"))?;
        let secret_access_key = text(access.get("secretAccessKey"))
            .ok_or_else(|| missing("accessToken.secretAccessKey"))?;
        let session_token =
            text(access.get("sessionToken")).ok_or_else(|| missing("accessToken.sessionToken"))?;
        let expires_at = text(access.get("expiresAt"))
            .ok_or_else(|| missing("accessToken.expiresAt"))
            .and_then(|stated| {
                instant(stated).ok_or_else(|| {
                    signin.defect("states an accessToken.expiresAt that is no instant")
                })
            })?;
        let mut credentials = Credentials::new(access_key_id, secret_access_key)
            .with_session_token(session_token)
            .with_expiry(expires_at);
        if let Some(account) = text(access.get("accountId")) {
            credentials = credentials.with_account_id(account);
        }
        Ok(Self {
            refresh_token: Secret::new(refresh_token),
            dpop_key: Secret::new(dpop_key),
            client_id: client_id.to_owned(),
            credentials,
            document,
        })
    }

    /// The sign-in a refresh at `now` answered with `answer`: the new set
    /// under the account the cached one named, lapsing `expiresIn` from
    /// `now`, and the rotated refresh token - the held one where the answer
    /// rotated none. The error is the field the answer lacked.
    fn refreshed(
        &self,
        answer: &Value,
        now: SystemTime,
    ) -> std::result::Result<Self, &'static str> {
        let access = answer.get("accessToken").ok_or("accessToken")?;
        let access_key_id = text(access.get("accessKeyId")).ok_or("accessToken.accessKeyId")?;
        let secret_access_key =
            text(access.get("secretAccessKey")).ok_or("accessToken.secretAccessKey")?;
        let session_token = text(access.get("sessionToken")).ok_or("accessToken.sessionToken")?;
        let lifetime = answer
            .get("expiresIn")
            .and_then(Value::as_u64)
            .ok_or("expiresIn")?;
        // The document states whole seconds; the set lapses at the instant
        // it states, so a set read back is the set answered.
        let expires_at = whole_seconds(
            now.checked_add(Duration::from_secs(lifetime).min(MAX_LIFETIME))
                .unwrap_or(now),
        );
        let refresh_token = text(answer.get("refreshToken"))
            .map_or_else(|| self.refresh_token.clone(), Secret::new);
        let account_id = self.credentials.account_id();

        let mut stated = Map::new();
        stated.insert("accessKeyId".to_owned(), access_key_id.into());
        stated.insert("secretAccessKey".to_owned(), secret_access_key.into());
        stated.insert("sessionToken".to_owned(), session_token.into());
        if let Some(account) = account_id {
            stated.insert("accountId".to_owned(), account.into());
        }
        stated.insert("expiresAt".to_owned(), iso8601(expires_at).into());
        let mut document = self.document.clone();
        document.insert("accessToken".to_owned(), Value::Object(stated));
        document.insert("refreshToken".to_owned(), refresh_token.expose().into());

        let mut credentials = Credentials::new(access_key_id, secret_access_key)
            .with_session_token(session_token)
            .with_expiry(expires_at);
        if let Some(account) = account_id {
            credentials = credentials.with_account_id(account);
        }
        Ok(Self {
            document,
            credentials,
            refresh_token,
            client_id: self.client_id.clone(),
            dpop_key: self.dpop_key.clone(),
        })
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Token")
            .field("credentials", &self.credentials)
            .field("client_id", &self.client_id)
            .field("refresh_token", &self.refresh_token)
            .field("dpop_key", &self.dpop_key)
            .finish_non_exhaustive()
    }
}

/// Trade the sign-in's refresh token for the next set: one unsigned request,
/// tried again as [`ATTEMPTS`] allows, each attempt carrying a proof of its
/// own.
fn refresh(
    http: &crate::http::Session,
    endpoint: &str,
    held: &Token,
    signin: Signin<'_>,
    now: SystemTime,
) -> Result<Token> {
    let pair = key_pair(held.dpop_key.expose())
        .map_err(|cause| signin.defect(format_args!("holds a DPoP key that {cause}")))?;
    let endpoint = endpoint.trim().trim_end_matches('/');
    let url = format!("{endpoint}{TOKEN_PATH}");
    let body = serde_json::json!({
        "clientId": held.client_id,
        "grantType": GRANT,
        "refreshToken": held.refresh_token.expose(),
    })
    .to_string();
    let (status, kind, answer) = exchange(http, &url, body, pair, signin, endpoint)?;
    let document: Value = serde_json::from_slice(&answer).unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(refused(status, kind, &document, signin));
    }
    held.refreshed(&document, now).map_err(|field| {
        Error::remote(
            SERVICE,
            OPERATION,
            status,
            "MalformedAnswer",
            format!("the answer carried no {field} - {}", signin.way_out()),
            signin.session,
        )
    })
}

/// Send the refresh, and read the status, the error type the service names
/// in `x-amzn-ErrorType` and the body of the answer it ends on.
///
/// A refresh is idempotent while its token stands, so the HTTP client tries
/// a throttle, a server failure or a transport failure again - each attempt
/// carrying a proof of its own, because the service refuses an identifier
/// it has seen and a proof names the instant it is sent at.
fn exchange(
    http: &crate::http::Session,
    url: &str,
    body: String,
    pair: EcdsaKeyPair,
    signin: Signin<'_>,
    endpoint: &str,
) -> Result<(u16, Option<String>, std::sync::Arc<[u8]>)> {
    let pair = std::sync::Arc::new(pair);
    let request = http
        .post(url, body)?
        .with_header("content-type", "application/json")?
        .with_header("accept", "application/json")?
        .with_timeout(TIMEOUT)
        .with_max_attempts(ATTEMPTS)
        .with_idempotent(true)
        .with_attempt_headers(move |attempt| {
            let proof = jti()
                .and_then(|jti| {
                    proof(
                        &pair,
                        &attempt.url().to_string(),
                        unix_seconds(SystemTime::now()),
                        &jti,
                    )
                })
                .map_err(|cause| {
                    Error::Io(std::io::Error::other(format!(
                        "could not prove its key: {cause}"
                    )))
                })?;
            let mut headers = crate::http::Headers::new();
            headers.insert("dpop", &proof)?;
            Ok(headers)
        });
    match super::Answer::of(&request) {
        Ok(answer) => Ok((answer.status, answer.error_type, answer.body)),
        Err(error) => Err(transport_failure(endpoint, signin, &error)),
    }
}

/// The refusal an answer that is not a success is: the service's OAuth 2.0
/// error code and message, after the way out that code calls for.
fn refused(status: u16, kind: Option<String>, document: &Value, signin: Signin<'_>) -> Error {
    // A code is a short word; anything else a body states there is no code,
    // so an answer cannot fill a refusal with whatever it likes.
    let is_code = |code: &str| {
        code.len() <= 64
            && code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    let code = text(document.get("error"))
        .filter(|code| is_code(code))
        .map(str::to_owned)
        .or(kind.filter(|kind| is_code(kind)))
        .unwrap_or_else(|| format!("{OPERATION}Failed"));
    let way_out = signin.way_out();
    let remedy = match code.as_str() {
        "TOKEN_EXPIRED" => format!("the sign-in has ended - {way_out}"),
        "USER_CREDENTIALS_CHANGED" => {
            format!("the password changed since the sign-in - {way_out}")
        }
        "INSUFFICIENT_PERMISSIONS" => format!(
            "the signed-in identity lacks the signin:CreateOAuth2Token permission - grant it, \
             or {way_out} as an identity that has it"
        ),
        _ => format!("the refresh was refused - {way_out} if it persists"),
    };
    // The remedy leads because a refusal keeps one line of the message, and
    // the service's own words are the part that may run long.
    let message = match text(document.get("message")).and_then(|said| said.lines().next()) {
        Some(said) => format!("{remedy} ({said})"),
        None => remedy,
    };
    Error::remote(SERVICE, OPERATION, status, code, message, signin.session)
}

/// A JSON field's text, when it holds text that is not empty.
fn text(field: Option<&Value>) -> Option<&str> {
    field
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// `instant` cut to the whole second it falls in.
fn whole_seconds(instant: SystemTime) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(unix_seconds(instant))
}

/// The whole seconds from the epoch to `instant`; zero before it.
fn unix_seconds(instant: SystemTime) -> u64 {
    instant
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The P-256 key pair a sign-in's refresh token is bound to, from the PEM
/// the cache files it as: SEC1 `EC PRIVATE KEY`, which `aws login` writes,
/// or PKCS#8 `PRIVATE KEY`. The error reads after "a key that".
fn key_pair(pem_text: &str) -> std::result::Result<EcdsaKeyPair, String> {
    let random = SystemRandom::new();
    let (label, der) = pem(pem_text)?;
    match label.as_str() {
        "EC PRIVATE KEY" => {
            let (scalar, point) = sec1(&der)?;
            EcdsaKeyPair::from_private_key_and_public_key(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &scalar,
                point,
                &random,
            )
            .map_err(|error| format!("is not a P-256 key pair ({error})"))
        }
        "PRIVATE KEY" => EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &random)
            .map_err(|error| format!("is not a PKCS#8 P-256 key pair ({error})")),
        other => Err(format!(
            "is a PEM {other}, where an EC PRIVATE KEY or a PRIVATE KEY is expected"
        )),
    }
}

/// The label a PEM block states, and the DER it wraps.
///
/// A literal `\n` - what a writer that escaped the document twice leaves -
/// reads as the line break it meant. A decoding failure names no byte of
/// the body, which is key material.
fn pem(text: &str) -> std::result::Result<(String, Vec<u8>), String> {
    const BEGIN: &str = "-----BEGIN ";
    let text = text.replace("\\n", "\n");
    let opened = text
        .find(BEGIN)
        .map(|start| &text[start + BEGIN.len()..])
        .ok_or("is not PEM: it has no BEGIN line")?;
    let (label, rest) = opened
        .split_once("-----")
        // RFC 7468 labels are capitals, digits and blanks: a damaged block
        // whose body sits where its label should is no label, and is never
        // named back.
        .filter(|(label, _)| {
            label.len() <= 64
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b' ')
        })
        .ok_or("is not PEM: its BEGIN line names no label")?;
    let end = format!("-----END {label}-----");
    let (body, _) = rest
        .split_once(&end)
        .ok_or_else(|| format!("is a PEM {label} with no END line"))?;
    let body: String = body
        .chars()
        .filter(|glyph| !glyph.is_whitespace())
        .collect();
    let der = STANDARD
        .decode(body)
        .map_err(|_| format!("is a PEM {label} whose body is not base64"))?;
    Ok((label.to_owned(), der))
}

/// The private scalar and the public point of a SEC1 `ECPrivateKey`
/// (RFC 5915), refused unless the curve it names, when it names one, is
/// P-256 - a key that names none is proven by the pair it must form with
/// its point.
fn sec1(der: &[u8]) -> std::result::Result<([u8; SCALAR_LEN], &[u8]), String> {
    const MALFORMED: &str = "is not a DER ECPrivateKey";
    let (tag, body, trailing) = element(der).ok_or(MALFORMED)?;
    if tag != SEQUENCE || !trailing.is_empty() {
        return Err(MALFORMED.to_owned());
    }
    let (tag, version, body) = element(body).ok_or(MALFORMED)?;
    if tag != INTEGER || version != [1] {
        return Err(format!("{MALFORMED} of version 1"));
    }
    let (tag, scalar, mut body) = element(body).ok_or(MALFORMED)?;
    if tag != OCTET_STRING || scalar.is_empty() || scalar.len() > SCALAR_LEN {
        return Err("holds no P-256 private scalar".to_owned());
    }
    let mut point = None;
    while !body.is_empty() {
        let (tag, contents, rest) = element(body).ok_or(MALFORMED)?;
        match (tag, element(contents)) {
            (PARAMETERS, Some((OBJECT, curve, []))) if curve != P256 => {
                return Err(format!(
                    "is on the curve {}, not on P-256 (1.2.840.10045.3.1.7)",
                    object_identifier(curve)
                ));
            }
            (PARAMETERS, Some((OBJECT, _, []))) => {}
            // A BIT STRING's first byte counts the unused bits of its last;
            // a point uses every one.
            (PUBLIC_KEY, Some((BIT_STRING, [0, stated @ ..], []))) => point = Some(stated),
            _ => return Err(MALFORMED.to_owned()),
        }
        body = rest;
    }
    // Signing needs the point, and ring derives none: `aws login` writes it.
    let point = point.ok_or("states no public point")?;
    if point.len() != POINT_LEN || point.first() != Some(&0x04) {
        return Err("states a public point that is not an uncompressed P-256 one".to_owned());
    }
    // RFC 5915 states the scalar at the curve's full width; a writer that
    // dropped its leading zeros is read as meaning them.
    let mut padded = [0_u8; SCALAR_LEN];
    padded[SCALAR_LEN - scalar.len()..].copy_from_slice(scalar);
    Ok((padded, point))
}

/// One DER element of `input`: its tag, its contents, and what follows it.
/// Only the definite lengths a key takes - up to 65535 bytes - are read.
fn element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = match first {
        0..=0x7f => (usize::from(first), rest),
        0x81 => {
            let (&length, rest) = rest.split_first()?;
            (usize::from(length), rest)
        }
        0x82 => {
            let (length, rest) = rest.split_first_chunk::<2>()?;
            (usize::from(u16::from_be_bytes(*length)), rest)
        }
        _ => return None,
    };
    let (contents, rest) = rest.split_at_checked(length)?;
    Some((tag, contents, rest))
}

/// The dotted spelling of an object identifier's DER contents.
fn object_identifier(contents: &[u8]) -> String {
    let mut arcs: Vec<u64> = Vec::new();
    let mut arc: u64 = 0;
    for &byte in contents {
        if arc > u64::MAX >> 7 {
            return "an unreadable object identifier".to_owned();
        }
        arc = (arc << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            arcs.push(arc);
            arc = 0;
        }
    }
    let Some((&first, rest)) = arcs.split_first() else {
        return "an empty object identifier".to_owned();
    };
    // The first encoded arc carries two: 40 times the first, plus the second.
    let (top, second) = match first {
        0..40 => (0, first),
        40..80 => (1, first - 40),
        _ => (2, first - 80),
    };
    let mut dotted = format!("{top}.{second}");
    for arc in rest {
        dotted.push('.');
        dotted.push_str(&arc.to_string());
    }
    dotted
}

/// The base64url `x` and `y` of the pair's public point, as its JWK states
/// them.
fn jwk(pair: &EcdsaKeyPair) -> (String, String) {
    // ring holds a P-256 public key as the uncompressed point,
    // 0x04 || x || y.
    let point = pair.public_key().as_ref();
    let coordinate = |range: std::ops::Range<usize>| {
        URL_SAFE_NO_PAD.encode(point.get(range).unwrap_or_default())
    };
    (
        coordinate(1..1 + SCALAR_LEN),
        coordinate(1 + SCALAR_LEN..POINT_LEN),
    )
}

/// A `DPoP` proof (RFC 9449) that the holder of `pair` sends `POST url`: a
/// compact JWS over ES256 whose header carries the public key and whose
/// claims name the request, the instant `issued` (Unix seconds) and the
/// proof's own identifier `jti`.
fn proof(
    pair: &EcdsaKeyPair,
    url: &str,
    issued: u64,
    jti: &str,
) -> std::result::Result<String, String> {
    let (x, y) = jwk(pair);
    let header = serde_json::json!({
        "typ": "dpop+jwt",
        "alg": "ES256",
        "jwk": {"kty": "EC", "x": x, "y": y, "crv": "P-256"},
    });
    let claims = serde_json::json!({"htm": "POST", "htu": url, "iat": issued, "jti": jti});
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    // The FIXED signing algorithm answers what JWS wants: r and s, 32 bytes
    // each, rather than a DER sequence.
    let signature = pair
        .sign(&SystemRandom::new(), input.as_bytes())
        .map_err(|_| "the key could not sign the proof".to_owned())?;
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    ))
}

/// A fresh proof identifier: a random UUID, version 4.
fn jti() -> std::result::Result<String, String> {
    let mut bytes = [0_u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "the system's random source failed".to_owned())?;
    // RFC 9562's version and variant bits; the other 122 stay random.
    let random = u128::from_be_bytes(bytes);
    let stamped =
        (random & !(0xf_u128 << 76) & !(0x3_u128 << 62)) | (0x4_u128 << 76) | (0x2_u128 << 62);
    Ok(crate::Uuid::new(stamped).to_string())
}

/// Report a failure to reach the service at all.
fn transport_failure(endpoint: &str, signin: Signin<'_>, error: &impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        format!(
            "could not reach the AWS Sign-In service at {endpoint} to refresh the AWS Console \
             sign-in {}: {error}; try again once it answers, or {}",
            signin.session,
            signin.way_out()
        ),
    ))
}

fn refusal(message: String) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message,
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/login.rs` pins and a caller cannot reach.
    //!
    //! The refresh is driven over a socket, against the identity fake,
    //! through [`credentials`]; the cache's file names, its document, the key
    //! a sign-in is bound to and the proof that key signs are pinned by
    //! value. Each item forwards.
    pub use super::Token;

    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use crate::Result;
    use crate::aws::Credentials;

    /// A cached set lapsing within this is refreshed.
    pub const REFRESH_WINDOW: Duration = super::REFRESH_WINDOW;

    /// The Sign-In service a sign-in made in `region` is refreshed at.
    ///
    /// # Errors
    ///
    /// A region that is not one host label, or FIPS or dual-stack where no
    /// host is spelled.
    pub fn endpoint(
        region: &str,
        fips: bool,
        dualstack: bool,
    ) -> std::result::Result<String, String> {
        super::endpoint(region, fips, dualstack)
    }

    /// Where `aws login` files the sign-in `login_session` names.
    pub fn cache_path(cache_directory: &Path, login_session: &str) -> PathBuf {
        super::cache_path(cache_directory, login_session)
    }

    /// The credential set the sign-in `login_session` names at `now`, a
    /// refresh sent to `endpoint` through a client of default settings.
    ///
    /// # Errors
    ///
    /// A refusal naming the session, the cause and the way out.
    pub fn credentials(
        endpoint: &str,
        cache_directory: &Path,
        login_session: &str,
        profile: &str,
        now: SystemTime,
    ) -> Result<Credentials> {
        super::credentials(
            crate::aws::Session::new().with_environment(false).http()?,
            endpoint,
            cache_directory,
            login_session,
            profile,
            &|_| false,
            now,
        )
    }

    /// [`credentials`], with the store having refused the key `refused`.
    ///
    /// # Errors
    ///
    /// As [`credentials`].
    pub fn credentials_refusing(
        endpoint: &str,
        cache_directory: &Path,
        login_session: &str,
        profile: &str,
        refused: &str,
        now: SystemTime,
    ) -> Result<Credentials> {
        super::credentials(
            crate::aws::Session::new().with_environment(false).http()?,
            endpoint,
            cache_directory,
            login_session,
            profile,
            &|key| key == refused,
            now,
        )
    }

    /// The sign-in a cache document filed at `path` states.
    ///
    /// # Errors
    ///
    /// A document that is not JSON, or that lacks a field, named.
    pub fn parse(bytes: &[u8], login_session: &str, profile: &str, path: &Path) -> Result<Token> {
        super::Token::parse(
            bytes,
            super::Signin {
                session: login_session,
                profile,
                path,
            },
        )
    }

    /// The base64url `x` and `y` of the public point of the key `pem` holds.
    ///
    /// # Errors
    ///
    /// Why the key is not a P-256 key pair a proof is signed with.
    pub fn jwk(pem: &str) -> std::result::Result<(String, String), String> {
        super::key_pair(pem).map(|pair| super::jwk(&pair))
    }

    /// The proof the key `pem` holds signs for `POST url` at `issued`, under
    /// the identifier `jti`.
    ///
    /// # Errors
    ///
    /// As [`jwk`], or a signature the key could not make.
    pub fn proof(
        pem: &str,
        url: &str,
        issued: u64,
        jti: &str,
    ) -> std::result::Result<String, String> {
        super::proof(&super::key_pair(pem)?, url, issued, jti)
    }

    /// A fresh proof identifier.
    ///
    /// # Errors
    ///
    /// The system's random source failing.
    pub fn jti() -> std::result::Result<String, String> {
        super::jti()
    }
}
