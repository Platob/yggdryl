//! # yggdryl
//!
//! The pure-Rust core of the **yggdryl** project: small, dependency-free
//! [`Uri`] and [`Url`] value types.
//!
//! - [`Uri`] is the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
//!   shape: `scheme:[//authority]path[?query][#fragment]`.
//! - [`Url`] is the common subset that always has an authority, decomposed into
//!   `username`, `password`, `host` and `port`.
//!
//! The Python and Node extensions in the wider project wrap these types so the
//! behaviour is identical across every language binding.

use std::fmt;

/// Error returned when [`Uri::parse`] cannot interpret its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// The input was empty.
    Empty,
    /// No `scheme:` prefix was present.
    MissingScheme,
    /// The scheme contained characters outside `ALPHA *( ALPHA / DIGIT / +-. )`.
    InvalidScheme,
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UriError::Empty => write!(f, "uri is empty"),
            UriError::MissingScheme => write!(f, "uri has no scheme"),
            UriError::InvalidScheme => write!(f, "uri scheme is invalid"),
        }
    }
}

impl std::error::Error for UriError {}

/// Error returned when [`Url::parse`] cannot interpret its input.
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
/// use yggdryl::Uri;
///
/// let uri = Uri::parse("https://example.com/docs?page=2#intro").unwrap();
/// assert_eq!(uri.scheme(), "https");
/// assert_eq!(uri.authority(), Some("example.com"));
/// assert_eq!(uri.path(), "/docs");
/// assert_eq!(uri.query(), Some("page=2"));
/// assert_eq!(uri.fragment(), Some("intro"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri {
    scheme: String,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Uri {
    /// Parses a string into a [`Uri`].
    pub fn parse(input: &str) -> Result<Uri, UriError> {
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
        if !is_valid_scheme(scheme) {
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

        Ok(Uri {
            scheme: scheme.to_string(),
            authority,
            path,
            query,
            fragment,
        })
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

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;
        if let Some(authority) = &self.authority {
            write!(f, "//{authority}")?;
        }
        write!(f, "{}", self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Uri {
    type Err = UriError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uri::parse(s)
    }
}

/// A URL: a [`Uri`] that always has an authority, with the authority broken out
/// into `username`, `password`, `host` and `port`.
///
/// ```
/// use yggdryl::Url;
///
/// let url = Url::parse("https://user:pw@example.com:8443/api?v=1#top").unwrap();
/// assert_eq!(url.scheme(), "https");
/// assert_eq!(url.username(), Some("user"));
/// assert_eq!(url.password(), Some("pw"));
/// assert_eq!(url.host(), "example.com");
/// assert_eq!(url.port(), Some(8443));
/// assert_eq!(url.path(), "/api");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    scheme: String,
    username: Option<String>,
    password: Option<String>,
    host: String,
    port: Option<u16>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Url {
    /// Parses a string into a [`Url`]. Requires a scheme, an authority and a
    /// non-empty host.
    pub fn parse(input: &str) -> Result<Url, UrlError> {
        let uri = Uri::parse(input)?;
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
        })
    }

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

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.authority())?;
        write!(f, "{}", self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Url {
    type Err = UrlError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Url::parse(s)
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

/// Error returned when [`Version::parse`] cannot interpret its input.
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
/// use yggdryl::Version;
///
/// let v = Version::parse("1.4.2").unwrap();
/// assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
/// assert_eq!(Version::parse("2").unwrap(), Version::new(2, 0, 0));
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

    /// Parses a `major[.minor[.patch]]` string. Omitted components default to `0`.
    pub fn parse(input: &str) -> Result<Version, VersionError> {
        if input.is_empty() {
            return Err(VersionError::Empty);
        }
        let mut parts = [0u64; 3];
        for (index, part) in input.split('.').enumerate() {
            if index == 3 {
                return Err(VersionError::TooManyComponents);
            }
            parts[index] = part
                .parse::<u64>()
                .map_err(|_| VersionError::InvalidNumber(part.to_string()))?;
        }
        Ok(Version {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
        })
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
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl std::str::FromStr for Version {
    type Err = VersionError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Version::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_full() {
        let uri = Uri::parse("https://example.com/docs?page=2#intro").unwrap();
        assert_eq!(uri.scheme(), "https");
        assert_eq!(uri.authority(), Some("example.com"));
        assert_eq!(uri.path(), "/docs");
        assert_eq!(uri.query(), Some("page=2"));
        assert_eq!(uri.fragment(), Some("intro"));
    }

    #[test]
    fn uri_without_authority() {
        let uri = Uri::parse("mailto:alice@example.com").unwrap();
        assert_eq!(uri.scheme(), "mailto");
        assert_eq!(uri.authority(), None);
        assert_eq!(uri.path(), "alice@example.com");
    }

    #[test]
    fn uri_errors() {
        assert_eq!(Uri::parse(""), Err(UriError::Empty));
        assert_eq!(Uri::parse("no-scheme/path"), Err(UriError::MissingScheme));
        assert_eq!(Uri::parse(":no-scheme"), Err(UriError::MissingScheme));
        assert_eq!(Uri::parse("1http://x"), Err(UriError::InvalidScheme));
    }

    #[test]
    fn uri_round_trips() {
        for input in [
            "https://example.com/docs?page=2#intro",
            "mailto:alice@example.com",
            "file:///etc/hosts",
            "urn:isbn:0451450523",
        ] {
            assert_eq!(Uri::parse(input).unwrap().to_string(), input);
        }
    }

    #[test]
    fn url_full() {
        let url = Url::parse("https://user:pw@example.com:8443/api?v=1#top").unwrap();
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
        let url = Url::parse("http://example.com").unwrap();
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), None);
        assert_eq!(url.username(), None);
        assert_eq!(url.path(), "");
    }

    #[test]
    fn url_username_only() {
        let url = Url::parse("ftp://anon@files.example.com/pub").unwrap();
        assert_eq!(url.username(), Some("anon"));
        assert_eq!(url.password(), None);
        assert_eq!(url.host(), "files.example.com");
    }

    #[test]
    fn url_ipv6() {
        let url = Url::parse("http://[::1]:8080/status").unwrap();
        assert_eq!(url.host(), "::1");
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.to_string(), "http://[::1]:8080/status");
    }

    #[test]
    fn url_errors() {
        assert_eq!(
            Url::parse("mailto:alice@example.com"),
            Err(UrlError::MissingAuthority)
        );
        assert_eq!(Url::parse("http://user@:80"), Err(UrlError::MissingHost));
        assert!(matches!(
            Url::parse("http://example.com:notaport"),
            Err(UrlError::InvalidPort(_))
        ));
        assert_eq!(
            Url::parse("notauri"),
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
            assert_eq!(Url::parse(input).unwrap().to_string(), input);
        }
    }

    #[test]
    fn url_authority_is_reconstructed() {
        let url = Url::parse("https://user:pw@example.com:8443/api").unwrap();
        assert_eq!(url.authority(), "user:pw@example.com:8443");
    }

    #[test]
    fn version_parse_full() {
        let v = Version::parse("1.4.2").unwrap();
        assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
    }

    #[test]
    fn version_parse_partial_defaults_to_zero() {
        assert_eq!(Version::parse("2").unwrap(), Version::new(2, 0, 0));
        assert_eq!(Version::parse("2.5").unwrap(), Version::new(2, 5, 0));
    }

    #[test]
    fn version_errors() {
        assert_eq!(Version::parse(""), Err(VersionError::Empty));
        assert_eq!(
            Version::parse("1.2.3.4"),
            Err(VersionError::TooManyComponents)
        );
        assert_eq!(
            Version::parse("1.x.0"),
            Err(VersionError::InvalidNumber("x".to_string()))
        );
        assert_eq!(
            Version::parse("1..0"),
            Err(VersionError::InvalidNumber(String::new()))
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
        assert_eq!(Version::parse("1.4.2").unwrap().to_string(), "1.4.2");
        assert_eq!(Version::parse("3").unwrap().to_string(), "3.0.0");
    }
}
