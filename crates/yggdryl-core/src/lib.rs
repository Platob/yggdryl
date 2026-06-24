//! # yggdryl-core
//!
//! Dependency-free foundations shared by the **yggdryl** crates:
//!
//! - the generic [`FromInput`] parsing trait (with [`Input`], [`Mapping`] and
//!   [`Params`]) — every parse takes a `safe` flag (`true` validates fully,
//!   `false` is a faster, lenient parse);
//! - URL-safe percent-encoding ([`percent_encode`] / [`percent_decode`]) and the
//!   lower-level component helpers used by [`yggdryl-url`](https://crates.io/crates/yggdryl-url);
//! - the [`Version`] (`major.minor.patch`) value type.
//!
//! The `Uri` and `Url` types live in the `yggdryl-url` crate, which builds on
//! these foundations.

use std::collections::BTreeMap;
use std::fmt;

/// A set of named components, used by [`FromInput::from_mapping`].
///
/// Keys are component names (`"scheme"`, `"host"`, `"major"`, …) and values are
/// their string form. Which keys each type understands is documented on its
/// [`FromInput`] implementation.
pub type Mapping = BTreeMap<String, String>;

/// A multi-valued query-parameter map: `key` → list of values, mirroring how a
/// query string may repeat a key (`?a=1&a=2`). Used by [`Uri::params`] /
/// [`Url::params`] and friends.
pub type Params = BTreeMap<String, Vec<String>>;

/// The input forms accepted by [`FromInput::from_`].
pub enum Input<'a> {
    /// A full string to be parsed, e.g. `"https://example.com"`.
    Str(&'a str),
    /// A [`Mapping`] of already-split components.
    Mapping(&'a Mapping),
}

impl<'a> From<&'a str> for Input<'a> {
    fn from(value: &'a str) -> Self {
        Input::Str(value)
    }
}

impl<'a> From<&'a String> for Input<'a> {
    fn from(value: &'a String) -> Self {
        Input::Str(value.as_str())
    }
}

impl<'a> From<&'a Mapping> for Input<'a> {
    fn from(value: &'a Mapping) -> Self {
        Input::Mapping(value)
    }
}

/// A generic parsing interface implemented by [`Uri`], [`Url`] and [`Version`].
///
/// Implementors provide [`from_str`](FromInput::from_str) and
/// [`from_mapping`](FromInput::from_mapping), each taking a `safe` flag — `true`
/// validates the input thoroughly, `false` is a faster, lenient parse. The
/// [`from_`](FromInput::from_) entry point dispatches over any [`Input`] form and
/// **always parses safely** (the common default); use `from_str`/`from_mapping`
/// with `safe = false` for the lenient path.
pub trait FromInput: Sized {
    /// The error produced when parsing fails.
    type Err;

    /// Parses a full string.
    fn from_str(input: &str, safe: bool) -> Result<Self, Self::Err>;

    /// Parses from a [`Mapping`] of pre-split components. The default reads a
    /// `"str"` entry and delegates to [`from_str`](FromInput::from_str), so a
    /// type only needs `from_str`; [`Uri`], [`Url`] and [`Version`] override this
    /// with a component-based parse that avoids a useless string round-trip.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<Self, Self::Err> {
        Self::from_str(
            fields.get("str").map(String::as_str).unwrap_or_default(),
            safe,
        )
    }

    /// Parses any supported [`Input`] form with full validation (`safe = true`).
    fn from_<'a, I: Into<Input<'a>>>(input: I) -> Result<Self, Self::Err> {
        match input.into() {
            Input::Str(s) => Self::from_str(s, true),
            Input::Mapping(m) => Self::from_mapping(m, true),
        }
    }
}

/// The output forms produced by [`ToOutput`], mirroring [`Input`].
pub enum Output {
    /// A rendered string, e.g. `"https://example.com"`.
    Str(String),
    /// A [`Mapping`] of components.
    Mapping(Mapping),
}

/// The inverse of [`FromInput`]: render a value back into a string or a component
/// [`Mapping`]. Implemented by [`Uri`], [`Url`] and [`Version`].
///
/// `to_mapping` is the inverse of [`FromInput::from_mapping`], so
/// `T::from_(&value.to_mapping())` round-trips.
pub trait ToOutput {
    /// Renders to a string. `encode` controls percent-encoding where relevant.
    fn to_str(&self, encode: bool) -> String;

    /// Renders to a component [`Mapping`]. The default wraps the string form under
    /// a `"str"` key (the inverse of the default [`from_mapping`](FromInput::from_mapping));
    /// [`Uri`], [`Url`] and [`Version`] override it with real component maps that
    /// avoid a useless string serialization.
    fn to_mapping(&self) -> Mapping {
        Mapping::from([("str".to_string(), self.to_str(true))])
    }

    /// Renders to any [`Output`] form: a [`Mapping`] when `as_mapping`, otherwise
    /// the string form (whose encoding is controlled by `encode`).
    fn to_(&self, as_mapping: bool, encode: bool) -> Output {
        if as_mapping {
            Output::Mapping(self.to_mapping())
        } else {
            Output::Str(self.to_str(encode))
        }
    }
}

/// Error from [`percent_decode`] (and surfaced by `safe` parses).
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
/// ```
/// use yggdryl_core::percent_decode;
/// assert_eq!(percent_decode("a%20b").unwrap(), "a b");
/// ```
pub fn percent_decode(input: &str) -> Result<String, EncodingError> {
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
    String::from_utf8(out).map_err(|_| EncodingError::InvalidUtf8)
}

/// Validates that every `%` in `input` is followed by two hex digits, used by
/// `safe` parses.
pub fn validate_percent_encoding(input: &str) -> Result<(), EncodingError> {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let ok = bytes.get(i + 1).is_some_and(|b| b.is_ascii_hexdigit())
                && bytes.get(i + 2).is_some_and(|b| b.is_ascii_hexdigit());
            if !ok {
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
pub fn encode_component(input: &str, keep: &[u8]) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let is_escape = byte == b'%'
            && bytes.get(i + 1).is_some_and(|b| b.is_ascii_hexdigit())
            && bytes.get(i + 2).is_some_and(|b| b.is_ascii_hexdigit());
        if is_escape {
            out.push_str(&input[i..i + 3]);
            i += 3;
        } else {
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
    out
}
