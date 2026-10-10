//! How long a terminal lets a connection stay quiet.
//!
//! `xmla serve` reads `--read-timeout` as a length of time the way every
//! timeout the core has is read: seconds, a fraction allowed, with an
//! optional unit, so `30`, `2.5`, `1500ms` and `30s` are four spellings of
//! lengths and nothing here picks between readings. The reader is the
//! core's; this file adds only the bounds a server has, which the argument
//! parser states rather than the socket, so a refusal names the flag and
//! binds nothing.

use std::time::Duration;

use yggdryl::http::{HttpOptions, ServerOptions};

/// The `--read-timeout` a text states.
///
/// The length is read by the one reader behind the `timeout` property of
/// [`HttpOptions`] - which treats blank text as a property not given, so a
/// blank value is refused here, where it is a value - and kept within what a
/// server holds: above zero and at most [`ServerOptions::MAX_TIMEOUT`].
///
/// # Errors
///
/// Returns the core's refusal of text that states no length of time, or the
/// bounds the length is outside of.
pub fn read_timeout(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("expected a length of time, got nothing".to_owned());
    }
    let length = HttpOptions::from_properties([("timeout", text)])
        .map_err(|error| error.to_string())?
        .timeout();
    if length.is_zero() || length > ServerOptions::MAX_TIMEOUT {
        return Err(format!(
            "{text} is not above zero and at most {} seconds",
            ServerOptions::MAX_TIMEOUT.as_secs()
        ));
    }
    Ok(length)
}
