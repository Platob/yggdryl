//! The [`Url`] value type — a [`Uri`] that always has an authority, decomposed
//! into `username` / `password` / `host` / `port` (and its [`UrlError`]).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

#[allow(unused_imports)]
use crate::log_event;
use crate::url::{
    build_query, is_valid_scheme, join_path, path_name, path_parts, query_param, query_to_params,
    render_component, split_stem_ext, JoinInput, KEEP_AUTHORITY, KEEP_FRAGMENT, KEEP_PATH,
    KEEP_QUERY,
};
use crate::{validate_percent_encoding, MediaType, MimeType, Params, Uri, UriError};

/// Error returned when [`Url`] parsing cannot interpret its input.
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

/// A URL: a [`Uri`] that always has an authority, with the authority broken out
/// into `username`, `password`, `host` and `port`.
///
/// ```
/// use yggdryl_core::Url;
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

/// String/mapping parsers.
impl Url {
    /// Parses a string into a [`Url`]. Requires a scheme, an authority and a
    /// non-empty host.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Url, UrlError> {
        log_event!(trace, "Url::from_str {input:?}");
        Url::from_uri(&Uri::from_str(input)?)
    }

    /// Builds a [`Url`] from a `BTreeMap`. Recognised keys: `scheme` and `host`
    /// (required), `username`, `password`, `port`, `path`, `query`, `fragment`.
    pub fn from_mapping(fields: &BTreeMap<String, String>) -> Result<Url, UrlError> {
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
        Url::from_parts(
            self.scheme,
            self.username,
            self.password,
            self.host,
            self.port,
            self.path,
            build_query(params, encode),
            self.fragment,
        )
    }

    /// Clones the non-query fields once and attaches `query`. The query mutators
    /// use this instead of `self.clone()` so the replaced (old) query — which we
    /// fully own and are about to discard — is never cloned.
    fn cloned_with_query(&self, query: Option<String>) -> Url {
        Url::from_parts(
            self.scheme.clone(),
            self.username.clone(),
            self.password.clone(),
            self.host.clone(),
            self.port,
            self.path.clone(),
            query,
            self.fragment.clone(),
        )
    }

    /// Returns a copy with `key` set to `values`, adding the parameter if absent
    /// or replacing its values if present. Values are percent-encoded when
    /// `encode`.
    pub fn add_param(&self, key: impl Into<String>, values: Vec<String>, encode: bool) -> Url {
        let mut params = self.params(true);
        params.insert(key.into(), values);
        self.cloned_with_query(build_query(&params, encode))
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
        self.rendered(encode).to_string()
    }

    /// The cached rendering as a borrowed `&str` (computed once per `encode`),
    /// shared by [`to_str`](Url::to_str) and the [`Display`] impl.
    fn rendered(&self, encode: bool) -> &str {
        let cache = if encode { &self.encoded } else { &self.decoded };
        cache.get_or_init(|| self.render(encode))
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
        self.cloned_with_query(build_query(&current, encode))
    }

    /// Returns a copy with one parameter removed (single delete).
    pub fn remove_param(&self, key: &str, encode: bool) -> Url {
        let mut current = self.params(true);
        current.remove(key);
        self.cloned_with_query(build_query(&current, encode))
    }

    /// Returns a copy with several parameters removed (bulk delete).
    pub fn remove_params(&self, keys: &[String], encode: bool) -> Url {
        let mut current = self.params(true);
        for key in keys {
            current.remove(key);
        }
        self.cloned_with_query(build_query(&current, encode))
    }

    /// Returns a copy with the entire query removed.
    pub fn clear_params(&self) -> Url {
        self.cloned_with_query(None)
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

impl From<&Url> for MediaType {
    /// Builds the [`MediaType`] stack from the URL's path (see [`Url::media_type`]).
    fn from(url: &Url) -> MediaType {
        MediaType::from_path(&url.path)
    }
}

/// Serialises as the encoded URL string, the inverse of [`Url::from_str`].
#[cfg(feature = "serde")]
impl serde::Serialize for Url {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Url {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Url, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Url::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// Path joining (see [`Uri::join`]).
impl Url {
    /// Returns a copy whose path is `reference` resolved against `self`'s path
    /// (RFC 3986 §5.2.4). See [`Uri::join`] for the input forms (a path string, a
    /// sequence of segments, or another reference) and semantics; the authority
    /// (userinfo / host / port) is preserved, the query and fragment dropped.
    ///
    /// ```
    /// use yggdryl_core::Url;
    ///
    /// let base = Url::from_str("https://user@h:8443/a/b/c?k=v#f").unwrap();
    /// assert_eq!(base.join("../x").to_string(), "https://user@h:8443/a/x");
    /// assert_eq!(base.join(["d", "e f"]).path(), "/a/b/d/e%20f");
    /// ```
    pub fn join(&self, reference: impl JoinInput) -> Url {
        // A `Url` always has an authority, so the RFC 3986 §5.2.3 empty-path rule
        // applies (an authority-only base roots the reference).
        let path = join_path(&self.path, reference.to_reference().as_ref(), true);
        Url::from_parts(
            self.scheme.clone(),
            self.username.clone(),
            self.password.clone(),
            self.host.clone(),
            self.port,
            path,
            None,
            None,
        )
    }
}

impl fmt::Display for Url {
    /// Renders the encoded form, writing the cached rendering directly (no clone).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.rendered(true))
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

/// Component rendering (the inherent [`to_str`](Url::to_str) lives with the other
/// builders above).
impl Url {
    /// The inverse of `from_mapping`: keys `scheme`, `host` and any of
    /// `username`, `password`, `port`, `path`, `query`, `fragment` that are set.
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::from([
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
