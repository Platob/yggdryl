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
use crate::auth::Environment;
use crate::{Error, Result};

/// Where the ECS container agent serves credentials from.
const CONTAINER_HOST: &str = "http://169.254.170.2";
/// The endpoint is on-link, so an answer is immediate or not coming.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Attempts before an endpoint that does not answer is given up on.
const ATTEMPTS: u32 = 3;
/// The largest credential document read.
const MAX_ANSWER: u64 = 64 * 1024;

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
pub(crate) fn credentials(agent: &ureq::Agent, env: &Environment) -> Result<Option<Credentials>> {
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
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let mut request = agent
            .get(&url)
            .config()
            .timeout_global(Some(TIMEOUT))
            .build();
        if let Some(token) = &token {
            request = request.header("Authorization", token);
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(error) => {
                last = Some(format!(
                    "the container credential endpoint {url} did not answer: {error}"
                ));
                continue;
            }
        };
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_vec()
            .map_err(|error| refusal(format!("reading container credentials failed: {error}")))?;
        if status >= 500 {
            last = Some(format!(
                "the container credential endpoint answered {status}"
            ));
            continue;
        }
        if status != 200 {
            return Err(Error::remote(
                "ecs",
                "ContainerCredentials",
                status,
                "CredentialsUnavailable",
                format!("the container credential endpoint answered {status}"),
                url,
            ));
        }
        return credentials::parse_document(&body, "container credentials").map(Some);
    }
    Err(refusal(last.unwrap_or_else(|| {
        "the container credential endpoint did not answer".to_owned()
    })))
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
    if !is_allowed_full_uri(&full) {
        return Err(refusal(format!(
            "AWS_CONTAINER_CREDENTIALS_FULL_URI {full} is neither https nor on a loopback \
             or container-agent address, so nothing here presents a token to it"
        )));
    }
    Ok(Some(full))
}

/// Whether a full URI is on a host the AWS tools present a token to.
fn is_allowed_full_uri(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("https") {
        return true;
    }
    if !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or(authority)
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    let host = host.to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    // An address, never a name that begins like one: `127.0.0.1.example`
    // is a host somebody else answers.
    if let Ok(address) = host.parse::<std::net::Ipv4Addr>() {
        return address.is_loopback()
            || address == std::net::Ipv4Addr::new(169, 254, 170, 2)
            || address == std::net::Ipv4Addr::new(169, 254, 170, 23);
    }
    if let Ok(address) = host.parse::<std::net::Ipv6Addr>() {
        return address.is_loopback()
            || address == std::net::Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x23);
    }
    false
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
    //! What `rust/tests/aws/container.rs` pins and a caller cannot reach.
    //!
    //! The host rule is the whole of what keeps a token off a stranger's
    //! host, so it is pinned by spelling; the fetch itself is driven over a
    //! socket by the suite.

    /// Whether a full URI is on a host the AWS tools present a token to.
    pub fn is_allowed_full_uri(url: &str) -> bool {
        super::is_allowed_full_uri(url)
    }
}
