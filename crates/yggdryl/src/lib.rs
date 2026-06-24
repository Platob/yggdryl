//! # yggdryl
//!
//! The pure-Rust core of the **yggdryl** project: small, dependency-free
//! [`Uri`], [`Url`] and [`Version`] value types.
//!
//! - [`Uri`] is the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
//!   shape: `scheme:[//authority]path[?query][#fragment]`.
//! - [`Url`] is the common subset that always has an authority, decomposed into
//!   `username`, `password`, `host` and `port`.
//! - [`Version`] is a generic `major.minor.patch` version.
//!
//! All three implement the generic [`FromInput`] trait, which exposes
//! [`from_str`](FromInput::from_str), [`from_mapping`](FromInput::from_mapping) and
//! a [`from_`](FromInput::from_) entry point that accepts either form. Every
//! parse takes a `safe` flag: `true` fully validates the input, `false` is a
//! faster, lenient parse. URL-safe [`percent_encode`]/[`percent_decode`] round
//! out the API.
//!
//! The Python and Node extensions in the wider project wrap these types so the
//! behaviour is identical across every language binding.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

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
/// use yggdryl::percent_encode;
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
/// use yggdryl::percent_decode;
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
fn validate_percent_encoding(input: &str) -> Result<(), EncodingError> {
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
const KEEP_AUTHORITY: &[u8] = b":@[]";
const KEEP_PATH: &[u8] = b"/:@";
const KEEP_QUERY: &[u8] = b"/:@?&=";
const KEEP_FRAGMENT: &[u8] = b"/:@?";

/// Percent-encodes `input` for output, preserving the bytes in `keep` (the
/// component's structural delimiters) and any already-valid `%XX` escape — so it
/// is idempotent and never double-encodes.
fn encode_component(input: &str, keep: &[u8]) -> String {
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
fn render_component(input: &str, keep: &[u8], encode: bool) -> String {
    if encode {
        encode_component(input, keep)
    } else {
        percent_decode(input).unwrap_or_else(|_| input.to_string())
    }
}

/// Splits a `key=value&key=value2` query into a multimap. Repeated keys
/// accumulate their values; when `decode`, each key/value is percent-decoded
/// (parts that fail to decode are kept verbatim).
fn query_to_params(query: &str, decode: bool) -> Params {
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
fn params_to_query(params: &Params, encode: bool) -> String {
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

/// Error returned when [`Uri`] parsing cannot interpret its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// The input was empty.
    Empty,
    /// No `scheme:` prefix was present.
    MissingScheme,
    /// The scheme contained characters outside `ALPHA *( ALPHA / DIGIT / +-. )`.
    InvalidScheme,
    /// A `safe` parse found a malformed `%XX` escape.
    InvalidEncoding(EncodingError),
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UriError::Empty => write!(f, "uri is empty"),
            UriError::MissingScheme => write!(f, "uri has no scheme"),
            UriError::InvalidScheme => write!(f, "uri scheme is invalid"),
            UriError::InvalidEncoding(e) => write!(f, "{e}"),
        }
    }
}

impl From<EncodingError> for UriError {
    fn from(e: EncodingError) -> Self {
        UriError::InvalidEncoding(e)
    }
}

impl std::error::Error for UriError {}

/// Error returned when [`Url::from_`] cannot interpret its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    /// The input was not even a valid [`Uri`].
    Uri(UriError),
    /// The URI had no `//authority` component (e.g. `mailto:foo@bar`).
    MissingAuthority,
    /// The authority had an empty host.
    MissingHost,
    /// The port was present but not a valid `u16`.
    InvalidPort(String),
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::Uri(e) => write!(f, "{e}"),
            UrlError::MissingAuthority => write!(f, "url has no authority"),
            UrlError::MissingHost => write!(f, "url has no host"),
            UrlError::InvalidPort(p) => write!(f, "url has an invalid port '{p}'"),
        }
    }
}

impl std::error::Error for UrlError {}

impl From<UriError> for UrlError {
    fn from(e: UriError) -> Self {
        UrlError::Uri(e)
    }
}

/// Returns `true` if `scheme` matches `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
fn is_valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// A generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) URI:
/// `scheme:[//authority]path[?query][#fragment]`.
///
/// ```
/// use yggdryl::{FromInput, Uri};
///
/// let uri = Uri::from_str("https://example.com/docs?page=2#intro", true).unwrap();
/// assert_eq!(uri.scheme(), "https");
/// assert_eq!(uri.authority(), Some("example.com"));
/// assert_eq!(uri.path(), "/docs");
/// assert_eq!(uri.query(), Some("page=2"));
/// assert_eq!(uri.fragment(), Some("intro"));
/// ```
#[derive(Debug, Default, Clone)]
pub struct Uri {
    scheme: String,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
    // Lazily-computed, cached encoded/decoded renderings (see `to_str`). Excluded
    // from equality so two URIs with the same components compare equal regardless
    // of which renderings have been materialised.
    encoded: OnceLock<String>,
    decoded: OnceLock<String>,
}

impl PartialEq for Uri {
    fn eq(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.authority == other.authority
            && self.path == other.path
            && self.query == other.query
            && self.fragment == other.fragment
    }
}

impl Eq for Uri {}

impl FromInput for Uri {
    type Err = UriError;

    /// Parses a string into a [`Uri`]. When `safe`, the scheme and any `%XX`
    /// escapes are validated; otherwise those checks are skipped.
    fn from_str(input: &str, safe: bool) -> Result<Uri, UriError> {
        if input.is_empty() {
            return Err(UriError::Empty);
        }

        // Peel off the fragment, then the query, from the right.
        let (rest, fragment) = split_once_owned(input, '#');
        let (rest, query) = split_once_owned(rest, '?');

        // The scheme is everything up to the first ':'.
        let colon = rest.find(':').ok_or(UriError::MissingScheme)?;
        let scheme = &rest[..colon];
        if scheme.is_empty() {
            return Err(UriError::MissingScheme);
        }
        if safe && !is_valid_scheme(scheme) {
            return Err(UriError::InvalidScheme);
        }

        // The hier-part: an optional `//authority` followed by the path.
        let after = &rest[colon + 1..];
        let (authority, path) = match after.strip_prefix("//") {
            Some(tail) => match tail.find('/') {
                Some(slash) => (Some(tail[..slash].to_string()), tail[slash..].to_string()),
                None => (Some(tail.to_string()), String::new()),
            },
            None => (None, after.to_string()),
        };

        let uri = Uri {
            scheme: scheme.to_string(),
            authority,
            path,
            query,
            fragment,
            ..Default::default()
        };
        if safe {
            uri.validate_encoding()?;
        }
        Ok(uri)
    }

    /// Builds a [`Uri`] from a [`Mapping`]. Recognised keys: `scheme` (required),
    /// `authority`, `path`, `query`, `fragment`.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<Uri, UriError> {
        let scheme = fields.get("scheme").ok_or(UriError::MissingScheme)?;
        if scheme.is_empty() {
            return Err(UriError::MissingScheme);
        }
        if safe && !is_valid_scheme(scheme) {
            return Err(UriError::InvalidScheme);
        }
        let uri = Uri {
            scheme: scheme.clone(),
            authority: fields.get("authority").cloned(),
            path: fields.get("path").cloned().unwrap_or_default(),
            query: fields.get("query").cloned(),
            fragment: fields.get("fragment").cloned(),
            ..Default::default()
        };
        if safe {
            uri.validate_encoding()?;
        }
        Ok(uri)
    }
}

impl Uri {
    /// Checks that every component is well-formed percent-encoding.
    fn validate_encoding(&self) -> Result<(), UriError> {
        for part in [self.authority.as_deref(), Some(self.path.as_str())]
            .into_iter()
            .chain([self.query.as_deref(), self.fragment.as_deref()])
            .flatten()
        {
            validate_percent_encoding(part)?;
        }
        Ok(())
    }

    /// The scheme, e.g. `"https"`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The authority (`userinfo@host:port`), if present.
    pub fn authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }

    /// The path, possibly empty.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The query string (without the leading `?`), if present.
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The fragment (without the leading `#`), if present.
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }
}

/// Constructors and functional `with_*` builders.
impl Uri {
    /// Builds a [`Uri`] from all of its parts.
    pub fn from_parts(
        scheme: String,
        authority: Option<String>,
        path: String,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Uri {
        Uri {
            scheme,
            authority,
            path,
            query,
            fragment,
            ..Default::default()
        }
    }

    /// Creates a minimal [`Uri`] from a scheme and path (no authority, query or
    /// fragment).
    pub fn new(scheme: impl Into<String>, path: impl Into<String>) -> Uri {
        Uri {
            scheme: scheme.into(),
            authority: None,
            path: path.into(),
            query: None,
            fragment: None,
            ..Default::default()
        }
    }

    /// Returns a copy of this URI, overriding any component for which `Some` is
    /// given and keeping `self`'s value otherwise. Call `copy(None, …)` to clone.
    pub fn copy(
        &self,
        scheme: Option<String>,
        authority: Option<String>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Uri {
        Uri {
            scheme: scheme.unwrap_or_else(|| self.scheme.clone()),
            authority: authority.or_else(|| self.authority.clone()),
            path: path.unwrap_or_else(|| self.path.clone()),
            query: query.or_else(|| self.query.clone()),
            fragment: fragment.or_else(|| self.fragment.clone()),
            ..Default::default()
        }
    }

    /// Returns a copy with the scheme replaced.
    pub fn with_scheme(self, scheme: impl Into<String>) -> Uri {
        Uri::from_parts(
            scheme.into(),
            self.authority,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the authority set.
    pub fn with_authority(self, authority: impl Into<String>) -> Uri {
        Uri::from_parts(
            self.scheme,
            Some(authority.into()),
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the authority removed.
    pub fn without_authority(self) -> Uri {
        Uri::from_parts(self.scheme, None, self.path, self.query, self.fragment)
    }

    /// Returns a copy with the path replaced.
    pub fn with_path(self, path: impl Into<String>) -> Uri {
        Uri::from_parts(
            self.scheme,
            self.authority,
            path.into(),
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the query set.
    pub fn with_query(self, query: impl Into<String>) -> Uri {
        Uri::from_parts(
            self.scheme,
            self.authority,
            self.path,
            Some(query.into()),
            self.fragment,
        )
    }

    /// Returns a copy with the query removed.
    pub fn without_query(self) -> Uri {
        Uri::from_parts(self.scheme, self.authority, self.path, None, self.fragment)
    }

    /// Returns a copy with the fragment set.
    pub fn with_fragment(self, fragment: impl Into<String>) -> Uri {
        Uri::from_parts(
            self.scheme,
            self.authority,
            self.path,
            self.query,
            Some(fragment.into()),
        )
    }

    /// Returns a copy with the fragment removed.
    pub fn without_fragment(self) -> Uri {
        Uri::from_parts(self.scheme, self.authority, self.path, self.query, None)
    }

    /// Returns the query parsed into a [`Params`] multimap. When `decode`, keys
    /// and values are percent-decoded. An absent query yields an empty map.
    pub fn params(&self, decode: bool) -> Params {
        self.query
            .as_deref()
            .map(|q| query_to_params(q, decode))
            .unwrap_or_default()
    }

    /// Returns a copy whose query is built from `params`. When `encode`, each key
    /// and value is percent-encoded. An empty map clears the query.
    pub fn with_params(self, params: &Params, encode: bool) -> Uri {
        let query = (!params.is_empty()).then(|| params_to_query(params, encode));
        Uri::from_parts(self.scheme, self.authority, self.path, query, self.fragment)
    }

    /// Returns a copy with `key` set to `values`, adding the parameter if absent
    /// or replacing its values if present. Values are percent-encoded when
    /// `encode`.
    pub fn add_param(&self, key: impl Into<String>, values: Vec<String>, encode: bool) -> Uri {
        let mut params = self.params(true);
        params.insert(key.into(), values);
        self.clone().with_params(&params, encode)
    }

    /// Renders the URI as a string. When `encode`, each component is percent-
    /// encoded for transport (idempotent); when not, components are percent-
    /// decoded for display. Both renderings are cached.
    pub fn to_str(&self, encode: bool) -> String {
        let cache = if encode { &self.encoded } else { &self.decoded };
        cache.get_or_init(|| self.render(encode)).clone()
    }

    /// Builds the rendering for [`Uri::to_str`].
    fn render(&self, encode: bool) -> String {
        let mut out = format!("{}:", self.scheme);
        if let Some(authority) = &self.authority {
            out.push_str("//");
            out.push_str(&render_component(authority, KEEP_AUTHORITY, encode));
        }
        out.push_str(&render_component(&self.path, KEEP_PATH, encode));
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(&render_component(query, KEEP_QUERY, encode));
        }
        if let Some(fragment) = &self.fragment {
            out.push('#');
            out.push_str(&render_component(fragment, KEEP_FRAGMENT, encode));
        }
        out
    }
}

impl fmt::Display for Uri {
    /// Renders the encoded form (`to_str(true)`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str(true))
    }
}

/// A URL: a [`Uri`] that always has an authority, with the authority broken out
/// into `username`, `password`, `host` and `port`.
///
/// ```
/// use yggdryl::{FromInput, Url};
///
/// let url = Url::from_str("https://user:pw@example.com:8443/api?v=1#top", true).unwrap();
/// assert_eq!(url.scheme(), "https");
/// assert_eq!(url.username(), Some("user"));
/// assert_eq!(url.password(), Some("pw"));
/// assert_eq!(url.host(), "example.com");
/// assert_eq!(url.port(), Some(8443));
/// assert_eq!(url.path(), "/api");
/// ```
#[derive(Debug, Default, Clone)]
pub struct Url {
    scheme: String,
    username: Option<String>,
    password: Option<String>,
    host: String,
    port: Option<u16>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
    // Cached encoded/decoded renderings; excluded from equality (see `Uri`).
    encoded: OnceLock<String>,
    decoded: OnceLock<String>,
}

impl PartialEq for Url {
    fn eq(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.username == other.username
            && self.password == other.password
            && self.host == other.host
            && self.port == other.port
            && self.path == other.path
            && self.query == other.query
            && self.fragment == other.fragment
    }
}

impl Eq for Url {}

impl FromInput for Url {
    type Err = UrlError;

    /// Parses a string into a [`Url`]. Requires a scheme, an authority and a
    /// non-empty host. `safe` is forwarded to the underlying [`Uri`] parse.
    fn from_str(input: &str, safe: bool) -> Result<Url, UrlError> {
        let uri = Uri::from_str(input, safe)?;
        let authority = uri.authority.ok_or(UrlError::MissingAuthority)?;

        // Split optional `userinfo@` from `host[:port]`.
        let (userinfo, host_port) = match authority.split_once('@') {
            Some((user, rest)) => (Some(user), rest),
            None => (None, authority.as_str()),
        };

        let (username, password) = match userinfo {
            Some(info) => match info.split_once(':') {
                Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
                None => (Some(info.to_string()), None),
            },
            None => (None, None),
        };

        let (host, port) = split_host_port(host_port)?;
        if host.is_empty() {
            return Err(UrlError::MissingHost);
        }

        Ok(Url {
            scheme: uri.scheme,
            username,
            password,
            host,
            port,
            path: uri.path,
            query: uri.query,
            fragment: uri.fragment,
            ..Default::default()
        })
    }

    /// Builds a [`Url`] from a [`Mapping`]. Recognised keys: `scheme` and `host`
    /// (required), `username`, `password`, `port`, `path`, `query`, `fragment`.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<Url, UrlError> {
        let scheme = fields
            .get("scheme")
            .ok_or(UrlError::Uri(UriError::MissingScheme))?;
        if safe && !is_valid_scheme(scheme) {
            return Err(UrlError::Uri(UriError::InvalidScheme));
        }
        let host = fields
            .get("host")
            .filter(|h| !h.is_empty())
            .ok_or(UrlError::MissingHost)?;
        let port = match fields.get("port") {
            Some(p) => Some(parse_port(p)?),
            None => None,
        };
        let url = Url {
            scheme: scheme.clone(),
            username: fields.get("username").cloned(),
            password: fields.get("password").cloned(),
            host: host.clone(),
            port,
            path: fields.get("path").cloned().unwrap_or_default(),
            query: fields.get("query").cloned(),
            fragment: fields.get("fragment").cloned(),
            ..Default::default()
        };
        if safe {
            for part in [url.path.as_str()]
                .into_iter()
                .chain(url.query.as_deref())
                .chain(url.fragment.as_deref())
            {
                validate_percent_encoding(part).map_err(UriError::from)?;
            }
        }
        Ok(url)
    }
}

impl Url {
    /// The scheme, e.g. `"https"`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The username from the userinfo, if present.
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// The password from the userinfo, if present.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// The host. For IPv6 literals this is the inner address without brackets,
    /// e.g. `"::1"`.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The explicit port, if one was given.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// The path, possibly empty.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The query string (without the leading `?`), if present.
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The fragment (without the leading `#`), if present.
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// Reconstructs the authority (`userinfo@host:port`) component.
    pub fn authority(&self) -> String {
        let mut out = String::new();
        if let Some(user) = &self.username {
            out.push_str(user);
            if let Some(pw) = &self.password {
                out.push(':');
                out.push_str(pw);
            }
            out.push('@');
        }
        push_host(&mut out, &self.host);
        if let Some(port) = self.port {
            out.push(':');
            out.push_str(&port.to_string());
        }
        out
    }
}

/// Constructors and functional `with_*` builders.
impl Url {
    /// Builds a [`Url`] from all of its parts.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        scheme: String,
        username: Option<String>,
        password: Option<String>,
        host: String,
        port: Option<u16>,
        path: String,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Url {
        Url {
            scheme,
            username,
            password,
            host,
            port,
            path,
            query,
            fragment,
            ..Default::default()
        }
    }

    /// Creates a minimal [`Url`] from a scheme and host.
    pub fn new(scheme: impl Into<String>, host: impl Into<String>) -> Url {
        Url {
            scheme: scheme.into(),
            host: host.into(),
            ..Default::default()
        }
    }

    /// Returns a copy of this URL, overriding any component for which `Some` is
    /// given and keeping `self`'s value otherwise. Call `copy(None, …)` to clone.
    #[allow(clippy::too_many_arguments)]
    pub fn copy(
        &self,
        scheme: Option<String>,
        username: Option<String>,
        password: Option<String>,
        host: Option<String>,
        port: Option<u16>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Url {
        Url::from_parts(
            scheme.unwrap_or_else(|| self.scheme.clone()),
            username.or_else(|| self.username.clone()),
            password.or_else(|| self.password.clone()),
            host.unwrap_or_else(|| self.host.clone()),
            port.or(self.port),
            path.unwrap_or_else(|| self.path.clone()),
            query.or_else(|| self.query.clone()),
            fragment.or_else(|| self.fragment.clone()),
        )
    }

    /// Returns a copy with the scheme replaced.
    pub fn with_scheme(self, scheme: impl Into<String>) -> Url {
        Url::from_parts(
            scheme.into(),
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the username set.
    pub fn with_username(self, username: impl Into<String>) -> Url {
        Url::from_parts(
            self.scheme,
            Some(username.into()),
            self.password,
            self.host,
            self.port,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the password set.
    pub fn with_password(self, password: impl Into<String>) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            Some(password.into()),
            self.host,
            self.port,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with username and password removed.
    pub fn without_userinfo(self) -> Url {
        Url::from_parts(
            self.scheme,
            None,
            None,
            self.host,
            self.port,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the host replaced.
    pub fn with_host(self, host: impl Into<String>) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            host.into(),
            self.port,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the port set.
    pub fn with_port(self, port: u16) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            Some(port),
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the port removed.
    pub fn without_port(self) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            None,
            self.path,
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the path replaced.
    pub fn with_path(self, path: impl Into<String>) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            path.into(),
            self.query,
            self.fragment,
        )
    }

    /// Returns a copy with the query set.
    pub fn with_query(self, query: impl Into<String>) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            Some(query.into()),
            self.fragment,
        )
    }

    /// Returns a copy with the query removed.
    pub fn without_query(self) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            None,
            self.fragment,
        )
    }

    /// Returns a copy with the fragment set.
    pub fn with_fragment(self, fragment: impl Into<String>) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            self.query,
            Some(fragment.into()),
        )
    }

    /// Returns a copy with the fragment removed.
    pub fn without_fragment(self) -> Url {
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            self.query,
            None,
        )
    }

    /// Returns the query parsed into a [`Params`] multimap. When `decode`, keys
    /// and values are percent-decoded. An absent query yields an empty map.
    pub fn params(&self, decode: bool) -> Params {
        self.query
            .as_deref()
            .map(|q| query_to_params(q, decode))
            .unwrap_or_default()
    }

    /// Returns a copy whose query is built from `params`. When `encode`, each key
    /// and value is percent-encoded. An empty map clears the query.
    pub fn with_params(self, params: &Params, encode: bool) -> Url {
        let query = (!params.is_empty()).then(|| params_to_query(params, encode));
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            query,
            self.fragment,
        )
    }

    /// Returns a copy with `key` set to `values`, adding the parameter if absent
    /// or replacing its values if present. Values are percent-encoded when
    /// `encode`.
    pub fn add_param(&self, key: impl Into<String>, values: Vec<String>, encode: bool) -> Url {
        let mut params = self.params(true);
        params.insert(key.into(), values);
        self.clone().with_params(&params, encode)
    }

    /// Views this URL as a generic [`Uri`] — the "is-a URI" relationship, since
    /// every [`Url`] is a [`Uri`] with a reconstructed authority.
    pub fn to_uri(&self) -> Uri {
        Uri::from_parts(
            self.scheme.clone(),
            Some(self.authority()),
            self.path.clone(),
            self.query.clone(),
            self.fragment.clone(),
        )
    }

    /// Renders the URL as a string. When `encode`, each component is percent-
    /// encoded for transport (idempotent); when not, components are percent-
    /// decoded for display. Both renderings are cached.
    pub fn to_str(&self, encode: bool) -> String {
        let cache = if encode { &self.encoded } else { &self.decoded };
        cache.get_or_init(|| self.render(encode)).clone()
    }

    /// Builds the rendering for [`Url::to_str`].
    fn render(&self, encode: bool) -> String {
        let mut out = format!("{}://", self.scheme);
        if let Some(user) = &self.username {
            out.push_str(&render_component(user, KEEP_AUTHORITY, encode));
            if let Some(pw) = &self.password {
                out.push(':');
                out.push_str(&render_component(pw, KEEP_AUTHORITY, encode));
            }
            out.push('@');
        }
        push_host(
            &mut out,
            &render_component(&self.host, KEEP_AUTHORITY, encode),
        );
        if let Some(port) = self.port {
            out.push(':');
            out.push_str(&port.to_string());
        }
        out.push_str(&render_component(&self.path, KEEP_PATH, encode));
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(&render_component(query, KEEP_QUERY, encode));
        }
        if let Some(fragment) = &self.fragment {
            out.push('#');
            out.push_str(&render_component(fragment, KEEP_FRAGMENT, encode));
        }
        out
    }
}

impl From<&Url> for Uri {
    fn from(url: &Url) -> Uri {
        url.to_uri()
    }
}

impl From<Url> for Uri {
    fn from(url: Url) -> Uri {
        url.to_uri()
    }
}

impl fmt::Display for Url {
    /// Renders the encoded form (`to_str(true)`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str(true))
    }
}

/// Splits `input` on the first `sep`, returning the part before and the
/// owned part after (or `None` if `sep` is absent).
fn split_once_owned(input: &str, sep: char) -> (&str, Option<String>) {
    match input.split_once(sep) {
        Some((head, tail)) => (head, Some(tail.to_string())),
        None => (input, None),
    }
}

/// Splits a `host[:port]` (handling `[IPv6]` literals) into its host and port.
fn split_host_port(input: &str) -> Result<(String, Option<u16>), UrlError> {
    if let Some(after_open) = input.strip_prefix('[') {
        // IPv6 literal: `[host]` optionally followed by `:port`.
        let close = after_open
            .find(']')
            .ok_or_else(|| UrlError::InvalidPort(input.to_string()))?;
        let host = after_open[..close].to_string();
        let tail = &after_open[close + 1..];
        let port = match tail.strip_prefix(':') {
            Some(p) => Some(parse_port(p)?),
            None if tail.is_empty() => None,
            None => return Err(UrlError::InvalidPort(input.to_string())),
        };
        Ok((host, port))
    } else {
        match input.rsplit_once(':') {
            Some((host, port)) => Ok((host.to_string(), Some(parse_port(port)?))),
            None => Ok((input.to_string(), None)),
        }
    }
}

/// Parses a port, treating anything that is not a `u16` as an error.
fn parse_port(port: &str) -> Result<u16, UrlError> {
    port.parse::<u16>()
        .map_err(|_| UrlError::InvalidPort(port.to_string()))
}

/// Appends a host to `out`, wrapping IPv6 literals (those containing `:`) in
/// brackets.
fn push_host(out: &mut String, host: &str) {
    if host.contains(':') {
        out.push('[');
        out.push_str(host);
        out.push(']');
    } else {
        out.push_str(host);
    }
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
/// use yggdryl::{FromInput, Version};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_full() {
        let uri = Uri::from_str("https://example.com/docs?page=2#intro", true).unwrap();
        assert_eq!(uri.scheme(), "https");
        assert_eq!(uri.authority(), Some("example.com"));
        assert_eq!(uri.path(), "/docs");
        assert_eq!(uri.query(), Some("page=2"));
        assert_eq!(uri.fragment(), Some("intro"));
    }

    #[test]
    fn uri_without_authority() {
        let uri = Uri::from_str("mailto:alice@example.com", true).unwrap();
        assert_eq!(uri.scheme(), "mailto");
        assert_eq!(uri.authority(), None);
        assert_eq!(uri.path(), "alice@example.com");
    }

    #[test]
    fn uri_errors() {
        assert_eq!(Uri::from_str("", true), Err(UriError::Empty));
        assert_eq!(
            Uri::from_str("no-scheme/path", true),
            Err(UriError::MissingScheme)
        );
        assert_eq!(
            Uri::from_str(":no-scheme", true),
            Err(UriError::MissingScheme)
        );
        assert_eq!(
            Uri::from_str("1http://x", true),
            Err(UriError::InvalidScheme)
        );
    }

    #[test]
    fn uri_round_trips() {
        for input in [
            "https://example.com/docs?page=2#intro",
            "mailto:alice@example.com",
            "file:///etc/hosts",
            "urn:isbn:0451450523",
        ] {
            assert_eq!(Uri::from_str(input, true).unwrap().to_string(), input);
        }
    }

    #[test]
    fn uri_unsafe_skips_scheme_validation() {
        // An invalid scheme is rejected when safe, accepted when not.
        assert_eq!(Uri::from_str("1http:x", true), Err(UriError::InvalidScheme));
        assert_eq!(Uri::from_str("1http:x", false).unwrap().scheme(), "1http");
    }

    #[test]
    fn uri_safe_validates_percent_encoding() {
        assert!(Uri::from_str("http://h/a%zz", true).is_err());
        // A bad escape is tolerated by the fast path.
        assert_eq!(
            Uri::from_str("http://h/a%zz", false).unwrap().path(),
            "/a%zz"
        );
        // Well-formed escapes always pass.
        assert!(Uri::from_str("http://h/a%20b", true).is_ok());
    }

    #[test]
    fn uri_from_mapping() {
        let fields = Mapping::from([
            ("scheme".to_string(), "https".to_string()),
            ("authority".to_string(), "example.com".to_string()),
            ("path".to_string(), "/x".to_string()),
        ]);
        let uri = Uri::from_(&fields, true).unwrap();
        assert_eq!(uri.to_string(), "https://example.com/x");
    }

    #[test]
    fn url_full() {
        let url = Url::from_str("https://user:pw@example.com:8443/api?v=1#top", true).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.username(), Some("user"));
        assert_eq!(url.password(), Some("pw"));
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), Some(8443));
        assert_eq!(url.path(), "/api");
        assert_eq!(url.query(), Some("v=1"));
        assert_eq!(url.fragment(), Some("top"));
    }

    #[test]
    fn url_minimal() {
        let url = Url::from_str("http://example.com", true).unwrap();
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), None);
        assert_eq!(url.username(), None);
        assert_eq!(url.path(), "");
    }

    #[test]
    fn url_username_only() {
        let url = Url::from_str("ftp://anon@files.example.com/pub", true).unwrap();
        assert_eq!(url.username(), Some("anon"));
        assert_eq!(url.password(), None);
        assert_eq!(url.host(), "files.example.com");
    }

    #[test]
    fn url_ipv6() {
        let url = Url::from_str("http://[::1]:8080/status", true).unwrap();
        assert_eq!(url.host(), "::1");
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.to_string(), "http://[::1]:8080/status");
    }

    #[test]
    fn url_errors() {
        assert_eq!(
            Url::from_str("mailto:alice@example.com", true),
            Err(UrlError::MissingAuthority)
        );
        assert_eq!(
            Url::from_str("http://user@:80", true),
            Err(UrlError::MissingHost)
        );
        assert!(matches!(
            Url::from_str("http://example.com:notaport", true),
            Err(UrlError::InvalidPort(_))
        ));
        assert_eq!(
            Url::from_str("notauri", true),
            Err(UrlError::Uri(UriError::MissingScheme))
        );
    }

    #[test]
    fn url_round_trips() {
        for input in [
            "https://user:pw@example.com:8443/api?v=1#top",
            "http://example.com",
            "ftp://anon@files.example.com/pub",
            "http://[::1]:8080/status",
        ] {
            assert_eq!(Url::from_str(input, true).unwrap().to_string(), input);
        }
    }

    #[test]
    fn url_authority_is_reconstructed() {
        let url = Url::from_str("https://user:pw@example.com:8443/api", true).unwrap();
        assert_eq!(url.authority(), "user:pw@example.com:8443");
    }

    #[test]
    fn url_from_mapping() {
        let fields = Mapping::from([
            ("scheme".to_string(), "https".to_string()),
            ("host".to_string(), "example.com".to_string()),
            ("port".to_string(), "8443".to_string()),
            ("path".to_string(), "/api".to_string()),
        ]);
        let url = Url::from_mapping(&fields, true).unwrap();
        assert_eq!(url.to_string(), "https://example.com:8443/api");

        let missing_host = Mapping::from([("scheme".to_string(), "https".to_string())]);
        assert_eq!(
            Url::from_mapping(&missing_host, true),
            Err(UrlError::MissingHost)
        );
    }

    #[test]
    fn version_parse_full() {
        let v = Version::from_str("1.4.2", true).unwrap();
        assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
    }

    #[test]
    fn version_parse_partial_defaults_to_zero() {
        assert_eq!(Version::from_str("2", true).unwrap(), Version::new(2, 0, 0));
        assert_eq!(
            Version::from_str("2.5", true).unwrap(),
            Version::new(2, 5, 0)
        );
    }

    #[test]
    fn version_errors() {
        assert_eq!(Version::from_str("", true), Err(VersionError::Empty));
        assert_eq!(
            Version::from_str("1.2.3.4", true),
            Err(VersionError::TooManyComponents)
        );
        assert_eq!(
            Version::from_str("1.x.0", true),
            Err(VersionError::InvalidNumber("x".to_string()))
        );
        assert_eq!(
            Version::from_str("1..0", true),
            Err(VersionError::InvalidNumber(String::new()))
        );
    }

    #[test]
    fn version_unsafe_is_lenient() {
        // Fast path ignores extra components and treats junk as zero.
        assert_eq!(
            Version::from_str("1.2.3.4", false).unwrap(),
            Version::new(1, 2, 3)
        );
        assert_eq!(
            Version::from_str("1.x.0", false).unwrap(),
            Version::new(1, 0, 0)
        );
    }

    #[test]
    fn version_from_mapping() {
        let fields = Mapping::from([
            ("major".to_string(), "1".to_string()),
            ("minor".to_string(), "4".to_string()),
        ]);
        assert_eq!(
            Version::from_mapping(&fields, true).unwrap(),
            Version::new(1, 4, 0)
        );
    }

    #[test]
    fn version_orders_numerically() {
        assert!(Version::new(1, 4, 2) < Version::new(1, 10, 0));
        assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
        let mut versions = [
            Version::new(1, 2, 0),
            Version::new(1, 0, 5),
            Version::new(0, 9, 9),
        ];
        versions.sort();
        assert_eq!(
            versions,
            [
                Version::new(0, 9, 9),
                Version::new(1, 0, 5),
                Version::new(1, 2, 0),
            ]
        );
    }

    #[test]
    fn version_round_trips() {
        assert_eq!(
            Version::from_str("1.4.2", true).unwrap().to_string(),
            "1.4.2"
        );
        assert_eq!(Version::from_str("3", true).unwrap().to_string(), "3.0.0");
    }

    #[test]
    fn uri_constructors_and_builders() {
        let uri = Uri::new("https", "/docs")
            .with_authority("example.com")
            .with_query("page=2")
            .with_fragment("intro");
        assert_eq!(uri.to_string(), "https://example.com/docs?page=2#intro");

        // with_*/without_* leave the original untouched (functional style).
        let bare = uri.clone().without_query().without_fragment();
        assert_eq!(bare.to_string(), "https://example.com/docs");
        assert_eq!(uri.query(), Some("page=2"));

        let from_parts = Uri::from_parts("ftp".into(), Some("h".into()), "/p".into(), None, None);
        assert_eq!(from_parts.to_string(), "ftp://h/p");
    }

    #[test]
    fn copy_overrides_selected_fields() {
        let uri = Uri::new("https", "/a").with_authority("h");
        // Override one field; the rest come from `self`.
        let moved = uri.copy(None, None, Some("/b".into()), None, None);
        assert_eq!(moved.to_string(), "https://h/b");
        // copy(None, …) is a plain clone.
        assert_eq!(uri.copy(None, None, None, None, None), uri);

        let v = Version::new(1, 4, 2);
        assert_eq!(v.copy(Some(2), None, None), Version::new(2, 4, 2));
    }

    #[test]
    fn url_constructors_and_builders() {
        let url = Url::new("https", "example.com")
            .with_port(8443)
            .with_username("user")
            .with_password("pw")
            .with_path("/api");
        assert_eq!(url.to_string(), "https://user:pw@example.com:8443/api");

        let public = url.clone().without_userinfo().without_port();
        assert_eq!(public.to_string(), "https://example.com/api");
        assert_eq!(url.port(), Some(8443));
    }

    #[test]
    fn version_builders() {
        let v = Version::new(1, 0, 0).with_minor(4).with_patch(2);
        assert_eq!(v, Version::new(1, 4, 2));
        assert_eq!(v.with_major(2), Version::new(2, 4, 2));
        assert_eq!(v, Version::new(1, 4, 2));
    }

    #[test]
    fn params_round_trip_and_multivalue() {
        let url = Url::from_str("https://h/p?a=1&a=2&b=hello%20world", true).unwrap();
        let params = url.params(true);
        assert_eq!(
            params.get("a"),
            Some(&vec!["1".to_string(), "2".to_string()])
        );
        assert_eq!(params.get("b"), Some(&vec!["hello world".to_string()]));

        // Building a query encodes each part.
        let built = Uri::new("https", "/p").with_params(
            &Params::from([("q".to_string(), vec!["a b".to_string()])]),
            true,
        );
        assert_eq!(built.query(), Some("q=a%20b"));

        // add_param adds a new key or replaces an existing one's values.
        let updated = built.add_param("q", vec!["x".to_string(), "y".to_string()], true);
        assert_eq!(updated.query(), Some("q=x&q=y"));
        let added = updated.add_param("r", vec!["1".to_string()], true);
        assert_eq!(added.query(), Some("q=x&q=y&r=1"));
    }

    #[test]
    fn to_str_encode_decode() {
        let url = Url::from_str("https://h/a%20b?q=x%20y#f%20g", false).unwrap();
        // encode=true ensures a transport-safe string (idempotent, no double-encode).
        assert_eq!(url.to_str(true), "https://h/a%20b?q=x%20y#f%20g");
        // encode=false decodes each component for display.
        assert_eq!(url.to_str(false), "https://h/a b?q=x y#f g");
        // Display uses the encoded form.
        assert_eq!(url.to_string(), "https://h/a%20b?q=x%20y#f%20g");

        // A space in a freshly-built component is encoded by to_str(true).
        // (no authority, so a single slash for the path)
        assert_eq!(Uri::new("https", "/a b").to_str(true), "https:/a%20b");
    }

    #[test]
    fn url_is_a_uri() {
        let url = Url::from_str("https://user@h:8443/p?x=1#f", true).unwrap();
        let uri: Uri = url.to_uri();
        assert_eq!(uri.scheme(), "https");
        assert_eq!(uri.authority(), Some("user@h:8443"));
        assert_eq!(uri.path(), "/p");
        assert_eq!(uri.to_string(), "https://user@h:8443/p?x=1#f");
        // `From` conversions are available too.
        let via_from = Uri::from(&url);
        assert_eq!(via_from, uri);
    }

    #[test]
    fn percent_encode_decode_round_trip() {
        assert_eq!(percent_encode("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(percent_decode("a%20b%2Fc%3Fd").unwrap(), "a b/c?d");
        assert_eq!(percent_encode("safe-._~"), "safe-._~");
        assert!(percent_decode("%zz").is_err());
        assert!(percent_decode("%2").is_err());
    }
}
