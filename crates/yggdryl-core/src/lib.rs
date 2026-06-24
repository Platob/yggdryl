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
/// [`from_mapping`](FromInput::from_mapping); the [`from_`](FromInput::from_) entry
/// point dispatches over an [`Input`] for free. Every method takes a `safe`
/// flag — `true` validates the input thoroughly, `false` is a faster, lenient
/// parse that skips the optional checks.
pub trait FromInput: Sized {
    /// The error produced when parsing fails.
    type Err;

    /// Parses a full string.
    fn from_str(input: &str, safe: bool) -> Result<Self, Self::Err>;

    /// Parses from a [`Mapping`] of pre-split components.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<Self, Self::Err>;

    /// Parses from any supported [`Input`] form.
    fn from_<'a, I: Into<Input<'a>>>(input: I, safe: bool) -> Result<Self, Self::Err> {
        match input.into() {
            Input::Str(s) => Self::from_str(s, safe),
            Input::Mapping(m) => Self::from_mapping(m, safe),
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

// Per-component sets of delimiter bytes that are left as-is when encoding a
// component for output (on top of the always-safe unreserved set).
pub const KEEP_AUTHORITY: &[u8] = b":@[]";
pub const KEEP_PATH: &[u8] = b"/:@";
pub const KEEP_QUERY: &[u8] = b"/:@?&=";
pub const KEEP_FRAGMENT: &[u8] = b"/:@?";

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

/// Renders a component either percent-encoded (`encode`) or percent-decoded
/// (best effort), used by `to_str(encode)`.
pub fn render_component(input: &str, keep: &[u8], encode: bool) -> String {
    if encode {
        encode_component(input, keep)
    } else {
        percent_decode(input).unwrap_or_else(|_| input.to_string())
    }
}

/// Splits a `key=value&key=value2` query into a multimap. Repeated keys
/// accumulate their values; when `decode`, each key/value is percent-decoded
/// (parts that fail to decode are kept verbatim).
pub fn query_to_params(query: &str, decode: bool) -> Params {
    let unescape = |s: &str| {
        if decode {
            percent_decode(s).unwrap_or_else(|_| s.to_string())
        } else {
            s.to_string()
        }
    };
    let mut params = Params::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params
            .entry(unescape(key))
            .or_default()
            .push(unescape(value));
    }
    params
}

/// Builds a `key=value&…` query from a [`Params`] multimap. When `encode`, each
/// key/value is percent-encoded. Keys with several values are emitted once per
/// value; pairs come out in the map's (sorted) order for a deterministic result.
pub fn params_to_query(params: &Params, encode: bool) -> String {
    let escape = |s: &str| {
        if encode {
            percent_encode(s)
        } else {
            s.to_string()
        }
    };
    let mut pairs = Vec::new();
    for (key, values) in params {
        let key = escape(key);
        if values.is_empty() {
            pairs.push(key);
        } else {
            for value in values {
                pairs.push(format!("{key}={}", escape(value)));
            }
        }
    }
    pairs.join("&")
}

/// Error returned when [`Version::from_`] cannot interpret its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// The input was empty.
    Empty,
    /// More than three dot-separated components were given.
    TooManyComponents,
    /// A component was not a non-negative integer.
    InvalidNumber(String),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionError::Empty => write!(f, "version is empty"),
            VersionError::TooManyComponents => {
                write!(f, "version has more than three components")
            }
            VersionError::InvalidNumber(part) => {
                write!(f, "version component '{part}' is not a number")
            }
        }
    }
}

impl std::error::Error for VersionError {}

/// A generic `major.minor.patch` version.
///
/// Ordering is numeric and field-major (`major`, then `minor`, then `patch`), so
/// `Version`s sort the way you would expect. Parsing accepts one, two or three
/// components; any that are omitted default to `0`.
///
/// ```
/// use yggdryl_core::{FromInput, Version};
///
/// let v = Version::from_str("1.4.2", true).unwrap();
/// assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
/// assert_eq!(Version::from_str("2", true).unwrap(), Version::new(2, 0, 0));
/// assert!(Version::new(1, 4, 2) < Version::new(1, 10, 0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// Creates a version from its components.
    pub fn new(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// The major component.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// The minor component.
    pub fn minor(&self) -> u64 {
        self.minor
    }

    /// The patch component.
    pub fn patch(&self) -> u64 {
        self.patch
    }

    /// Returns a copy of this version, overriding any component for which `Some`
    /// is given and keeping `self`'s value otherwise. Call `copy(None, …)` to
    /// clone.
    pub fn copy(&self, major: Option<u64>, minor: Option<u64>, patch: Option<u64>) -> Version {
        Version {
            major: major.unwrap_or(self.major),
            minor: minor.unwrap_or(self.minor),
            patch: patch.unwrap_or(self.patch),
        }
    }

    /// Returns a copy with the major component replaced.
    pub fn with_major(mut self, major: u64) -> Version {
        self.major = major;
        self
    }

    /// Returns a copy with the minor component replaced.
    pub fn with_minor(mut self, minor: u64) -> Version {
        self.minor = minor;
        self
    }

    /// Returns a copy with the patch component replaced.
    pub fn with_patch(mut self, patch: u64) -> Version {
        self.patch = patch;
        self
    }
}

impl FromInput for Version {
    type Err = VersionError;

    /// Parses a `major[.minor[.patch]]` string. When `safe`, every component must
    /// be a non-negative integer and there may be at most three; when not `safe`,
    /// extra components are ignored and non-numeric ones become `0`.
    fn from_str(input: &str, safe: bool) -> Result<Version, VersionError> {
        if input.is_empty() {
            return Err(VersionError::Empty);
        }
        let mut parts = [0u64; 3];
        for (index, part) in input.split('.').enumerate() {
            if index == 3 {
                if safe {
                    return Err(VersionError::TooManyComponents);
                }
                break;
            }
            parts[index] = match part.parse::<u64>() {
                Ok(n) => n,
                Err(_) if !safe => 0,
                Err(_) => return Err(VersionError::InvalidNumber(part.to_string())),
            };
        }
        Ok(Version {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
        })
    }

    /// Builds a [`Version`] from a [`Mapping`]. Recognised keys: `major`, `minor`
    /// and `patch`; any omitted default to `0`.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<Version, VersionError> {
        let component = |key: &str| -> Result<u64, VersionError> {
            match fields.get(key) {
                Some(value) => match value.parse::<u64>() {
                    Ok(n) => Ok(n),
                    Err(_) if !safe => Ok(0),
                    Err(_) => Err(VersionError::InvalidNumber(value.clone())),
                },
                None => Ok(0),
            }
        };
        Ok(Version {
            major: component("major")?,
            minor: component("minor")?,
            patch: component("patch")?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
