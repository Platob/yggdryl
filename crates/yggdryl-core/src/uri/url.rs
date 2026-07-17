//! [`Url`] — an absolute URI (one that carries a scheme).

use core::fmt;
use std::borrow::Cow;

use super::{Authority, Uri, UriError};

/// An **absolute** URI: a [`Uri`] that is guaranteed to carry a scheme.
///
/// DESIGN: "absolute" here means *scheme-present*, matching RFC 3986's notion of an
/// absolute-URI. The authority stays **optional** — `mailto:user@host` and `file:/etc/x`
/// are valid `Url`s with no `//` authority — so only the scheme is required. A `Url` wraps
/// a `Uri` so the two share one parser and one canonical form; the only added invariant is
/// the present scheme, which is why [`scheme`](Url::scheme) returns `&str`, not `Option`.
///
/// ```
/// use yggdryl_core::uri::Url;
///
/// let url = Url::parse_str("https://example.com/a/b.txt").unwrap();
/// assert_eq!(url.scheme(), "https");
/// assert_eq!(url.host(), "example.com");
/// assert_eq!(url.name(), Some("b.txt"));
///
/// // A scheme-less input is not an absolute URL.
/// assert!(Url::parse_str("/relative/path").is_err());
/// ```
#[derive(Debug, Clone)]
pub struct Url {
    inner: Uri,
}

impl Url {
    /// Parses `s` into an absolute URL.
    ///
    /// # Errors
    /// [`UriError::MissingScheme`] if `s` has no scheme, plus any [`Uri::parse_str`] error.
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// assert_eq!(Url::parse_str("sc://h/p").unwrap().scheme(), "sc");
    /// ```
    pub fn parse_str(s: &str) -> Result<Url, UriError> {
        Url::try_from(Uri::parse_str(s)?)
    }

    /// The scheme (always present).
    pub fn scheme(&self) -> &str {
        // Guaranteed `Some` by the constructor invariant.
        self.inner.scheme().unwrap_or_default()
    }

    /// The authority — **total**: an empty [`Authority`] when the URL has none (a `mailto:` /
    /// `file:` URL). A URL almost always has one, so this returns a value rather than an `Option`;
    /// test presence explicitly with [`has_authority`](Url::has_authority).
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// assert_eq!(Url::parse_str("https://h:8080/p").unwrap().authority().port(), Some(8080));
    /// assert_eq!(Url::parse_str("mailto:a@b.com").unwrap().authority().host(), ""); // no authority
    /// ```
    pub fn authority(&self) -> Authority {
        self.inner.authority().cloned().unwrap_or_default()
    }

    /// Whether this URL carries a `//` authority (`false` for `mailto:` / `file:/path`).
    pub fn has_authority(&self) -> bool {
        self.inner.authority().is_some()
    }

    /// The userinfo user, if any.
    pub fn user(&self) -> Option<&str> {
        self.inner.user()
    }

    /// The userinfo password, if any.
    pub fn password(&self) -> Option<&str> {
        self.inner.password()
    }

    /// The host — **total**: an empty string when the URL has no authority (an IPv6 literal keeps
    /// its brackets). A URL almost always has a host, so this returns a `&str` rather than an
    /// `Option`; `host().is_empty()` (or [`has_authority`](Url::has_authority)) tests presence.
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// assert_eq!(Url::parse_str("https://example.com/p").unwrap().host(), "example.com");
    /// assert_eq!(Url::parse_str("mailto:a@b.com").unwrap().host(), "");
    /// ```
    pub fn host(&self) -> &str {
        self.inner.host().unwrap_or("")
    }

    /// Whether the host is a bracketed IPv6 literal — see [`Uri::host_is_ipv6`].
    pub fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped, if this URL has an authority — see
    /// [`Uri::host_unbracketed`].
    pub fn host_unbracketed(&self) -> Option<&str> {
        self.inner.host_unbracketed()
    }

    /// The port as written, if any. For the port to actually connect to use
    /// [`port_or_default`](Url::port_or_default).
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// The default port registered for this URL's scheme, or `None` if it has no known
    /// default — see [`default_port`](crate::uri::default_port).
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// assert_eq!(Url::parse_str("wss://h/s").unwrap().default_port(), Some(443));
    /// assert_eq!(Url::parse_str("s3://bucket/key").unwrap().default_port(), None);
    /// ```
    pub fn default_port(&self) -> Option<u16> {
        self.inner.default_port()
    }

    /// The **effective** port to connect to: the explicit [`port`](Url::port), else the
    /// scheme's [`default_port`](Url::default_port) — see [`Uri::port_or_default`].
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// assert_eq!(Url::parse_str("https://h/p").unwrap().port_or_default(), Some(443));
    /// assert_eq!(Url::parse_str("http://h:8080/p").unwrap().port_or_default(), Some(8080));
    /// ```
    pub fn port_or_default(&self) -> Option<u16> {
        self.inner.port_or_default()
    }

    /// The path, always POSIX slash-normalized.
    pub fn path(&self) -> &str {
        self.inner.path()
    }

    /// The query, if any.
    pub fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    /// The fragment, if any.
    pub fn fragment(&self) -> Option<&str> {
        self.inner.fragment()
    }

    /// The last non-empty path segment (the filename), or `None` for a directory-like path.
    pub fn name(&self) -> Option<&str> {
        self.inner.name()
    }

    /// The filename without its last extension.
    pub fn stem(&self) -> Option<&str> {
        self.inner.stem()
    }

    /// The last extension of the filename (without the dot).
    pub fn extension(&self) -> Option<&str> {
        self.inner.extension()
    }

    /// Every extension of a multi-dot filename, outermost-last.
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    // ---- query parameters (map access + CRUD) --------------------------------------

    /// The first value of query parameter `key`, or `None` — see [`Uri::param`].
    pub fn param(&self, key: &str) -> Option<&str> {
        self.inner.param(key)
    }

    /// Every value of query parameter `key`, in order — see [`Uri::param_all`].
    pub fn param_all(&self, key: &str) -> Vec<&str> {
        self.inner.param_all(key)
    }

    /// All query parameters as ordered `(key, value)` pairs — see [`Uri::params`].
    pub fn params(&self) -> Vec<(&str, &str)> {
        self.inner.params()
    }

    /// All query parameters **grouped by key** (each key → all its values) — see
    /// [`Uri::params_grouped`].
    pub fn params_grouped(&self) -> Vec<(&str, Vec<&str>)> {
        self.inner.params_grouped()
    }

    /// The first value of query parameter `key`, percent-decoded — see
    /// [`Uri::param_decoded`].
    pub fn param_decoded(&self, key: &str) -> Option<Cow<'_, str>> {
        self.inner.param_decoded(key)
    }

    /// Every value of query parameter `key`, percent-decoded — see
    /// [`Uri::param_all_decoded`].
    pub fn param_all_decoded(&self, key: &str) -> Vec<Cow<'_, str>> {
        self.inner.param_all_decoded(key)
    }

    /// All query parameters as `(key, value)` pairs, percent-decoded — see
    /// [`Uri::params_decoded`].
    pub fn params_decoded(&self) -> Vec<(Cow<'_, str>, Cow<'_, str>)> {
        self.inner.params_decoded()
    }

    /// Whether query parameter `key` is present — see [`Uri::has_param`].
    pub fn has_param(&self, key: &str) -> bool {
        self.inner.has_param(key)
    }

    /// Sets query parameter `key` to `value` (map semantics) — see [`Uri::set_param`].
    pub fn set_param(&mut self, key: &str, value: &str) {
        self.inner.set_param(key, value);
    }

    /// Returns this URL with query parameter `key` set — see [`Uri::with_param`].
    pub fn with_param(mut self, key: &str, value: &str) -> Self {
        self.inner.set_param(key, value);
        self
    }

    /// Removes every occurrence of query parameter `key` — see [`Uri::remove_param`].
    pub fn remove_param(&mut self, key: &str) -> bool {
        self.inner.remove_param(key)
    }

    /// Returns this URL with query parameter `key` removed — see [`Uri::without_param`].
    pub fn without_param(mut self, key: &str) -> Self {
        self.inner.remove_param(key);
        self
    }

    /// Bulk-updates the query from `(key, value)` pairs — see [`Uri::set_params`].
    pub fn set_params(&mut self, params: &[(&str, &str)]) {
        self.inner.set_params(params);
    }

    /// Returns this URL with the bulk update applied — see [`Uri::with_params`].
    pub fn with_params(mut self, params: &[(&str, &str)]) -> Self {
        self.inner.set_params(params);
        self
    }

    /// Sorts and cleans the query parameters in place — see [`Uri::normalize_params`].
    pub fn normalize_params(&mut self) {
        self.inner.normalize_params();
    }

    /// Returns this URL with the query normalized — see [`Uri::with_normalized_params`].
    pub fn with_normalized_params(mut self) -> Self {
        self.inner.normalize_params();
        self
    }

    // ---- builder mutators ----------------------------------------------------------

    /// Returns this URL with the scheme set.
    pub fn with_scheme(mut self, scheme: &str) -> Self {
        self.inner.set_scheme(scheme);
        self
    }

    /// Returns this URL with the whole authority replaced (pass `None` to drop it).
    pub fn with_authority(mut self, authority: Option<Authority>) -> Self {
        self.inner.set_authority(authority);
        self
    }

    /// Returns this URL with the host set.
    pub fn with_host(mut self, host: &str) -> Self {
        self.inner.set_host(host);
        self
    }

    /// Returns this URL with the port set.
    pub fn with_port(mut self, port: u16) -> Self {
        self.inner.set_port(port);
        self
    }

    /// Returns this URL with the userinfo user set.
    pub fn with_user(mut self, user: &str) -> Self {
        self.inner.set_user(user);
        self
    }

    /// Returns this URL with the userinfo password set.
    pub fn with_password(mut self, password: &str) -> Self {
        self.inner.set_password(password);
        self
    }

    /// Returns this URL with the path set, re-normalized to POSIX slashes.
    pub fn with_path(mut self, path: &str) -> Self {
        self.inner.set_path(path);
        self
    }

    /// Returns this URL with the query set.
    pub fn with_query(mut self, query: &str) -> Self {
        self.inner.set_query(query);
        self
    }

    /// Returns this URL with the fragment set.
    pub fn with_fragment(mut self, fragment: &str) -> Self {
        self.inner.set_fragment(fragment);
        self
    }

    // ---- in-place setters ----------------------------------------------------------

    /// Sets the scheme.
    pub fn set_scheme(&mut self, scheme: &str) {
        self.inner.set_scheme(scheme);
    }

    /// Replaces the whole authority (pass `None` to drop it).
    pub fn set_authority(&mut self, authority: Option<Authority>) {
        self.inner.set_authority(authority);
    }

    /// Sets the host.
    pub fn set_host(&mut self, host: &str) {
        self.inner.set_host(host);
    }

    /// Sets the port.
    pub fn set_port(&mut self, port: u16) {
        self.inner.set_port(port);
    }

    /// Sets the userinfo user.
    pub fn set_user(&mut self, user: &str) {
        self.inner.set_user(user);
    }

    /// Sets the userinfo password.
    pub fn set_password(&mut self, password: &str) {
        self.inner.set_password(password);
    }

    /// Sets the path, re-normalizing back-slashes to forward slashes.
    pub fn set_path(&mut self, path: &str) {
        self.inner.set_path(path);
    }

    /// Sets the query.
    pub fn set_query(&mut self, query: &str) {
        self.inner.set_query(query);
    }

    /// Sets the fragment.
    pub fn set_fragment(&mut self, fragment: &str) {
        self.inner.set_fragment(fragment);
    }

    // ---- byte codec + interchange --------------------------------------------------

    /// The canonical URL string as UTF-8 bytes.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        self.inner.serialize_bytes()
    }

    /// Decodes a URL from the UTF-8 bytes produced by [`serialize_bytes`](Url::serialize_bytes).
    ///
    /// # Errors
    /// [`UriError::NonUtf8`] if the bytes are not UTF-8, [`UriError::MissingScheme`] if the
    /// decoded URI is not absolute, or any [`Uri::parse_str`] error.
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// let url = Url::parse_str("sc://h/p").unwrap();
    /// assert_eq!(Url::deserialize_bytes(&url.serialize_bytes()).unwrap(), url);
    /// ```
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Url, UriError> {
        Url::try_from(Uri::deserialize_bytes(bytes)?)
    }

    /// Borrows the wrapped [`Uri`].
    pub fn as_uri(&self) -> &Uri {
        &self.inner
    }

    /// Unwraps into the underlying [`Uri`] (infallible — a URL is always a URI).
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// let uri = Url::parse_str("sc://h").unwrap().into_uri();
    /// assert_eq!(uri.scheme(), Some("sc"));
    /// ```
    pub fn into_uri(self) -> Uri {
        self.inner
    }

    // ---- combinators (copy / joinpath / merge) -------------------------------------

    /// An explicit copy of this URL — the cross-language name for a clone.
    pub fn copy(&self) -> Url {
        self.clone()
    }

    /// Returns this URL with `path` joined onto its path — see [`Uri::joinpath`]. The scheme
    /// is preserved, so the result is still an absolute URL.
    ///
    /// ```
    /// use yggdryl_core::uri::Url;
    ///
    /// let base = Url::parse_str("https://api.example.com/v1").unwrap();
    /// assert_eq!(base.joinpath("users/42").to_string(), "https://api.example.com/v1/users/42");
    /// ```
    pub fn joinpath(&self, path: &str) -> Url {
        Url {
            inner: self.inner.joinpath(path),
        }
    }

    /// Returns a copy of this URL overlaid by `other` — see [`Uri::merge_with`]. Both carry a
    /// scheme, so the result is always an absolute URL.
    pub fn merge_with(&self, other: &Url) -> Url {
        Url {
            inner: self.inner.merge_with(&other.inner),
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// A URL is a URI — infallible.
impl From<Url> for Uri {
    fn from(url: Url) -> Uri {
        url.inner
    }
}

/// A URI is a URL only when it carries a scheme.
impl TryFrom<Uri> for Url {
    type Error = UriError;

    fn try_from(uri: Uri) -> Result<Url, UriError> {
        if uri.scheme().is_none() {
            return Err(UriError::MissingScheme {
                input: uri.to_string(),
            });
        }
        Ok(Url { inner: uri })
    }
}

// Value semantics by canonical string.
impl PartialEq for Url {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Url {}

impl core::hash::Hash for Url {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl crate::io::Serializable for Url {
    type Error = UriError;

    fn serialize_bytes(&self) -> Vec<u8> {
        Url::serialize_bytes(self)
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, UriError> {
        Url::deserialize_bytes(bytes)
    }
}
