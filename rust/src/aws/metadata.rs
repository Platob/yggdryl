//! The EC2 instance metadata service, which serves an instance its own role.
//!
//! Every EC2 instance, and every service built on one, answers on a
//! link-local address: a session token over IMDSv2, the name of the role the
//! instance profile carries, that role's current keys, and the instance's
//! own region. Off an instance the address answers nothing, so silence is
//! read as "not an instance" after the one short timeout the AWS tools also
//! wait, and only a service that answered and then failed is a failure.

use std::time::Duration;

use super::credentials::{self, Credentials};
use crate::{Error, Result};

/// Where the service lives when nothing names another address.
pub(crate) const IPV4_ENDPOINT: &str = "http://169.254.169.254";
/// Where it lives on an IPv6-only instance.
pub(crate) const IPV6_ENDPOINT: &str = "http://[fd00:ec2::254]";
/// How long an IMDSv2 session token is asked to last.
const TOKEN_TTL: &str = "21600";
/// The largest metadata document read.
const MAX_ANSWER: u64 = 64 * 1024;
/// The one path that answers the instance's own region.
const IDENTITY_DOCUMENT: &str = "/latest/dynamic/instance-identity/document";
/// Under this path, the role name, then the role's keys.
const SECURITY_CREDENTIALS: &str = "/latest/meta-data/iam/security-credentials/";

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

/// What asking for a session token established.
enum Reach {
    /// A token, to send with every read.
    Token(String),
    /// The service answered without issuing one, and may be read as IMDSv1.
    NoToken,
    /// Nothing answered: this is not an instance.
    Unreachable,
}

impl Imds {
    /// The keys of the role the instance profile carries.
    ///
    /// `None` when the service does not answer, or answers that the instance
    /// has no role.
    ///
    /// # Errors
    ///
    /// A service that answered and then failed, or answered something that is
    /// not a credential document.
    pub(crate) fn credentials(&self, agent: &ureq::Agent) -> Result<Option<Credentials>> {
        let token = match self.reach(agent) {
            Reach::Token(token) => Some(token),
            Reach::NoToken if self.v1_allowed => None,
            Reach::NoToken | Reach::Unreachable => return Ok(None),
        };
        let Some((status, body)) = self.get(agent, SECURITY_CREDENTIALS, token.as_deref())? else {
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
        let Some((status, body)) = self.get(agent, &path, token.as_deref())? else {
            return Ok(None);
        };
        if status >= 500 {
            return Err(answered(status));
        }
        if status != 200 {
            return Ok(None);
        }
        credentials::parse_document(&body, "instance metadata").map(Some)
    }

    /// The region the instance runs in, from its identity document.
    pub(crate) fn region(&self, agent: &ureq::Agent) -> Option<String> {
        let token = match self.reach(agent) {
            Reach::Token(token) => Some(token),
            Reach::NoToken if self.v1_allowed => None,
            Reach::NoToken | Reach::Unreachable => return None,
        };
        let (status, body) = self
            .get(agent, IDENTITY_DOCUMENT, token.as_deref())
            .ok()??;
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

    /// Ask for an IMDSv2 session token.
    ///
    /// A `400`, `403`, `404` or `405` is the service saying it issues none -
    /// an older one, or a hop limit in the way - and reads go on without;
    /// any other failure is tried again within the attempts, and a service
    /// that never answers is not there.
    fn reach(&self, agent: &ureq::Agent) -> Reach {
        let mut answered = false;
        for _ in 0..self.attempts.max(1) {
            let answer = agent
                .put(format!("{}/latest/api/token", self.endpoint))
                .config()
                .timeout_global(Some(self.timeout))
                .build()
                .header("x-aws-ec2-metadata-token-ttl-seconds", TOKEN_TTL)
                .send_empty();
            let Ok(mut answer) = answer else {
                continue;
            };
            answered = true;
            match answer.status().as_u16() {
                200 => {
                    return match answer.body_mut().read_to_string() {
                        Ok(token) if !token.trim().is_empty() => {
                            Reach::Token(token.trim().to_owned())
                        }
                        _ => Reach::NoToken,
                    };
                }
                400 | 403 | 404 | 405 => return Reach::NoToken,
                _ => {}
            }
        }
        if answered {
            Reach::NoToken
        } else {
            Reach::Unreachable
        }
    }

    /// Read `path`, with the session token when there is one.
    ///
    /// A server-side failure is tried again within the attempts; `None` when
    /// nothing answered within them.
    fn get(
        &self,
        agent: &ureq::Agent,
        path: &str,
        token: Option<&str>,
    ) -> Result<Option<(u16, Vec<u8>)>> {
        let mut last = None;
        for _ in 0..self.attempts.max(1) {
            let mut request = agent
                .get(format!("{}{path}", self.endpoint))
                .config()
                .timeout_global(Some(self.timeout))
                .build();
            if let Some(token) = token {
                request = request.header("x-aws-ec2-metadata-token", token);
            }
            let Ok(mut answer) = request.call() else {
                continue;
            };
            let status = answer.status().as_u16();
            let body = answer
                .body_mut()
                .with_config()
                .limit(MAX_ANSWER)
                .read_to_vec()
                .map_err(|error| {
                    Error::Io(std::io::Error::other(format!(
                        "reading instance metadata failed: {error}"
                    )))
                })?;
            if status >= 500 {
                last = Some((status, body));
                continue;
            }
            return Ok(Some((status, body)));
        }
        Ok(last)
    }
}
