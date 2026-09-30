//! Which proxy a request goes through, read from the environment the way
//! curl and Python's `requests` read it, at the moment the request is sent.
//!
//! The options' own `proxy` wins over everything, and a client that does not
//! read the environment goes direct. Otherwise a host `no_proxy` names goes
//! direct; else `https_proxy` carries an `https` URL and `http_proxy` an
//! `http` one, then `all_proxy` either - each variable read lower case first,
//! then upper case, an empty one unset. Under CGI - `REQUEST_METHOD` set -
//! upper-case `HTTP_PROXY` is not read, because a server sets it from the
//! `Proxy` header of the request it is answering (httpoxy, CVE-2016-5385);
//! `requests` and curl leave it unread there too. The environment is read
//! for every request, so a process that sets, changes or clears a proxy
//! after its first request is followed at its next one.
//!
//! A `no_proxy` entry is a host, matched with every host under it (`.`, `*.`
//! and bare spellings alike, on a label boundary, so `example.com` never
//! covers `badexample.com`), an IP address, an IP network in CIDR form -
//! read by [`IpNetwork`], the one CIDR grammar, so `[fd00::]/8` is the
//! network and an IPv4-mapped IPv6 host is matched as the IPv4 address it
//! maps - any of them with `:port` to match that port alone, or `*` for
//! every host.

use std::net::IpAddr;

use crate::Url;
use crate::http::IpNetwork;

/// The variables naming the proxy of an `http` URL, lower case first.
const HTTP_PROXY: [&str; 2] = ["http_proxy", "HTTP_PROXY"];
/// The variables naming the proxy of an `https` URL.
const HTTPS_PROXY: [&str; 2] = ["https_proxy", "HTTPS_PROXY"];
/// The variables naming the proxy of any URL their scheme's leaves unset.
const ALL_PROXY: [&str; 2] = ["all_proxy", "ALL_PROXY"];
/// The variables naming the hosts that go direct.
const NO_PROXY: [&str; 2] = ["no_proxy", "NO_PROXY"];

/// The proxy URL the environment names for `url`, or `None` to go direct.
///
/// `variable` reads one environment variable - the process's in a client,
/// a map in a test - trimmed, an empty one `None`.
pub(crate) fn environment_proxy(
    url: &Url,
    variable: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let first = |names: [&str; 2]| names.iter().find_map(|name| variable(name));
    let host = url.hostname()?;
    let https = url.scheme().as_str().eq_ignore_ascii_case("https");
    let port = url
        .authority()
        .port()
        .unwrap_or(if https { 443 } else { 80 });
    if first(NO_PROXY).is_some_and(|list| bypasses(&list, host, Some(port))) {
        return None;
    }
    let named = if https {
        first(HTTPS_PROXY)
    } else if variable("REQUEST_METHOD").is_some() {
        variable(HTTP_PROXY[0])
    } else {
        first(HTTP_PROXY)
    };
    named.or_else(|| first(ALL_PROXY))
}

/// Whether the `no_proxy` list `list` sends `host`, at `port`, direct.
fn bypasses(list: &str, host: &str, port: Option<u16>) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let address = host.parse::<IpAddr>().ok();
    list.split([',', ' '])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry_matches(entry, &host, address, port))
}

/// Whether one `no_proxy` entry covers `host`.
fn entry_matches(entry: &str, host: &str, address: Option<IpAddr>, port: Option<u16>) -> bool {
    if entry == "*" {
        return true;
    }
    let (pattern, wanted_port) = split_port(entry);
    if wanted_port.is_some() && wanted_port != port {
        return false;
    }
    if let Some(address) = address {
        // An address alone is the network of that one address.
        return IpNetwork::from_str(pattern).is_ok_and(|network| network.contains(address));
    }
    let pattern = pattern.to_ascii_lowercase();
    let pattern = pattern
        .trim_start_matches('*')
        .trim_start_matches('.')
        .trim_end_matches('.');
    !pattern.is_empty()
        && (host == pattern
            || host
                .strip_suffix(pattern)
                .is_some_and(|rest| rest.ends_with('.')))
}

/// An entry and the port it names after its last `:`, when it names one; an
/// IPv6 address's own colons are no port, and a bracketed address keeps its
/// brackets and the `/bits` after them: `[fd00::]/8:443` is `[fd00::]/8`
/// at port 443.
fn split_port(entry: &str) -> (&str, Option<u16>) {
    if entry.starts_with('[')
        && let Some(close) = entry.find(']')
    {
        let tail = &entry[close + 1..];
        let (network, port) = match tail.rsplit_once(':') {
            Some((bits, port)) => (close + 1 + bits.len(), port.parse().ok()),
            None => (entry.len(), None),
        };
        return (&entry[..network], port);
    }
    match entry.rsplit_once(':') {
        Some((pattern, port)) if !pattern.contains(':') => match port.parse() {
            Ok(port) => (pattern, Some(port)),
            Err(_) => (entry, None),
        },
        _ => (entry, None),
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/proxy.rs` pins and a caller cannot reach.
    //!
    //! The choice is pinned against a map standing in for the environment,
    //! so no test changes the process's own under a suite running beside it.
    use std::collections::BTreeMap;

    use crate::{Result, Url};

    /// The proxy `variables` name for `url`, or `None` to go direct.
    pub fn environment_proxy(url: &str, variables: &[(&str, &str)]) -> Result<Option<String>> {
        let url = Url::from_str(url)?;
        let variables: BTreeMap<&str, &str> = variables.iter().copied().collect();
        Ok(super::environment_proxy(&url, |name| {
            variables
                .get(name)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        }))
    }
}
