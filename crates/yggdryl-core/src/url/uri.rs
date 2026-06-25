//! The generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) [`Uri`] value
//! type (and its [`UriError`]).

use std::fmt;
use std::sync::OnceLock;

#[allow(unused_imports)]
use crate::log_event;
use crate::url::{
    build_query, is_valid_scheme, path_name, path_parts, query_param, query_to_params,
    render_component, split_stem_ext, KEEP_AUTHORITY, KEEP_FRAGMENT, KEEP_PATH, KEEP_QUERY,
};
use crate::{
    validate_percent_encoding, EncodingError, Mapping, MediaType, MimeType, Params, Url, UrlError,
};

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

/// A generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) URI:
/// `scheme:[//authority]path[?query][#fragment]`.
///
/// ```
/// use yggdryl_core::Uri;
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

/// String/mapping parsers.
impl Uri {
    /// Parses a string into a [`Uri`].
    ///
    /// Windows-style `\` separators are normalised to `/`. If no scheme is given
    /// the input is treated as a path with the `file` scheme (a single-letter
    /// "scheme" is read as a Windows drive letter, also `file`). The scheme and
    /// any `%XX` escapes are validated.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Uri, UriError> {
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
    pub fn from_mapping(fields: &Mapping) -> Result<Uri, UriError> {
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
        Uri::from_parts(
            self.scheme,
            self.authority,
            self.path,
            build_query(params, encode),
            self.fragment,
        )
    }

    /// Clones the non-query fields once and attaches `query`. The query mutators
    /// use this instead of `self.clone()` so the replaced (old) query — which we
    /// fully own and are about to discard — is never cloned.
    fn cloned_with_query(&self, query: Option<String>) -> Uri {
        Uri::from_parts(
            self.scheme.clone(),
            self.authority.clone(),
            self.path.clone(),
            query,
            self.fragment.clone(),
        )
    }

    /// Returns a copy with `key` set to `values`, adding the parameter if absent
    /// or replacing its values if present. Values are percent-encoded when
    /// `encode`.
    pub fn add_param(&self, key: impl Into<String>, values: Vec<String>, encode: bool) -> Uri {
        let mut params = self.params(true);
        params.insert(key.into(), values);
        self.cloned_with_query(build_query(&params, encode))
    }

    /// Renders the URI as a string. When `encode`, each component is percent-
    /// encoded for transport (idempotent); when not, components are percent-
    /// decoded for display. Both renderings are cached.
    pub fn to_str(&self, encode: bool) -> String {
        self.rendered(encode).to_string()
    }

    /// The cached rendering as a borrowed `&str` (computed once per `encode`),
    /// shared by [`to_str`](Uri::to_str) and the [`Display`] impl so neither
    /// re-renders nor clones beyond the caller's needs.
    fn rendered(&self, encode: bool) -> &str {
        let cache = if encode { &self.encoded } else { &self.decoded };
        cache.get_or_init(|| self.render(encode))
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
        self.cloned_with_query(build_query(&current, encode))
    }

    /// Returns a copy with one parameter removed (single delete).
    pub fn remove_param(&self, key: &str, encode: bool) -> Uri {
        let mut current = self.params(true);
        current.remove(key);
        self.cloned_with_query(build_query(&current, encode))
    }

    /// Returns a copy with several parameters removed (bulk delete).
    pub fn remove_params(&self, keys: &[String], encode: bool) -> Uri {
        let mut current = self.params(true);
        for key in keys {
            current.remove(key);
        }
        self.cloned_with_query(build_query(&current, encode))
    }

    /// Returns a copy with the entire query removed.
    pub fn clear_params(&self) -> Uri {
        self.cloned_with_query(None)
    }
}

impl fmt::Display for Uri {
    /// Renders the encoded form, writing the cached rendering directly (no clone).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.rendered(true))
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

/// Component rendering (the inherent [`to_str`](Uri::to_str) lives with the other
/// builders above).
impl Uri {
    /// The inverse of `from_mapping`: keys `scheme`, `authority`, `path`,
    /// `query`, `fragment` (only the present components).
    pub fn to_mapping(&self) -> Mapping {
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

impl From<&Uri> for MediaType {
    /// Builds the [`MediaType`] stack from the URI's path (see [`Uri::media_type`]).
    fn from(uri: &Uri) -> MediaType {
        MediaType::from_path(&uri.path)
    }
}

/// Serialises as the encoded URI string, the inverse of [`Uri::from_str`].
#[cfg(feature = "serde")]
impl serde::Serialize for Uri {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Uri {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Uri, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Uri::from_str(&raw).map_err(serde::de::Error::custom)
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
