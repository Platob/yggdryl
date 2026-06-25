//! URL-safe percent-encoding ([`percent_encode`] / [`percent_decode`]) and the
//! lower-level component helpers used by the URL types.

use std::borrow::Cow;
use std::fmt;

/// Error from [`percent_decode`] (and surfaced by validated parses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodingError {
    /// A `%` was not followed by two hexadecimal digits.
    InvalidEscape(String),
    /// The decoded bytes were not valid UTF-8.
    InvalidUtf8,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodingError::InvalidEscape(s) => write!(f, "invalid percent-escape in '{s}'"),
            EncodingError::InvalidUtf8 => write!(f, "percent-decoded bytes are not valid UTF-8"),
        }
    }
}

impl std::error::Error for EncodingError {}

/// Returns `true` for the RFC 3986 *unreserved* characters, which never need
/// percent-encoding: `ALPHA / DIGIT / "-" / "." / "_" / "~"`.
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// Returns `true` if `bytes[i..]` begins with a valid `%XX` escape (a `%`
/// followed by two hex digits). Shared by the encode/validate scanners.
fn is_escape_at(bytes: &[u8], i: usize) -> bool {
    bytes[i] == b'%'
        && bytes.get(i + 1).is_some_and(|b| b.is_ascii_hexdigit())
        && bytes.get(i + 2).is_some_and(|b| b.is_ascii_hexdigit())
}

/// Percent-encodes `input` (URL-safe): every byte outside the unreserved set is
/// written as `%XX`, e.g. a space becomes `%20`.
///
/// ```
/// use yggdryl_core::percent_encode;
/// assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
/// ```
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0f));
        }
    }
    out
}

/// Percent-decodes `input`, turning each `%XX` escape back into a byte.
///
/// Zero-copy with a check: when `input` carries no `%`, it is already its own
/// decoding (and valid UTF-8 by virtue of being a `&str`), so the input is
/// borrowed unchanged; an allocation happens only when there is something to
/// decode.
///
/// ```
/// use yggdryl_core::percent_decode;
/// assert_eq!(percent_decode("a%20b").unwrap(), "a b");
/// ```
pub fn percent_decode(input: &str) -> Result<Cow<'_, str>, EncodingError> {
    if !input.contains('%') {
        return Ok(Cow::Borrowed(input));
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes
                .get(i + 1)
                .and_then(|b| hex_value(*b))
                .ok_or_else(|| EncodingError::InvalidEscape(input.to_string()))?;
            let lo = bytes
                .get(i + 2)
                .and_then(|b| hex_value(*b))
                .ok_or_else(|| EncodingError::InvalidEscape(input.to_string()))?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .map(Cow::Owned)
        .map_err(|_| EncodingError::InvalidUtf8)
}

/// Validates that every `%` in `input` is followed by two hex digits, used by
/// parsing.
pub fn validate_percent_encoding(input: &str) -> Result<(), EncodingError> {
    // Zero-copy check: nothing to validate without a `%`.
    if !input.contains('%') {
        return Ok(());
    }
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if !is_escape_at(bytes, i) {
                return Err(EncodingError::InvalidEscape(input.to_string()));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Maps a nibble (0–15) to its uppercase hex digit.
fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

/// Maps an ASCII hex digit to its value (0–15), or `None`.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-encodes `input` for output, preserving the bytes in `keep` (the
/// component's structural delimiters) and any already-valid `%XX` escape — so it
/// is idempotent and never double-encodes.
///
/// Zero-copy with a check: it first scans for the first byte that needs escaping
/// and borrows `input` unchanged when there is none; only then does it allocate,
/// copying the already-valid prefix verbatim before encoding the remainder.
pub fn encode_component<'a>(input: &'a str, keep: &[u8]) -> Cow<'a, str> {
    let bytes = input.as_bytes();
    // A byte passes through untouched if it is unreserved, a kept delimiter, or
    // the start of an already-valid escape (advancing three bytes).
    let mut i = 0;
    let split = loop {
        if i >= bytes.len() {
            break None;
        }
        if is_escape_at(bytes, i) {
            i += 3;
        } else if is_unreserved(bytes[i]) || keep.contains(&bytes[i]) {
            i += 1;
        } else {
            break Some(i);
        }
    };
    let Some(start) = split else {
        return Cow::Borrowed(input);
    };

    let mut out = String::with_capacity(input.len() + 8);
    out.push_str(&input[..start]);
    let mut i = start;
    while i < bytes.len() {
        if is_escape_at(bytes, i) {
            out.push_str(&input[i..i + 3]);
            i += 3;
        } else {
            let byte = bytes[i];
            if is_unreserved(byte) || keep.contains(&byte) {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0f));
            }
            i += 1;
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_edge_cases() {
        // Empty and all-unreserved inputs are returned unchanged.
        assert_eq!(percent_encode(""), "");
        assert_eq!(percent_encode("Aa0-._~"), "Aa0-._~");
        // Multi-byte UTF-8 is encoded byte-by-byte, upper-case hex.
        assert_eq!(percent_encode("é"), "%C3%A9");
        assert_eq!(percent_encode("a b"), "a%20b");
    }

    #[test]
    fn percent_decode_edge_cases() {
        // Fast path: no `%` returns the input verbatim.
        assert_eq!(percent_decode("plain text").unwrap(), "plain text");
        assert_eq!(percent_decode("").unwrap(), "");
        // Mixed-case hex digits both decode.
        assert_eq!(percent_decode("%c3%A9").unwrap(), "é");
        // Truncated or non-hex escapes error.
        assert_eq!(
            percent_decode("a%"),
            Err(EncodingError::InvalidEscape("a%".to_string()))
        );
        assert!(percent_decode("%2").is_err());
        assert!(percent_decode("%zz").is_err());
        // A `%` mid-string with valid hex round-trips with encode.
        assert_eq!(
            percent_decode(&percent_encode("100%/done")).unwrap(),
            "100%/done"
        );
    }

    #[test]
    fn validate_percent_encoding_edge_cases() {
        assert!(validate_percent_encoding("no escapes here").is_ok());
        assert!(validate_percent_encoding("ok%20ok").is_ok());
        assert!(validate_percent_encoding("bad%2").is_err());
        assert!(validate_percent_encoding("bad%gg").is_err());
        // A trailing `%` is invalid.
        assert!(validate_percent_encoding("trailing%").is_err());
    }

    #[test]
    fn zero_copy_borrows_when_unchanged() {
        // No `%` -> decode borrows; an escape forces an owned allocation.
        assert!(matches!(
            percent_decode("nothing here"),
            Ok(Cow::Borrowed(_))
        ));
        assert!(matches!(percent_decode("a%20b"), Ok(Cow::Owned(_))));
        // All-safe (incl. already-valid escapes) -> encode borrows; a byte that
        // must be escaped forces an owned allocation.
        assert!(matches!(
            encode_component("already/safe", b"/"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(encode_component("a%20b", b""), Cow::Borrowed(_)));
        assert!(matches!(
            encode_component("needs space", b""),
            Cow::Owned(_)
        ));
    }

    #[test]
    fn encode_component_is_idempotent() {
        // An already-encoded escape is preserved (never double-encoded).
        assert_eq!(encode_component("a%20b", b"/"), "a%20b");
        // Kept delimiters pass through; others are encoded.
        assert_eq!(encode_component("/a b", b"/"), "/a%20b");
        assert_eq!(encode_component("a/b", b""), "a%2Fb");
    }
}
