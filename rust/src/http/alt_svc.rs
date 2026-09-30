//! `Alt-Svc` (RFC 7838): where else an origin answers, read for the one
//! alternative this client takes up - HTTP/3 on the same host.
//!
//! The field is a comma-separated list of `protocol-id="alt-authority"`
//! entries, each with `;`-separated parameters, or the single word `clear`.
//! An `h3` entry whose authority names no host, or the origin's own host,
//! is taken with its port and its `ma` (seconds, 86,400 when absent); an
//! alternative on another host is passed over, so a request never goes
//! anywhere its origin's certificate was not already the proof of. `clear`
//! withdraws what the origin advertised before. A malformed entry is skipped
//! rather than refused: the field is advice, and the request it rode on has
//! already been answered.

use std::time::Duration;

/// How long an alternative stands when the field states no `ma`.
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(86_400);

/// What one `Alt-Svc` value says of HTTP/3 for its origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Advice {
    /// HTTP/3 is answered on `port` of the same host, for `max_age`.
    Http3 { port: u16, max_age: Duration },
    /// Every alternative the origin advertised is withdrawn.
    Clear,
}

/// The HTTP/3 advice in the `Alt-Svc` value `value` for the origin whose
/// host is `host`; `None` when it offers none this client takes.
pub(crate) fn http3(value: &str, host: &str) -> Option<Advice> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("clear") {
        return Some(Advice::Clear);
    }
    split_outside_quotes(value, ',').find_map(|entry| entry_advice(entry, host))
}

/// The advice of one list entry, when it is a usable `h3` one.
fn entry_advice(entry: &str, host: &str) -> Option<Advice> {
    let mut parts = split_outside_quotes(entry, ';');
    let (protocol, authority) = parts.next()?.trim().split_once('=')?;
    if !protocol.trim().eq_ignore_ascii_case("h3") {
        return None;
    }
    let authority = authority.trim().strip_prefix('"')?.strip_suffix('"')?;
    let (alternative, port) = authority.rsplit_once(':')?;
    let alternative = alternative.trim_start_matches('[').trim_end_matches(']');
    if !alternative.is_empty()
        && !alternative.eq_ignore_ascii_case(host.trim_start_matches('[').trim_end_matches(']'))
    {
        return None;
    }
    let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
    let mut max_age = DEFAULT_MAX_AGE;
    for parameter in parts {
        if let Some((name, value)) = parameter.trim().split_once('=')
            && name.trim().eq_ignore_ascii_case("ma")
        {
            max_age = Duration::from_secs(value.trim().trim_matches('"').parse().ok()?);
        }
    }
    Some(Advice::Http3 { port, max_age })
}

/// `value` split at `separator`, never inside a quoted string.
fn split_outside_quotes(value: &str, separator: char) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    let mut start = 0;
    let mut pieces = Vec::new();
    for (index, character) in value.char_indices() {
        match character {
            '"' => quoted = !quoted,
            other if other == separator && !quoted => {
                pieces.push(&value[start..index]);
                start = index + other.len_utf8();
            }
            _ => {}
        }
    }
    pieces.push(&value[start..]);
    pieces.into_iter()
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/alt_svc.rs` pins and a caller cannot reach.
    use std::time::Duration;

    /// The HTTP/3 advice `value` gives an origin on `host`: `None` for none,
    /// `Some(None)` for `clear`, else the port and the seconds it stands.
    pub fn http3(value: &str, host: &str) -> Option<Option<(u16, Duration)>> {
        super::http3(value, host).map(|advice| match advice {
            super::Advice::Http3 { port, max_age } => Some((port, max_age)),
            super::Advice::Clear => None,
        })
    }
}
