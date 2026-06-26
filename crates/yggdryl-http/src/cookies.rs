//! The dependency-free [`HttpCookies`] jar and its [`Cookie`] — RFC 6265 storage.

use crate::headers::HttpHeaders;
use crate::time::now_secs;
use yggdryl_core::Url;

/// One stored cookie, as parsed from a `Set-Cookie` response header.
///
/// The attributes follow RFC 6265 §5.2: a missing `Path` defaults to `"/"`, a
/// missing `Domain` makes the cookie **host-only** (it matches only the exact
/// request host), and `Max-Age` / `Expires` set the absolute expiry (Unix-epoch
/// seconds). `Secure` cookies are withheld over plain `http`.
///
/// ```
/// use yggdryl_http::Cookie;
/// use yggdryl_core::Url;
///
/// let url = Url::from_str("https://example.com/app").unwrap();
/// let cookie = Cookie::from_set_cookie("sid=abc; Path=/; Secure", &url).unwrap();
/// assert_eq!(cookie.name(), "sid");
/// assert_eq!(cookie.value(), "abc");
/// assert!(cookie.secure());
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    /// Absolute expiry as Unix-epoch seconds; `None` is a session cookie.
    expires: Option<f64>,
    secure: bool,
    http_only: bool,
    /// `true` when no `Domain` attribute was given — the cookie matches only the
    /// exact request host (RFC 6265 §5.3, step 6).
    host_only: bool,
}

impl Cookie {
    /// The cookie name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The cookie value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The domain the cookie is scoped to (the request host for a host-only one).
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The path the cookie is scoped to (default `"/"`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The absolute expiry as Unix-epoch seconds, if any (`None` = session cookie).
    pub fn expires(&self) -> Option<f64> {
        self.expires
    }

    /// Whether the cookie is `Secure` (sent only over `https`).
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Whether the cookie is `HttpOnly`.
    pub fn http_only(&self) -> bool {
        self.http_only
    }

    /// Whether the cookie is host-only (no `Domain` attribute — exact host match).
    pub fn host_only(&self) -> bool {
        self.host_only
    }

    /// Builds a cookie explicitly from a `name`/`value`, scoped to `url`'s host
    /// (host-only) and path `"/"`. Returns `None` if `name` is empty.
    pub fn new(name: impl Into<String>, value: impl Into<String>, url: &Url) -> Option<Cookie> {
        let name = name.into();
        if name.is_empty() {
            return None;
        }
        Some(Cookie {
            name,
            value: value.into(),
            domain: url.host().to_ascii_lowercase(),
            path: "/".to_string(),
            expires: None,
            secure: false,
            http_only: false,
            host_only: true,
        })
    }

    /// Parses one `Set-Cookie` header value (`name=value; Domain=…; Path=…;
    /// Secure; HttpOnly; Max-Age=…; Expires=…`) against the `request_url` that
    /// delivered it. Returns `None` when the `name=value` pair is missing or the
    /// name is empty (RFC 6265 §5.2, step 1). A bare `Domain` defaults the cookie
    /// to host-only; a missing `Path` defaults to `"/"`.
    pub fn from_set_cookie(value: &str, request_url: &Url) -> Option<Cookie> {
        let mut parts = value.split(';');
        let pair = parts.next()?.trim();
        let (name, value) = pair.split_once('=')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }

        let mut domain: Option<String> = None;
        let mut path: Option<String> = None;
        let mut secure = false;
        let mut http_only = false;
        let mut max_age: Option<f64> = None;
        let mut expires: Option<f64> = None;

        for attribute in parts {
            let attribute = attribute.trim();
            let (key, attr_value) = match attribute.split_once('=') {
                Some((key, attr_value)) => (key.trim(), attr_value.trim()),
                None => (attribute, ""),
            };
            match key.to_ascii_lowercase().as_str() {
                "domain" => {
                    // A leading dot is ignored (RFC 6265 §5.2.3); the domain is
                    // lower-cased and an empty value is dropped.
                    let host = attr_value.trim_start_matches('.').to_ascii_lowercase();
                    if !host.is_empty() {
                        domain = Some(host);
                    }
                }
                "path" if attr_value.starts_with('/') => path = Some(attr_value.to_string()),
                "secure" => secure = true,
                "httponly" => http_only = true,
                "max-age" => max_age = attr_value.parse::<f64>().ok(),
                "expires" => expires = crate::time::parse_http_date(attr_value),
                _ => {} // unknown attributes are ignored (RFC 6265 §5.2)
            }
        }

        // Max-Age wins over Expires (RFC 6265 §5.2.2); a non-positive Max-Age
        // expires the cookie immediately.
        let expires = match max_age {
            Some(seconds) => Some(now_secs() + seconds),
            None => expires,
        };

        // RFC 6265 §5.3 steps 5-6: a server may only set a `Domain` it
        // domain-matches. Reject a cross-domain `Domain` (cookie injection — e.g.
        // a.evil.com sending `Domain=example.com`) and a single-label / public-
        // suffix `Domain` (e.g. `Domain=com`, which would scope to every `.com`).
        let request_host = request_url.host().to_ascii_lowercase();
        if let Some(domain) = &domain {
            // A single-label (public-suffix-like) `Domain` such as `com` is rejected
            // — except when it is exactly the request host (e.g. `localhost` or an
            // intranet single-label host), which is always a legitimate same-host
            // Domain. Otherwise the host must domain-match the `Domain`.
            let dotless_non_host = !domain.contains('.') && *domain != request_host;
            if dotless_non_host || !domain_match(&request_host, domain, false) {
                log_event!(
                    warn,
                    "rejecting Set-Cookie {name:?}: Domain={domain:?} not allowed for host {request_host:?}"
                );
                return None;
            }
        }
        let host_only = domain.is_none();
        let domain = domain.unwrap_or(request_host);

        Some(Cookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain,
            path: path.unwrap_or_else(|| default_path(request_url)),
            expires,
            secure,
            http_only,
            host_only,
        })
    }

    /// Whether the cookie has passed its expiry (`Expires` / `Max-Age`); a session
    /// cookie (no expiry) never expires.
    fn is_expired(&self) -> bool {
        self.expires.is_some_and(|expiry| expiry <= now_secs())
    }

    /// Whether the cookie applies to `url`: a domain-match (RFC 6265 §5.1.3) **and**
    /// a path-match (§5.1.4), and — for a `Secure` cookie — an `https` scheme.
    fn matches(&self, url: &Url) -> bool {
        if self.secure && !url.scheme().eq_ignore_ascii_case("https") {
            return false;
        }
        domain_match(
            &url.host().to_ascii_lowercase(),
            &self.domain,
            self.host_only,
        ) && path_match(url.path(), &self.path)
    }
}

/// An RFC 6265 cookie jar: it ingests `Set-Cookie` from a response and produces
/// the `Cookie` header for a matching request, dependency-free.
///
/// Stored on the [`HttpSession`](crate::HttpSession) behind a mutex, it is
/// consulted before every dispatch (adding the `Cookie` header) and fed every
/// response's `Set-Cookie`. Cookies are keyed by `(name, domain, path)`, so a
/// later `Set-Cookie` for the same key replaces the earlier one, and expired
/// cookies are dropped on access.
///
/// ```
/// use yggdryl_http::{HttpCookies, HttpHeaders};
/// use yggdryl_core::Url;
///
/// let url = Url::from_str("https://example.com/").unwrap();
/// let mut jar = HttpCookies::new();
/// let mut headers = HttpHeaders::new();
/// headers.insert("Set-Cookie", "sid=abc; Path=/");
/// jar.set_from_response(&url, &headers);
/// assert_eq!(jar.header_for(&url).as_deref(), Some("sid=abc"));
/// ```
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HttpCookies {
    cookies: Vec<Cookie>,
}

impl HttpCookies {
    /// Creates an empty jar.
    pub fn new() -> HttpCookies {
        HttpCookies {
            cookies: Vec::new(),
        }
    }

    /// Ingests every `Set-Cookie` header in `headers`, scoping each to
    /// `request_url`. A cookie replaces any earlier one with the same
    /// `(name, domain, path)`; an unparseable header value is skipped.
    pub fn set_from_response(&mut self, request_url: &Url, headers: &HttpHeaders) {
        for value in headers.get_all("set-cookie") {
            if let Some(cookie) = Cookie::from_set_cookie(value, request_url) {
                self.set(cookie);
            } else {
                log_event!(warn, "skipping unparseable Set-Cookie: {value:?}");
            }
        }
    }

    /// Stores `cookie`, replacing any earlier one with the same name, domain and
    /// path (RFC 6265 §5.3, step 11).
    pub fn set(&mut self, cookie: Cookie) {
        self.cookies.retain(|stored| {
            stored.name != cookie.name
                || stored.domain != cookie.domain
                || stored.path != cookie.path
        });
        self.cookies.push(cookie);
    }

    /// Returns the jar with `cookie` stored (a builder-style [`set`](HttpCookies::set)).
    pub fn with_cookie(mut self, cookie: Cookie) -> HttpCookies {
        self.set(cookie);
        self
    }

    /// The `Cookie:` header value for `url` — every non-expired cookie whose
    /// domain-match, path-match and `Secure` rule apply, joined `"k=v; k2=v2"` —
    /// or `None` when nothing matches. Expired cookies are dropped as a side effect.
    ///
    /// Cookies with a longer `Path` are listed first (RFC 6265 §5.4); a stable
    /// sort keeps insertion order for equal-length paths.
    pub fn header_for(&mut self, url: &Url) -> Option<String> {
        self.cookies.retain(|cookie| !cookie.is_expired());
        let mut matched: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|cookie| cookie.matches(url))
            .collect();
        if matched.is_empty() {
            return None;
        }
        matched.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
        // Write straight into one reused String (no Vec<String> + join).
        let mut header = String::new();
        for cookie in matched {
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(&cookie.name);
            header.push('=');
            header.push_str(&cookie.value);
        }
        Some(header)
    }

    /// The first stored cookie named `name` (case-sensitive, per RFC 6265), if any.
    pub fn get(&self, name: &str) -> Option<&Cookie> {
        self.cookies.iter().find(|cookie| cookie.name == name)
    }

    /// Iterates the stored cookies in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Cookie> {
        self.cookies.iter()
    }

    /// Removes every stored cookie.
    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    /// The number of stored cookies.
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Whether the jar holds no cookies.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }
}

/// The default cookie path for `url` (RFC 6265 §5.1.4): the request path up to,
/// but not including, the last `/`; the empty / root / relative path is `"/"`.
fn default_path(url: &Url) -> String {
    let path = url.path();
    if !path.starts_with('/') {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

/// Domain-match per RFC 6265 §5.1.3: a host-only cookie matches only an identical
/// host; otherwise the host equals the cookie domain or is a `.`-suffixed subdomain
/// of it (and the host is not an IP, which never sub-domain-matches).
fn domain_match(host: &str, domain: &str, host_only: bool) -> bool {
    if host == domain {
        return true;
    }
    if host_only {
        return false;
    }
    host.ends_with(domain)
        && host.len() > domain.len()
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
        && !is_ip_literal(host)
}

/// Path-match per RFC 6265 §5.1.4: the request path equals the cookie path, or the
/// cookie path is a prefix ending in `/`, or the next request-path byte is `/`.
fn path_match(request_path: &str, cookie_path: &str) -> bool {
    let request_path = if request_path.is_empty() {
        "/"
    } else {
        request_path
    };
    if request_path == cookie_path {
        return true;
    }
    if !request_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/') || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
}

/// Whether `host` is an IPv4/IPv6 literal (which never participates in a
/// subdomain domain-match).
fn is_ip_literal(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
}
