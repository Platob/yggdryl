//! The EC2 instance metadata service, which serves an instance its own role.
//!
//! Every EC2 instance, and every service built on one, answers on a
//! link-local address: a session token over IMDSv2, the name of the role the
//! instance profile carries, that role's current keys, and the instance's
//! own region. Off an instance the address takes no connection, which is
//! read as "not an instance"; a token whose answer never comes back within
//! the one short timeout the AWS tools also wait - a hop limit of one drops
//! it on its way into a container - is a token not issued, and the reads
//! follow without one. Only a service that answered and then failed is a
//! failure.

use std::time::{Duration, SystemTime};

use super::credentials::{self, Credentials};
use crate::auth::iso8601;
use crate::{Error, Result};

/// Where the service lives when nothing names another address.
pub(crate) const IPV4_ENDPOINT: &str = "http://169.254.169.254";
/// Where it lives on an IPv6-only instance.
pub(crate) const IPV6_ENDPOINT: &str = "http://[fd00:ec2::254]";
/// How long an IMDSv2 session token is asked to last.
const TOKEN_TTL: &str = "21600";
/// The one path that answers the instance's own region.
const IDENTITY_DOCUMENT: &str = "/latest/dynamic/instance-identity/document";
/// Under this path, the role name, then the role's keys.
const SECURITY_CREDENTIALS: &str = "/latest/meta-data/iam/security-credentials/";
/// How far past now a lapsed set the service still serves is carried:
/// botocore's `ec2_credential_refresh_window`.
const STALE_EXTENSION: Duration = Duration::from_secs(600);
/// The least jitter added to [`STALE_EXTENSION`], in seconds.
const STALE_JITTER_MIN: u64 = 120;
/// The most jitter added to [`STALE_EXTENSION`], in seconds.
const STALE_JITTER_MAX: u64 = 600;

/// How the service is reached: the answers to the five knobs the AWS tools
/// read for it.
#[derive(Clone, Debug)]
pub(crate) struct Imds {
    /// The service's base URL, without a trailing slash.
    pub(crate) endpoint: String,
    /// The bound on each request.
    pub(crate) timeout: Duration,
    /// Attempts before the service is taken to be absent.
    pub(crate) attempts: u32,
    /// Whether a request without a session token may be sent when the
    /// service refuses to issue one.
    pub(crate) v1_allowed: bool,
}

impl Default for Imds {
    fn default() -> Self {
        Self {
            endpoint: IPV4_ENDPOINT.to_owned(),
            timeout: Duration::from_secs(1),
            attempts: 1,
            v1_allowed: true,
        }
    }
}

/// The service answered, and what it answered was a failure of its own.
fn answered(status: u16) -> Error {
    Error::Io(std::io::Error::other(format!(
        "the instance metadata service answered {status}"
    )))
}

/// Whether `error` is a deadline that ran out before an answer's head -
/// the connection's or the answer's, which the client does not tell apart.
fn timed_out(error: &Error) -> bool {
    matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut)
}

/// What asking for a session token established.
enum Reach {
    /// A token, to send with every read.
    Token(String),
    /// No token was issued - the service declined, or its answer did not
    /// come back in time - and the reads may go out as IMDSv1.
    NoToken,
    /// The service refused the request itself (`400`): no read follows,
    /// and the instance has no credentials to give.
    Refused,
    /// Nothing answered - the connection was refused, unreachable, or hung
    /// up on before any answer: this is not an instance.
    Unreachable,
}

impl Imds {
    /// The keys of the role the instance profile carries, read at `now`.
    ///
    /// `None` when the service does not answer, refuses the token request,
    /// issues no token while IMDSv1 is disabled, or answers that the instance
    /// has no role. A set whose expiry is already past at `now` is what the
    /// service serves while its credential service is unavailable, and is
    /// carried to `now` plus ten minutes and a jitter of two to ten more, as
    /// botocore carries it, with a warning naming it - so the instance keeps
    /// signing through the outage and asks again before the new expiry.
    ///
    /// # Errors
    ///
    /// A service that answered and then failed, or answered something that is
    /// not a credential document.
    pub(crate) fn credentials(
        &self,
        http: &crate::http::Session,
        now: SystemTime,
    ) -> Result<Option<Credentials>> {
        let token = match self.reach(http) {
            Reach::Token(token) => Some(token),
            Reach::NoToken if self.v1_allowed => None,
            Reach::NoToken | Reach::Refused | Reach::Unreachable => return Ok(None),
        };
        let Some((status, body)) = self.get(http, SECURITY_CREDENTIALS, token.as_deref())? else {
            return Ok(None);
        };
        if status >= 500 {
            return Err(answered(status));
        }
        if status != 200 {
            // An instance without a role has no credentials to give.
            return Ok(None);
        }
        let listing = String::from_utf8_lossy(&body);
        let Some(role) = listing.lines().map(str::trim).find(|line| !line.is_empty()) else {
            return Ok(None);
        };
        let path = format!("{SECURITY_CREDENTIALS}{role}");
        let Some((status, body)) = self.get(http, &path, token.as_deref())? else {
            return Ok(None);
        };
        if status >= 500 {
            return Err(answered(status));
        }
        if status != 200 {
            return Ok(None);
        }
        credentials::parse_document(&body, "instance metadata")
            .map(|found| Some(extended(found, now)))
    }

    /// The region the instance runs in, from its identity document.
    pub(crate) fn region(&self, http: &crate::http::Session) -> Option<String> {
        let token = match self.reach(http) {
            Reach::Token(token) => Some(token),
            Reach::NoToken if self.v1_allowed => None,
            Reach::NoToken | Reach::Refused | Reach::Unreachable => return None,
        };
        let (status, body) = self.get(http, IDENTITY_DOCUMENT, token.as_deref()).ok()??;
        if status != 200 {
            return None;
        }
        let document: serde_json::Value = serde_json::from_slice(&body).ok()?;
        document
            .get("region")
            .and_then(serde_json::Value::as_str)
            .filter(|region| !region.is_empty())
            .map(str::to_owned)
    }

    /// Ask for an IMDSv2 session token, as botocore's
    /// `_fetch_metadata_token` asks.
    ///
    /// A `403`, `404` or `405` is the service saying it issues none, and the
    /// reads go on without; a `400` is the service refusing the request, and
    /// nothing is read. A server-side failure is tried again within the
    /// attempts, and one the attempts did not mend issues no token either.
    /// A timeout is a token that never came back - a hop limit of one drops
    /// the answer on its way into a container, while the reads' answers
    /// arrive - and the reads go on without; the client does not tell a
    /// connection that never opened from an answer that never came, so off
    /// an instance whose address swallows packets the reads then time out
    /// too, one deadline more, as the AWS tools spend it. A connection
    /// refused, unreachable or hung up on before any answer is no service.
    fn reach(&self, http: &crate::http::Session) -> Reach {
        let request = match http.put(&format!("{}/latest/api/token", self.endpoint), "") {
            Ok(request) => self.bounded(request),
            Err(_) => return Reach::Unreachable,
        };
        let request = match request.with_header("x-aws-ec2-metadata-token-ttl-seconds", TOKEN_TTL) {
            Ok(request) => request,
            Err(_) => return Reach::Unreachable,
        };
        match super::Answer::of(&request) {
            Ok(answer) if answer.status == 200 => {
                let token = String::from_utf8_lossy(&answer.body).trim().to_owned();
                if token.is_empty() {
                    Reach::NoToken
                } else {
                    Reach::Token(token)
                }
            }
            Ok(answer) if answer.status == 400 => {
                log::debug!(
                    "the instance metadata service refused the token request at {} with 400",
                    self.endpoint
                );
                Reach::Refused
            }
            Err(error) if crate::http::is_unanswered(&error) && !timed_out(&error) => {
                Reach::Unreachable
            }
            // A status declining to issue one, a failure the attempts did
            // not mend, an answer that did not come back in time.
            _ => Reach::NoToken,
        }
    }

    /// Read `path`, with the session token when there is one.
    ///
    /// A server-side failure is tried again within the attempts, and the
    /// last one answered; `None` when nothing answered within them.
    fn get(
        &self,
        http: &crate::http::Session,
        path: &str,
        token: Option<&str>,
    ) -> Result<Option<(u16, std::sync::Arc<[u8]>)>> {
        let mut request = self.bounded(http.get(&format!("{}{path}", self.endpoint))?);
        if let Some(token) = token {
            request = request.with_header("x-aws-ec2-metadata-token", token)?;
        }
        match super::Answer::of(&request) {
            Ok(answer) => Ok(Some((answer.status, answer.body))),
            Err(error) if crate::http::is_unanswered(&error) => Ok(None),
            Err(error) => Err(Error::Io(std::io::Error::other(format!(
                "reading instance metadata failed: {error}"
            )))),
        }
    }

    /// `request` as the service is asked: directly, never through a proxy,
    /// each attempt within the timeout, as many attempts as configured.
    fn bounded(&self, request: crate::http::Request) -> crate::http::Request {
        request
            .with_direct(true)
            .with_deadline(self.timeout)
            .with_max_attempts(self.attempts.max(1))
    }
}

/// `found` as it may sign at `now`: untouched unless its expiry is already
/// past, else carried to `now` plus [`STALE_EXTENSION`] and a jitter between
/// [`STALE_JITTER_MIN`] and [`STALE_JITTER_MAX`] seconds - the jitter so the
/// instances of one fleet do not all ask again at one instant - with a
/// warning naming the set, its expiry and the new one.
fn extended(found: Credentials, now: SystemTime) -> Credentials {
    let Some(expiry) = found.expires_at().filter(|expiry| *expiry <= now) else {
        return found;
    };
    let jitter = STALE_JITTER_MIN
        + crate::http::retry::fresh_jitter() % (STALE_JITTER_MAX - STALE_JITTER_MIN + 1);
    let until = now + STALE_EXTENSION + Duration::from_secs(jitter);
    log::warn!(
        "the instance metadata service served the AWS credential set {} lapsed at {}, as it \
         does while its credential service is unavailable: signing with it until {} and asking \
         again before then",
        found.key_id_hint(),
        iso8601(expiry),
        iso8601(until)
    );
    found.with_expiry(until)
}
