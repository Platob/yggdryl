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
/// use yggdryl_core::io::Url;
///
/// let url = Url::parse("https://example.com/a/b.txt").unwrap();
/// assert_eq!(url.scheme(), "https");
/// assert_eq!(url.host(), Some("example.com"));
/// assert_eq!(url.name(), Some("b.txt"));
///
/// // A scheme-less input is not an absolute URL.
/// assert!(Url::parse("/relative/path").is_err());
/// ```
#[derive(Debug, Clone)]
pub struct Url {
    inner: Uri,
}

impl Url {
    /// Parses `s` into an absolute URL.
    ///
    /// # Errors
    /// [`UriError::MissingScheme`] if `s` has no scheme, plus any [`Uri::parse`] error.
    ///
    /// ```
    /// use yggdryl_core::io::Url;
    ///
    /// assert_eq!(Url::parse("sc://h/p").unwrap().scheme(), "sc");
    /// ```
    pub fn parse(s: &str) -> Result<Url, UriError> {
        Url::try_from(Uri::parse(s)?)
    }

    /// The scheme (always present).
    pub fn scheme(&self) -> &str {
        // Guaranteed `Some` by the constructor invariant.
        self.inner.scheme().unwrap_or_default()
    }

    /// The authority, if any.
    pub fn authority(&self) -> Option<&Authority> {
        self.inner.authority()
    }

    /// The userinfo user, if any.
    pub fn user(&self) -> Option<&str> {
        self.inner.user()
    }

    /// The userinfo password, if any.
    pub fn password(&self) -> Option<&str> {
        self.inner.password()
    }

    /// The host, if this URL has an authority.
    pub fn host(&self) -> Option<&str> {
        self.inner.host()
    }

    /// The port, if any.
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
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

    /// The first value of query parameter `key`, or `None` — see [`Uri::query_param`].
    pub fn query_param(&self, key: &str) -> Option<&str> {
        self.inner.query_param(key)
    }

    /// Every value of query parameter `key`, in order — see [`Uri::query_param_all`].
    pub fn query_param_all(&self, key: &str) -> Vec<&str> {
        self.inner.query_param_all(key)
    }

    /// All query parameters as ordered `(key, value)` pairs — see [`Uri::query_params`].
    pub fn query_params(&self) -> Vec<(&str, &str)> {
        self.inner.query_params()
    }

    /// The first value of query parameter `key`, percent-decoded — see
    /// [`Uri::query_param_decoded`].
    pub fn query_param_decoded(&self, key: &str) -> Option<Cow<'_, str>> {
        self.inner.query_param_decoded(key)
    }

    /// Every value of query parameter `key`, percent-decoded — see
    /// [`Uri::query_param_all_decoded`].
    pub fn query_param_all_decoded(&self, key: &str) -> Vec<Cow<'_, str>> {
        self.inner.query_param_all_decoded(key)
    }

    /// All query parameters as `(key, value)` pairs, percent-decoded — see
    /// [`Uri::query_params_decoded`].
    pub fn query_params_decoded(&self) -> Vec<(Cow<'_, str>, Cow<'_, str>)> {
        self.inner.query_params_decoded()
    }

    /// Whether query parameter `key` is present — see [`Uri::has_query_param`].
    pub fn has_query_param(&self, key: &str) -> bool {
        self.inner.has_query_param(key)
    }

    /// Sets query parameter `key` to `value` (map semantics) — see [`Uri::set_query_param`].
    pub fn set_query_param(&mut self, key: &str, value: &str) {
        self.inner.set_query_param(key, value);
    }

    /// Returns this URL with query parameter `key` set — see [`Uri::with_query_param`].
    pub fn with_query_param(mut self, key: &str, value: &str) -> Self {
        self.inner.set_query_param(key, value);
        self
    }

    /// Removes every occurrence of query parameter `key` — see [`Uri::remove_query_param`].
    pub fn remove_query_param(&mut self, key: &str) -> bool {
        self.inner.remove_query_param(key)
    }

    /// Returns this URL with query parameter `key` removed — see [`Uri::without_query_param`].
    pub fn without_query_param(mut self, key: &str) -> Self {
        self.inner.remove_query_param(key);
        self
    }

    /// Bulk-updates the query from `(key, value)` pairs — see [`Uri::set_query_params`].
    pub fn set_query_params(&mut self, params: &[(&str, &str)]) {
        self.inner.set_query_params(params);
    }

    /// Returns this URL with the bulk update applied — see [`Uri::with_query_params`].
    pub fn with_query_params(mut self, params: &[(&str, &str)]) -> Self {
        self.inner.set_query_params(params);
        self
    }

    /// Sorts and cleans the query parameters in place — see [`Uri::normalize_query`].
    pub fn normalize_query(&mut self) {
        self.inner.normalize_query();
    }

    /// Returns this URL with the query normalized — see [`Uri::with_normalized_query`].
    pub fn with_normalized_query(mut self) -> Self {
        self.inner.normalize_query();
        self
    }

    // ---- builder mutators ----------------------------------------------------------

    /// Returns this URL with the scheme set.
    pub fn with_scheme(mut self, scheme: &str) -> Self {
        self.inner.set_scheme(scheme);
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
    /// decoded URI is not absolute, or any [`Uri::parse`] error.
    ///
    /// ```
    /// use yggdryl_core::io::Url;
    ///
    /// let url = Url::parse("sc://h/p").unwrap();
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
    /// use yggdryl_core::io::Url;
    ///
    /// let uri = Url::parse("sc://h").unwrap().into_uri();
    /// assert_eq!(uri.scheme(), Some("sc"));
    /// ```
    pub fn into_uri(self) -> Uri {
        self.inner
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
