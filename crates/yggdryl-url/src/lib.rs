//! # yggdryl-url
//!
//! URI and URL value types for the **yggdryl** project, built on the
//! [`yggdryl-core`](https://crates.io/crates/yggdryl-core) foundations.
//!
//! - [`Uri`] is the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
//!   shape: `scheme:[//authority]path[?query][#fragment]`.
//! - [`Url`] is the common subset that always has an authority, decomposed into
//!   `username`, `password`, `host` and `port`.
//!
//! The shared [`FromInput`]/[`ToOutput`] traits, the [`Mapping`]/[`Params`] types
//! and the percent-encoding helpers are re-exported from `yggdryl-core`. The
//! `Version` type lives in the separate `yggdryl-version` crate.

use std::borrow::Cow;
use std::fmt;
use std::sync::OnceLock;

use yggdryl_core::{encode_component, validate_percent_encoding};

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate is dependency-free by default and pays no runtime cost).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

// Per-component sets of delimiter bytes that are left as-is when encoding a
// component for output (on top of the always-safe unreserved set).
pub(crate) const KEEP_AUTHORITY: &[u8] = b":@[]";
pub(crate) const KEEP_PATH: &[u8] = b"/:@";
pub(crate) const KEEP_QUERY: &[u8] = b"/:@?&=";
pub(crate) const KEEP_FRAGMENT: &[u8] = b"/:@?";

/// Renders a component either percent-encoded (`encode`) or percent-decoded
/// (best effort), used by `to_str(encode)`. Borrows `input` when nothing changes.
pub(crate) fn render_component<'a>(input: &'a str, keep: &[u8], encode: bool) -> Cow<'a, str> {
    if encode {
        encode_component(input, keep)
    } else {
        percent_decode(input).unwrap_or(Cow::Borrowed(input))
    }
}

/// Splits a `key=value&key=value2` query into a multimap. Repeated keys
/// accumulate their values; when `decode`, each key/value is percent-decoded
/// (parts that fail to decode are kept verbatim).
pub(crate) fn query_to_params(query: &str, decode: bool) -> Params {
    let unescape = |s: &str| -> String {
        if decode {
            percent_decode(s)
                .map(Cow::into_owned)
                .unwrap_or_else(|_| s.to_string())
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

/// Scans a query for a single (decoded) `key`, returning its decoded values, or
/// `None` if absent. Avoids building the whole [`Params`] map for a one-key
/// lookup ([`Uri::get_param`] / [`Uri::has_param`]).
pub(crate) fn query_param(query: &str, key: &str) -> Option<Vec<String>> {
    // Compare the raw key without allocating unless it carries an escape.
    let key_matches = |raw: &str| {
        if raw.contains('%') {
            percent_decode(raw).is_ok_and(|decoded| decoded == key)
        } else {
            raw == key
        }
    };
    let mut values: Option<Vec<String>> = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if key_matches(k) {
            let value = percent_decode(v)
                .map(Cow::into_owned)
                .unwrap_or_else(|_| v.to_string());
            values.get_or_insert_with(Vec::new).push(value);
        }
    }
    values
}

/// Builds a `key=value&…` query from a [`Params`] multimap. When `encode`, each
/// key/value is percent-encoded. Keys with several values are emitted once per
/// value; pairs come out in the map's (sorted) order for a deterministic result.
pub(crate) fn params_to_query(params: &Params, encode: bool) -> String {
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

// Re-exported so a dependent only needs `yggdryl-url`.
pub use yggdryl_core::{
    percent_decode, percent_encode, EncodingError, FromInput, Input, Mapping, Output, Params,
    ToOutput,
};
pub use yggdryl_media::{MediaError, MediaType, MimeType, Signature};

/// Error returned when [`Uri`] parsing cannot interpret its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// The input was empty.
    Empty,
    /// No `scheme:` prefix was present.
    MissingScheme,
    /// The scheme contained characters outside `ALPHA *( ALPHA / DIGIT / +-. )`.
    InvalidScheme,
    /// A malformed `%XX` escape was found.
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
/// use yggdryl_url::{FromInput, Uri};
///
/// let uri = Uri::from_str("https://example.com/docs?page=2#intro").unwrap();
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

    /// Parses a string into a [`Uri`].
    ///
    /// Windows-style `\` separators are normalised to `/`. If no scheme is given
    /// the input is treated as a path with the `file` scheme (a single-letter
    /// "scheme" is read as a Windows drive letter, also `file`). The scheme and
    /// any `%XX` escapes are validated.
    fn from_str(input: &str) -> Result<Uri, UriError> {
        log_event!(trace, "Uri::from_str {input:?}");
        if input.is_empty() {
            return Err(UriError::Empty);
        }
        // Normalise Windows separators so paths use `/` everywhere. Only allocate
        // when a backslash is actually present (the uncommon case).
        let normalized;
        let input = if input.contains('\\') {
            normalized = input.replace('\\', "/");
            normalized.as_str()
        } else {
            input
        };

        // Peel off the fragment, then the query, from the right.
        let (rest, fragment) = split_once_owned(input, '#');
        let (rest, query) = split_once_owned(rest, '?');

        // A scheme is a `:` that comes before the first `/`.
        let colon = rest.find(':');
        let slash = rest.find('/');
        let has_scheme = matches!(colon, Some(c) if slash.is_none_or(|s| c < s));

        let (scheme, authority, path) = if !has_scheme {
            // No scheme: default to `file`, the whole input is the path.
            log_event!(
                warn,
                "Uri::from_str: no scheme in {input:?}, defaulting to 'file'"
            );
            ("file".to_string(), None, rest.to_string())
        } else {
            let colon = colon.expect("has_scheme implies a colon");
            let raw_scheme = &rest[..colon];
            if raw_scheme.is_empty() {
                return Err(UriError::MissingScheme);
            }
            if raw_scheme.len() == 1 && raw_scheme.as_bytes()[0].is_ascii_alphabetic() {
                // A single-letter "scheme" is a Windows drive letter, e.g. `C:`.
                log_event!(
                    warn,
                    "Uri::from_str: single-letter scheme {raw_scheme:?} treated as a Windows drive (scheme 'file')"
                );
                let path = if rest.starts_with('/') {
                    rest.to_string()
                } else {
                    format!("/{rest}")
                };
                ("file".to_string(), None, path)
            } else {
                if !is_valid_scheme(raw_scheme) {
                    return Err(UriError::InvalidScheme);
                }
                // The hier-part: an optional `//authority` followed by the path.
                let after = &rest[colon + 1..];
                let (authority, path) = match after.strip_prefix("//") {
                    Some(tail) => match tail.find('/') {
                        Some(s) => (Some(tail[..s].to_string()), tail[s..].to_string()),
                        None => (Some(tail.to_string()), String::new()),
                    },
                    None => (None, after.to_string()),
                };
                (raw_scheme.to_string(), authority, path)
            }
        };

        let uri = Uri {
            scheme,
            authority,
            path,
            query,
            fragment,
            ..Default::default()
        };
        uri.validate_encoding()?;
        Ok(uri)
    }

    /// Builds a [`Uri`] from a [`Mapping`]. Recognised keys: `scheme` (required),
    /// `authority`, `path`, `query`, `fragment`.
    fn from_mapping(fields: &Mapping) -> Result<Uri, UriError> {
        // A missing scheme defaults to `file`; an empty one is an error.
        let scheme = match fields.get("scheme") {
            Some(s) if s.is_empty() => return Err(UriError::MissingScheme),
            Some(s) => s.clone(),
            None => {
                log_event!(warn, "Uri::from_mapping: no scheme, defaulting to 'file'");
                "file".to_string()
            }
        };
        if !is_valid_scheme(&scheme) {
            return Err(UriError::InvalidScheme);
        }
        let uri = Uri {
            scheme,
            authority: fields.get("authority").cloned(),
            path: fields.get("path").cloned().unwrap_or_default(),
            query: fields.get("query").cloned(),
            fragment: fields.get("fragment").cloned(),
            ..Default::default()
        };
        uri.validate_encoding()?;
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

/// Scheme extensions, conversions and query-parameter CRUD.
impl Uri {
    /// The base scheme before any `+` extension, e.g. `"https"` for `"https+zip"`.
    pub fn scheme_base(&self) -> &str {
        self.scheme.split('+').next().unwrap_or(&self.scheme)
    }

    /// The `+`-separated scheme extensions, e.g. `["zip"]` for `"https+zip"` and
    /// `[]` for a plain `"https"`.
    pub fn scheme_ext(&self) -> Vec<&str> {
        self.scheme.split('+').skip(1).collect()
    }

    /// Builds a [`Uri`] from a [`Url`].
    pub fn from_url(url: &Url) -> Uri {
        url.to_uri()
    }

    /// Parses this URI into a [`Url`] (requires an authority with a non-empty
    /// host).
    pub fn to_url(&self) -> Result<Url, UrlError> {
        Url::from_uri(self)
    }

    /// The decoded values of one query parameter, or `None` if absent.
    pub fn get_param(&self, key: &str) -> Option<Vec<String>> {
        self.query.as_deref().and_then(|q| query_param(q, key))
    }

    /// Whether the query contains a parameter named `key`.
    pub fn has_param(&self, key: &str) -> bool {
        self.get_param(key).is_some()
    }

    /// Returns a copy with one parameter created or replaced (single update).
    pub fn set_param(&self, key: impl Into<String>, values: Vec<String>, encode: bool) -> Uri {
        self.add_param(key, values, encode)
    }

    /// Returns a copy with every entry of `params` created or replaced, leaving
    /// other parameters untouched (bulk update).
    pub fn set_params(&self, params: &Params, encode: bool) -> Uri {
        let mut current = self.params(true);
        current.extend(params.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.clone().with_params(&current, encode)
    }

    /// Returns a copy with one parameter removed (single delete).
    pub fn remove_param(&self, key: &str, encode: bool) -> Uri {
        let mut current = self.params(true);
        current.remove(key);
        self.clone().with_params(&current, encode)
    }

    /// Returns a copy with several parameters removed (bulk delete).
    pub fn remove_params(&self, keys: &[String], encode: bool) -> Uri {
        let mut current = self.params(true);
        for key in keys {
            current.remove(key);
        }
        self.clone().with_params(&current, encode)
    }

    /// Returns a copy with the entire query removed.
    pub fn clear_params(&self) -> Uri {
        self.clone().without_query()
    }
}

impl fmt::Display for Uri {
    /// Renders the encoded form (`to_str(true)`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str(true))
    }
}

/// Renders one path component: percent-decoded unless `encode` keeps it as-is.
fn render_part(part: &str, encode: bool) -> String {
    if encode {
        part.to_string()
    } else {
        percent_decode(part)
            .map(Cow::into_owned)
            .unwrap_or_else(|_| part.to_string())
    }
}

/// Splits a `/`-separated path into its non-empty, rendered segments.
fn path_parts(path: &str, encode: bool) -> Vec<String> {
    path.split('/')
        .filter(|p| !p.is_empty())
        .map(|p| render_part(p, encode))
        .collect()
}

/// The last non-empty path segment (the file name), rendered.
fn path_name(path: &str, encode: bool) -> String {
    path.rsplit('/')
        .find(|p| !p.is_empty())
        .map(|p| render_part(p, encode))
        .unwrap_or_default()
}

/// Splits a file name into `(stem, extensions)` at the first non-leading `.`, so
/// `"a.tar.gz"` → `("a", ["tar", "gz"])` and `".bashrc"` → `(".bashrc", [])`.
fn split_stem_ext(name: &str) -> (&str, Vec<&str>) {
    let dot = if name.len() > 1 {
        name[1..].find('.').map(|i| i + 1)
    } else {
        None
    };
    match dot {
        Some(idx) => (&name[..idx], name[idx + 1..].split('.').collect()),
        None => (name, Vec::new()),
    }
}

/// Path-segment accessors. `encode` (default `false` in the bindings) keeps the
/// percent-encoded form; otherwise each piece is decoded.
impl Uri {
    /// The non-empty `/`-separated path segments.
    pub fn parts(&self, encode: bool) -> Vec<String> {
        path_parts(&self.path, encode)
    }

    /// The file name: the last non-empty path segment.
    pub fn name(&self, encode: bool) -> String {
        path_name(&self.path, encode)
    }

    /// The file name without its extensions.
    pub fn stem(&self, encode: bool) -> String {
        split_stem_ext(&self.name(encode)).0.to_string()
    }

    /// The file name's extensions, e.g. `["tar", "gz"]` for `archive.tar.gz`.
    pub fn extensions(&self, encode: bool) -> Vec<String> {
        split_stem_ext(&self.name(encode))
            .1
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// The [`MediaType`] stack inferred from the path's file extensions (e.g.
    /// `[Csv, Gzip]` for `data.csv.gz`), or `None` if no known extension is found.
    pub fn media_type(&self) -> Option<MediaType> {
        let media = MediaType::from(self);
        (!media.is_empty()).then_some(media)
    }

    /// The outermost [`MimeType`] inferred from the path's last known file
    /// extension (e.g. `Gzip` for `data.csv.gz`), or `None`.
    pub fn mime_type(&self) -> Option<MimeType> {
        MimeType::from_path(&self.path)
    }
}

impl ToOutput for Uri {
    fn to_str(&self, encode: bool) -> String {
        Uri::to_str(self, encode)
    }

    /// The inverse of `from_mapping`: keys `scheme`, `authority`, `path`,
    /// `query`, `fragment` (only the present components).
    fn to_mapping(&self) -> Mapping {
        let mut map = Mapping::from([("scheme".to_string(), self.scheme.clone())]);
        if let Some(authority) = &self.authority {
            map.insert("authority".to_string(), authority.clone());
        }
        if !self.path.is_empty() {
            map.insert("path".to_string(), self.path.clone());
        }
        if let Some(query) = &self.query {
            map.insert("query".to_string(), query.clone());
        }
        if let Some(fragment) = &self.fragment {
            map.insert("fragment".to_string(), fragment.clone());
        }
        map
    }
}

/// A URL: a [`Uri`] that always has an authority, with the authority broken out
/// into `username`, `password`, `host` and `port`.
///
/// ```
/// use yggdryl_url::{FromInput, Url};
///
/// let url = Url::from_str("https://user:pw@example.com:8443/api?v=1#top").unwrap();
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
    /// non-empty host.
    fn from_str(input: &str) -> Result<Url, UrlError> {
        log_event!(trace, "Url::from_str {input:?}");
        Url::from_uri(&Uri::from_str(input)?)
    }

    /// Builds a [`Url`] from a [`Mapping`]. Recognised keys: `scheme` and `host`
    /// (required), `username`, `password`, `port`, `path`, `query`, `fragment`.
    fn from_mapping(fields: &Mapping) -> Result<Url, UrlError> {
        // A missing scheme defaults to `file`.
        let scheme = match fields.get("scheme") {
            Some(s) => s.as_str(),
            None => {
                log_event!(warn, "Url::from_mapping: no scheme, defaulting to 'file'");
                "file"
            }
        };
        if !is_valid_scheme(scheme) {
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
            scheme: scheme.to_string(),
            username: fields.get("username").cloned(),
            password: fields.get("password").cloned(),
            host: host.clone(),
            port,
            path: fields.get("path").cloned().unwrap_or_default(),
            query: fields.get("query").cloned(),
            fragment: fields.get("fragment").cloned(),
            ..Default::default()
        };
        for part in [url.path.as_str()]
            .into_iter()
            .chain(url.query.as_deref())
            .chain(url.fragment.as_deref())
        {
            validate_percent_encoding(part).map_err(UriError::from)?;
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

/// Conversions, scheme extensions and query-parameter CRUD.
impl Url {
    /// Builds a [`Url`] from a [`Uri`] by decomposing its authority. Requires an
    /// authority with a non-empty host.
    pub fn from_uri(uri: &Uri) -> Result<Url, UrlError> {
        let authority = uri.authority().ok_or(UrlError::MissingAuthority)?;

        // Split optional `userinfo@` from `host[:port]`.
        let (userinfo, host_port) = match authority.split_once('@') {
            Some((user, rest)) => (Some(user), rest),
            None => (None, authority),
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
            scheme: uri.scheme().to_string(),
            username,
            password,
            host,
            port,
            path: uri.path().to_string(),
            query: uri.query().map(str::to_string),
            fragment: uri.fragment().map(str::to_string),
            ..Default::default()
        })
    }

    /// The base scheme before any `+` extension, e.g. `"https"` for `"https+zip"`.
    pub fn scheme_base(&self) -> &str {
        self.scheme.split('+').next().unwrap_or(&self.scheme)
    }

    /// The `+`-separated scheme extensions, e.g. `["zip"]` for `"https+zip"`.
    pub fn scheme_ext(&self) -> Vec<&str> {
        self.scheme.split('+').skip(1).collect()
    }

    /// The decoded values of one query parameter, or `None` if absent.
    pub fn get_param(&self, key: &str) -> Option<Vec<String>> {
        self.query.as_deref().and_then(|q| query_param(q, key))
    }

    /// Whether the query contains a parameter named `key`.
    pub fn has_param(&self, key: &str) -> bool {
        self.get_param(key).is_some()
    }

    /// Returns a copy with one parameter created or replaced (single update).
    pub fn set_param(&self, key: impl Into<String>, values: Vec<String>, encode: bool) -> Url {
        self.add_param(key, values, encode)
    }

    /// Returns a copy with every entry of `params` created or replaced, leaving
    /// other parameters untouched (bulk update).
    pub fn set_params(&self, params: &Params, encode: bool) -> Url {
        let mut current = self.params(true);
        current.extend(params.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.clone().with_params(&current, encode)
    }

    /// Returns a copy with one parameter removed (single delete).
    pub fn remove_param(&self, key: &str, encode: bool) -> Url {
        let mut current = self.params(true);
        current.remove(key);
        self.clone().with_params(&current, encode)
    }

    /// Returns a copy with several parameters removed (bulk delete).
    pub fn remove_params(&self, keys: &[String], encode: bool) -> Url {
        let mut current = self.params(true);
        for key in keys {
            current.remove(key);
        }
        self.clone().with_params(&current, encode)
    }

    /// Returns a copy with the entire query removed.
    pub fn clear_params(&self) -> Url {
        self.clone().without_query()
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

impl From<&Uri> for MediaType {
    /// Builds the [`MediaType`] stack from the URI's path (see [`Uri::media_type`]).
    fn from(uri: &Uri) -> MediaType {
        MediaType::from_path(&uri.path)
    }
}

impl From<&Url> for MediaType {
    /// Builds the [`MediaType`] stack from the URL's path (see [`Url::media_type`]).
    fn from(url: &Url) -> MediaType {
        MediaType::from_path(&url.path)
    }
}

impl fmt::Display for Url {
    /// Renders the encoded form (`to_str(true)`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str(true))
    }
}

/// Path-segment accessors (see [`Uri`] for `encode` semantics).
impl Url {
    /// The non-empty `/`-separated path segments.
    pub fn parts(&self, encode: bool) -> Vec<String> {
        path_parts(&self.path, encode)
    }

    /// The file name: the last non-empty path segment.
    pub fn name(&self, encode: bool) -> String {
        path_name(&self.path, encode)
    }

    /// The file name without its extensions.
    pub fn stem(&self, encode: bool) -> String {
        split_stem_ext(&self.name(encode)).0.to_string()
    }

    /// The file name's extensions, e.g. `["tar", "gz"]` for `archive.tar.gz`.
    pub fn extensions(&self, encode: bool) -> Vec<String> {
        split_stem_ext(&self.name(encode))
            .1
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// The [`MediaType`] stack inferred from the path's file extensions (e.g.
    /// `[Csv, Gzip]` for `data.csv.gz`), or `None` if no known extension is found.
    pub fn media_type(&self) -> Option<MediaType> {
        let media = MediaType::from(self);
        (!media.is_empty()).then_some(media)
    }

    /// The outermost [`MimeType`] inferred from the path's last known file
    /// extension (e.g. `Gzip` for `data.csv.gz`), or `None`.
    pub fn mime_type(&self) -> Option<MimeType> {
        MimeType::from_path(&self.path)
    }
}

impl ToOutput for Url {
    fn to_str(&self, encode: bool) -> String {
        Url::to_str(self, encode)
    }

    /// The inverse of `from_mapping`: keys `scheme`, `host` and any of
    /// `username`, `password`, `port`, `path`, `query`, `fragment` that are set.
    fn to_mapping(&self) -> Mapping {
        let mut map = Mapping::from([
            ("scheme".to_string(), self.scheme.clone()),
            ("host".to_string(), self.host.clone()),
        ]);
        if let Some(username) = &self.username {
            map.insert("username".to_string(), username.clone());
        }
        if let Some(password) = &self.password {
            map.insert("password".to_string(), password.clone());
        }
        if let Some(port) = self.port {
            map.insert("port".to_string(), port.to_string());
        }
        if !self.path.is_empty() {
            map.insert("path".to_string(), self.path.clone());
        }
        if let Some(query) = &self.query {
            map.insert("query".to_string(), query.clone());
        }
        if let Some(fragment) = &self.fragment {
            map.insert("fragment".to_string(), fragment.clone());
        }
        map
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_read_edge_cases() {
        // Encoded key, multi-value, empty value, and a bare flag (no `=`).
        let u = Uri::from_str("http://h/p?a%20b=1&c=2&c=3&d=&flag").unwrap();
        assert_eq!(u.get_param("a b"), Some(vec!["1".to_string()])); // key decoded
        assert_eq!(
            u.get_param("c"),
            Some(vec!["2".to_string(), "3".to_string()])
        );
        assert_eq!(u.get_param("d"), Some(vec![String::new()])); // empty value
        assert_eq!(u.get_param("flag"), Some(vec![String::new()])); // no `=`
        assert_eq!(u.get_param("missing"), None);
        assert!(u.has_param("a b") && u.has_param("flag") && !u.has_param("missing"));
        // No query at all.
        let bare = Uri::from_str("http://h/p").unwrap();
        assert_eq!(bare.get_param("x"), None);
        assert!(!bare.has_param("x"));
        // get_param agrees with the full params() map.
        assert_eq!(u.get_param("c"), u.params(true).get("c").cloned());
    }

    #[test]
    fn path_and_scheme_edge_cases() {
        // Trailing/leading/double slashes collapse to non-empty segments.
        let u = Uri::from_str("http://h//a//b/").unwrap();
        assert_eq!(u.parts(false), vec!["a", "b"]);
        assert_eq!(u.name(false), "b");
        // A dotfile has no extensions; its stem keeps the leading dot.
        let dot = Uri::from_str("file:/etc/.bashrc").unwrap();
        assert_eq!(dot.stem(false), ".bashrc");
        assert!(dot.extensions(false).is_empty());
        // Multi-`+` scheme splits into base + every extension.
        let s = Uri::from_str("a+b+c://h/p").unwrap();
        assert_eq!(s.scheme_base(), "a");
        assert_eq!(s.scheme_ext(), vec!["b", "c"]);
        // No path -> empty accessors, no panic.
        let empty = Uri::from_str("mailto:x@y").unwrap();
        assert_eq!(empty.name(false), "x@y");
        assert!(empty.parts(false).len() == 1);
    }

    #[test]
    fn uri_edge_cases() {
        // Empty input.
        assert_eq!(Uri::from_str(""), Err(UriError::Empty));
        // A `?`/`#` inside the query/fragment is kept (split is leftmost only).
        let u = Uri::from_str("http://h/p?a=b?c#x#y").unwrap();
        assert_eq!(u.query(), Some("a=b?c"));
        assert_eq!(u.fragment(), Some("x#y"));
        // A `:` after the first `/` is part of the path, not a scheme.
        assert_eq!(Uri::from_str("a/b:c").unwrap().scheme(), "file");
        // Empty query/fragment are preserved as `Some("")`.
        let e = Uri::from_str("http://h/p?#").unwrap();
        assert_eq!(e.query(), Some(""));
        assert_eq!(e.fragment(), Some(""));
        // Scheme-only with empty authority.
        let bare = Uri::from_str("http://").unwrap();
        assert_eq!(bare.authority(), Some(""));
        assert_eq!(bare.path(), "");
        // A trailing `%` is a malformed escape.
        assert!(Uri::from_str("http://h/a%").is_err());
        assert!(Uri::from_str("http://h/a%2").is_err());
    }

    #[test]
    fn url_edge_cases() {
        // Port out of the u16 range is rejected.
        assert!(matches!(
            Url::from_str("http://h:99999"),
            Err(UrlError::InvalidPort(_))
        ));
        // An unclosed IPv6 literal is rejected.
        assert!(Url::from_str("http://[::1/p").is_err());
        // Userinfo with an `@`-free password and an empty path round-trips.
        let u = Url::from_str("http://user:pa:ss@h:1/").unwrap();
        assert_eq!(u.username(), Some("user"));
        assert_eq!(u.password(), Some("pa:ss"));
        assert_eq!(u.port(), Some(1));
        // Empty host is rejected even with userinfo.
        assert_eq!(Url::from_str("http://u@:80"), Err(UrlError::MissingHost));
        // IPv6 with zone-id-like content round-trips through brackets.
        assert_eq!(
            Url::from_str("http://[::1]:8080/").unwrap().to_string(),
            "http://[::1]:8080/"
        );
    }

    #[test]
    fn uri_full() {
        let uri = Uri::from_str("https://example.com/docs?page=2#intro").unwrap();
        assert_eq!(uri.scheme(), "https");
        assert_eq!(uri.authority(), Some("example.com"));
        assert_eq!(uri.path(), "/docs");
        assert_eq!(uri.query(), Some("page=2"));
        assert_eq!(uri.fragment(), Some("intro"));
    }

    #[test]
    fn uri_without_authority() {
        let uri = Uri::from_str("mailto:alice@example.com").unwrap();
        assert_eq!(uri.scheme(), "mailto");
        assert_eq!(uri.authority(), None);
        assert_eq!(uri.path(), "alice@example.com");
    }

    #[test]
    fn uri_errors() {
        assert_eq!(Uri::from_str(""), Err(UriError::Empty));
        // An empty (but present) scheme is still an error.
        assert_eq!(Uri::from_str(":no-scheme"), Err(UriError::MissingScheme));
        assert_eq!(Uri::from_str("1http://x"), Err(UriError::InvalidScheme));
    }

    #[test]
    fn path_accessors() {
        let url = Url::from_str("https://h/a/b/archive.tar.gz").unwrap();
        assert_eq!(url.parts(false), vec!["a", "b", "archive.tar.gz"]);
        assert_eq!(url.name(false), "archive.tar.gz");
        assert_eq!(url.stem(false), "archive");
        assert_eq!(url.extensions(false), vec!["tar", "gz"]);
        // encode flag: decoded by default, kept when true.
        let enc = Uri::from_str("file:/dir/a%20b.txt").unwrap();
        assert_eq!(enc.name(false), "a b.txt");
        assert_eq!(enc.name(true), "a%20b.txt");
        // no extension and dotfiles.
        assert_eq!(
            Uri::from_str("file:/x/README").unwrap().extensions(false),
            Vec::<String>::new()
        );
        assert_eq!(
            Uri::from_str("file:/x/.bashrc").unwrap().stem(false),
            ".bashrc"
        );
    }

    #[test]
    fn media_type_inference() {
        // A single extension yields a one-element stack.
        assert_eq!(
            Url::from_str("https://h/a/data.json")
                .unwrap()
                .media_type()
                .unwrap()
                .types(),
            [MimeType::Json]
        );
        // Compound extensions yield the ordered stack (content first).
        assert_eq!(
            Uri::from_str("file:/dump/archive.tar.gz")
                .unwrap()
                .media_type()
                .unwrap()
                .types(),
            [MimeType::Tar, MimeType::Gzip]
        );
        // No (known) extension yields `None`.
        assert_eq!(Url::from_str("https://h/page").unwrap().media_type(), None);
        // `mime_type()` is the single outermost type; `From<&Uri>` mirrors it.
        let uri = Uri::from_str("file:/dump/archive.tar.gz").unwrap();
        assert_eq!(uri.mime_type(), Some(MimeType::Gzip));
        assert_eq!(
            MediaType::from(&uri).types(),
            [MimeType::Tar, MimeType::Gzip]
        );
        assert_eq!(Url::from_str("https://h/page").unwrap().mime_type(), None);
    }

    #[test]
    fn default_file_scheme_and_windows_paths() {
        // No scheme -> file.
        let u = Uri::from_str("no-scheme/path").unwrap();
        assert_eq!(u.scheme(), "file");
        assert_eq!(u.path(), "no-scheme/path");
        // A `:` after a `/` is part of the path, not a scheme.
        assert_eq!(Uri::from_str("a/b:c").unwrap().scheme(), "file");
        // Backslashes are normalised to `/`.
        let w = Uri::from_str("dir\\sub\\file").unwrap();
        assert_eq!(w.scheme(), "file");
        assert_eq!(w.path(), "dir/sub/file");
        // A drive letter is a Windows path -> file.
        let d = Uri::from_str("C:\\Users\\me").unwrap();
        assert_eq!(d.scheme(), "file");
        assert_eq!(d.path(), "/C:/Users/me");
    }

    #[test]
    fn uri_round_trips() {
        for input in [
            "https://example.com/docs?page=2#intro",
            "mailto:alice@example.com",
            "file:///etc/hosts",
            "urn:isbn:0451450523",
        ] {
            assert_eq!(Uri::from_str(input).unwrap().to_string(), input);
        }
    }

    #[test]
    fn uri_validates_scheme() {
        // An invalid scheme is always rejected.
        assert_eq!(Uri::from_str("1http:x"), Err(UriError::InvalidScheme));
    }

    #[test]
    fn uri_validates_percent_encoding() {
        // A malformed `%XX` escape is rejected; well-formed escapes pass.
        assert!(Uri::from_str("http://h/a%zz").is_err());
        assert!(Uri::from_str("http://h/a%20b").is_ok());
    }

    #[test]
    fn uri_from_mapping() {
        let fields = Mapping::from([
            ("scheme".to_string(), "https".to_string()),
            ("authority".to_string(), "example.com".to_string()),
            ("path".to_string(), "/x".to_string()),
        ]);
        let uri = Uri::from_(&fields).unwrap();
        assert_eq!(uri.to_string(), "https://example.com/x");
    }

    #[test]
    fn url_full() {
        let url = Url::from_str("https://user:pw@example.com:8443/api?v=1#top").unwrap();
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
        let url = Url::from_str("http://example.com").unwrap();
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), None);
        assert_eq!(url.username(), None);
        assert_eq!(url.path(), "");
    }

    #[test]
    fn url_username_only() {
        let url = Url::from_str("ftp://anon@files.example.com/pub").unwrap();
        assert_eq!(url.username(), Some("anon"));
        assert_eq!(url.password(), None);
        assert_eq!(url.host(), "files.example.com");
    }

    #[test]
    fn url_ipv6() {
        let url = Url::from_str("http://[::1]:8080/status").unwrap();
        assert_eq!(url.host(), "::1");
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.to_string(), "http://[::1]:8080/status");
    }

    #[test]
    fn url_errors() {
        assert_eq!(
            Url::from_str("mailto:alice@example.com"),
            Err(UrlError::MissingAuthority)
        );
        assert_eq!(Url::from_str("http://user@:80"), Err(UrlError::MissingHost));
        assert!(matches!(
            Url::from_str("http://example.com:notaport"),
            Err(UrlError::InvalidPort(_))
        ));
        // No scheme defaults to `file`, but a Url still needs an authority.
        assert_eq!(Url::from_str("notauri"), Err(UrlError::MissingAuthority));
    }

    #[test]
    fn url_round_trips() {
        for input in [
            "https://user:pw@example.com:8443/api?v=1#top",
            "http://example.com",
            "ftp://anon@files.example.com/pub",
            "http://[::1]:8080/status",
        ] {
            assert_eq!(Url::from_str(input).unwrap().to_string(), input);
        }
    }

    #[test]
    fn url_authority_is_reconstructed() {
        let url = Url::from_str("https://user:pw@example.com:8443/api").unwrap();
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
        let url = Url::from_mapping(&fields).unwrap();
        assert_eq!(url.to_string(), "https://example.com:8443/api");

        let missing_host = Mapping::from([("scheme".to_string(), "https".to_string())]);
        assert_eq!(Url::from_mapping(&missing_host), Err(UrlError::MissingHost));
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
    fn params_round_trip_and_multivalue() {
        let url = Url::from_str("https://h/p?a=1&a=2&b=hello%20world").unwrap();
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
    fn scheme_extensions() {
        let uri = Uri::from_str("https+zip://h/f").unwrap();
        assert_eq!(uri.scheme(), "https+zip");
        assert_eq!(uri.scheme_base(), "https");
        assert_eq!(uri.scheme_ext(), vec!["zip"]);
        let plain = Uri::from_str("https://h").unwrap();
        assert_eq!(plain.scheme_base(), "https");
        assert!(plain.scheme_ext().is_empty());
        // works on Url too
        let url = Url::from_str("git+ssh://h/r").unwrap();
        assert_eq!(url.scheme_base(), "git");
        assert_eq!(url.scheme_ext(), vec!["ssh"]);
    }

    #[test]
    fn uri_url_conversions() {
        let url = Url::from_str("https://user@h:8443/p?x=1").unwrap();
        let uri = Uri::from_url(&url);
        assert_eq!(uri.authority(), Some("user@h:8443"));
        // round-trip back to Url
        let back = uri.to_url().unwrap();
        assert_eq!(back, url);
        // a Uri without authority cannot become a Url
        assert_eq!(
            Uri::from_str("mailto:a@b").unwrap().to_url(),
            Err(UrlError::MissingAuthority)
        );
        // Url::from_uri mirrors to_url
        assert_eq!(Url::from_uri(&uri).unwrap(), url);
    }

    #[test]
    fn params_crud_single_and_bulk() {
        let base = Url::from_str("https://h/p?a=1&b=2&c=3").unwrap();
        assert_eq!(base.get_param("a"), Some(vec!["1".to_string()]));
        assert_eq!(base.get_param("z"), None);
        assert!(base.has_param("a") && !base.has_param("z"));

        // single update
        assert_eq!(
            base.set_param("a", vec!["9".into()], true).get_param("a"),
            Some(vec!["9".to_string()])
        );

        // bulk update (b replaced, d added, a/c kept)
        let bulk = base.set_params(
            &Params::from([
                ("b".to_string(), vec!["x".to_string()]),
                ("d".to_string(), vec!["y".to_string()]),
            ]),
            true,
        );
        assert_eq!(bulk.get_param("b"), Some(vec!["x".to_string()]));
        assert_eq!(bulk.get_param("d"), Some(vec!["y".to_string()]));
        assert_eq!(bulk.get_param("a"), Some(vec!["1".to_string()]));

        // single + bulk delete, and clear
        assert_eq!(base.remove_param("a", true).get_param("a"), None);
        let trimmed = base.remove_params(&["a".to_string(), "b".to_string()], true);
        assert_eq!(
            trimmed.params(true).keys().cloned().collect::<Vec<_>>(),
            vec!["c".to_string()]
        );
        assert_eq!(base.clear_params().query(), None);
    }

    #[test]
    fn to_str_encode_decode() {
        let url = Url::from_str("https://h/a%20b?q=x%20y#f%20g").unwrap();
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
        let url = Url::from_str("https://user@h:8443/p?x=1#f").unwrap();
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
