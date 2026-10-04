//! The credential endpoint a container platform serves.
//!
//! ECS and EKS Pod Identity hand a task its keys through an HTTP endpoint the
//! platform names in the environment: `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`
//! on the ECS agent's link-local address, or
//! `AWS_CONTAINER_CREDENTIALS_FULL_URI` anywhere the platform allows, with an
//! authorization token in `AWS_CONTAINER_AUTHORIZATION_TOKEN` or in the file
//! `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE` names. A full URI is accepted only
//! on the addresses the AWS tools accept it on, so a variable a hostile
//! neighbour set cannot send a task's token to a host of their choosing.

use std::time::Duration;

use super::credentials::{self, Credentials};
use crate::auth::{Environment, refusal};
use crate::{Error, Result, Scheme, Url};

/// Where the ECS container agent serves credentials from.
const CONTAINER_HOST: &str = "http://169.254.170.2";
/// The endpoint is on-link, so an answer is immediate or not coming.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Attempts before an endpoint that does not answer is given up on.
const ATTEMPTS: u32 = 3;

/// The credential set the container endpoint serves, when the environment
/// names one.
///
/// `None` when no variable names an endpoint.
///
/// # Errors
///
/// A full URI on a host the AWS tools refuse, a token file that cannot be
/// read or holds a newline, an endpoint that does not answer, or an answer
/// that is not a credential document.
pub(crate) fn credentials(
    http: &crate::http::Session,
    env: &Environment,
) -> Result<Option<Credentials>> {
    let Some(url) = uri(env)? else {
        return Ok(None);
    };
    let token = match env.get("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE") {
        Some(path) => Some(std::fs::read_to_string(&path).map_err(|error| {
            refusal(format!(
                "could not read AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE {path}: {error}"
            ))
        })?),
        None => env.get("AWS_CONTAINER_AUTHORIZATION_TOKEN"),
    };
    let token = token.map(|token| token.trim().to_owned());
    if token
        .as_deref()
        .is_some_and(|token| token.contains(['\r', '\n']))
    {
        return Err(refusal(
            "the container authorization token holds a line break, which no header may carry",
        ));
    }
    // The endpoint is on the machine or the task's own network, so it is
    // reached directly whatever proxy the environment names; a server-side
    // failure or a connection that took nothing is tried again.
    let mut request = http
        .get(&url)?
        .with_deadline(TIMEOUT)
        .with_max_attempts(ATTEMPTS)
        .with_direct(true);
    if let Some(token) = &token {
        request = request.with_header("authorization", token)?;
    }
    let answer = match super::Answer::of(&request) {
        Ok(answer) => answer,
        Err(error) => {
            return Err(refusal(format!(
                "the container credential endpoint {url} did not answer: {error}"
            )));
        }
    };
    if answer.status >= 500 {
        return Err(refusal(format!(
            "the container credential endpoint answered {}",
            answer.status
        )));
    }
    if answer.status != 200 {
        return Err(Error::remote(
            "ecs",
            "ContainerCredentials",
            answer.status,
            "CredentialsUnavailable",
            format!(
                "the container credential endpoint answered {}",
                answer.status
            ),
            url,
        ));
    }
    credentials::parse_document(&answer.body, "container credentials").map(Some)
}

/// The endpoint the environment names, when it names one.
///
/// # Errors
///
/// A full URI that is neither `https` nor on a loopback or link-local
/// address the platforms serve from.
pub(crate) fn uri(env: &Environment) -> Result<Option<String>> {
    // The relative URI first, as the AWS tools read them; it is a path on
    // the agent's address, so it starts with `/` or it names nothing.
    if let Some(relative) = env.get("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
        if !relative.starts_with('/') {
            return Err(refusal(format!(
                "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI {relative} is not a path on the container agent"
            )));
        }
        return Ok(Some(format!("{CONTAINER_HOST}{relative}")));
    }
    let Some(full) = env.get("AWS_CONTAINER_CREDENTIALS_FULL_URI") else {
        return Ok(None);
    };
    let Some(url) = allowed_full_uri(&full) else {
        return Err(refusal(format!(
            "AWS_CONTAINER_CREDENTIALS_FULL_URI {full} is neither https nor on a loopback \
             or container-agent address, so nothing here presents a token to it"
        )));
    };
    // The URL the rule judged, never the text beside it: the host checked is
    // the host the request goes to.
    Ok(Some(url.to_string()))
}

/// The URL a full URI names, when it is on a host the AWS tools present a
/// token to; `None` for everything else, text no URL reader reads included.
///
/// The URI is read once, by the crate's one URL reader, and the host the rule
/// judges is that reading's: the authority after its user information, with
/// no port. A text that is not a URL, or names no host, is refused, never
/// split by hand.
fn allowed_full_uri(text: &str) -> Option<Url> {
    use std::net::{Ipv4Addr, Ipv6Addr};

    let url = Url::from_str(text).ok()?;
    let host = url.authority().host().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    if *url.scheme() == Scheme::HTTPS {
        return Some(url);
    }
    if *url.scheme() != Scheme::HTTP {
        return None;
    }
    // An address, never a name that begins like one: `127.0.0.1.example`
    // is a host somebody else answers.
    let allowed = host == "localhost"
        || host.parse::<Ipv4Addr>().is_ok_and(|address| {
            address.is_loopback()
                || address == Ipv4Addr::new(169, 254, 170, 2)
                || address == Ipv4Addr::new(169, 254, 170, 23)
        })
        || host.parse::<Ipv6Addr>().is_ok_and(|address| {
            address.is_loopback() || address == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x23)
        });
    allowed.then_some(url)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/container.rs` pins and a caller cannot reach.
    //!
    //! The host rule is the whole of what keeps a token off a stranger's
    //! host, so it is pinned by spelling; the fetch itself is driven over a
    //! socket by the suite.

    /// Whether a full URI is on a host the AWS tools present a token to.
    pub fn is_allowed_full_uri(url: &str) -> bool {
        super::allowed_full_uri(url).is_some()
    }
}
