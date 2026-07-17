//! [`UriParts`] — the RFC 3986 top-level components of a [`Uri`](crate::uri::Uri) bundled into
//! one owned value, the destructuring counterpart of the individual component accessors.

use core::fmt;

/// The five RFC 3986 top-level parts of a URI, owned so the value can be held and moved freely
/// (no lifetime, per the crate's public-type rule). Built by
/// [`Uri::parts`](crate::uri::Uri::parts) / [`Url::parts`](crate::uri::Url::parts): the
/// convenience of reading every component in one call and destructuring them together.
///
/// ```
/// use yggdryl_core::uri::Uri;
///
/// let parts = Uri::parse_str("https://h:8080/a/b?q=1#f").unwrap().parts();
/// assert_eq!(parts.scheme.as_deref(), Some("https"));
/// assert_eq!(parts.authority.as_deref(), Some("h:8080"));
/// assert_eq!(parts.path, "/a/b");
/// assert_eq!(parts.query.as_deref(), Some("q=1"));
/// assert_eq!(parts.fragment.as_deref(), Some("f"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UriParts {
    /// The scheme, if any (`"https"`).
    pub scheme: Option<String>,
    /// The authority, if any, rendered as `[user[:password]@]host[:port]` (`"h:8080"`).
    pub authority: Option<String>,
    /// The path — always present (may be empty).
    pub path: String,
    /// The query string, if any (without the leading `?`).
    pub query: Option<String>,
    /// The fragment, if any (without the leading `#`).
    pub fragment: Option<String>,
}

impl fmt::Display for UriParts {
    /// Re-renders the URI from its parts (`scheme://authority/path?query#fragment`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(scheme) = &self.scheme {
            write!(f, "{scheme}:")?;
        }
        if let Some(authority) = &self.authority {
            write!(f, "//{authority}")?;
        }
        f.write_str(&self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}
