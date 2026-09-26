//! Cookies: RFC 6265 section 5 as a client keeps it.
//!
//! A server states a cookie in a `Set-Cookie` field; [`Cookie::from_set_cookie`]
//! reads that field with the section 5.2 algorithm and settles what the
//! section 5.3 storage step decides - the host the cookie belongs to, the
//! path it covers, when it lapses - so a stored cookie carries no unresolved
//! attribute. [`Cookie::matches`] is the section 5.4 question a request URL
//! asks of one cookie, and [`CookieJar`] is the store: one cookie per
//! `(name, domain, path)`, the `Cookie` header a URL receives ordered by path
//! length as the section asks, and expiry evicted where a clock is handed in.
//!
//! Every instant is UTC nanoseconds since the epoch, the crate's one clock
//! spelling, and no function reads the system clock: the caller states `now`.

use std::fmt::Write as _;

use smol_str::format_smolstr;

use super::Headers;
use crate::timezone::days_from_civil;
use crate::{Error, Result, Url};

/// Nanoseconds in one second.
const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// One cookie, as stored: the section 5.3 storage step already taken.
///
/// `domain` is the host the cookie belongs to, lower case and without a
/// leading dot: the `Domain` attribute when the server stated one, else the
/// host that set it - and `host_only` says which, because a host-only cookie
/// reaches that host alone while a domain cookie reaches its subdomains too.
/// `path` is the `Path` attribute, else the default path of the URL that set
/// it. `expires` is the instant it lapses, `Max-Age` winning over `Expires`,
/// and `None` for a session cookie, which lasts as long as the jar does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cookie {
    /// The cookie name, a token.
    pub name: String,
    /// The cookie value, as the server spelled it.
    pub value: String,
    /// The host or cover domain, lower case, no leading dot.
    pub domain: String,
    /// Whether only `domain` itself receives the cookie, never a subdomain.
    pub host_only: bool,
    /// The path prefix the cookie covers, opening with `/`.
    pub path: String,
    /// When the cookie lapses, UTC nanoseconds; `None` is a session cookie.
    pub expires: Option<i64>,
    /// Whether the cookie travels over `https` alone.
    pub secure: bool,
    /// Whether the server marked the cookie `HttpOnly` (inert for a client).
    pub http_only: bool,
}

impl Cookie {
    /// A domain cookie of `name` and `value` covering `domain` and every
    /// path, without expiry: what a caller adds to a session by hand.
    pub fn new(name: impl Into<String>, value: impl Into<String>, domain: &str) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            domain: domain.trim_start_matches('.').to_ascii_lowercase(),
            host_only: false,
            path: "/".to_owned(),
            expires: None,
            secure: false,
            http_only: false,
        }
    }

    /// Read one `Set-Cookie` field value set by `url` at `now_ns`.
    ///
    /// The section 5.2 algorithm: the first `;` splits the name-value pair
    /// from the attributes, each attribute is read by name and an unknown
    /// one is ignored, `Max-Age` (an integer count of seconds from `now_ns`,
    /// zero or negative lapsing at once) wins over `Expires` (the section
    /// 5.1.1 cookie-date, an unreadable one ignored), a `Domain` is stripped
    /// of its leading dot and lowered, and a `Path` that does not open with
    /// `/` is the URL's default path. Then the section 5.3 step: a `Domain`
    /// that does not cover the URL's host is refused, and no `Domain` makes
    /// the cookie host-only.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] naming the byte position when the pair has no `=`,
    /// the name is empty or not a token, the value carries a control byte or
    /// a `;`, or the `Domain` does not cover the URL's host.
    ///
    /// ```
    /// use yggdryl::Url;
    /// use yggdryl::http::Cookie;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("https://api.example.com/v1/orders")?;
    /// let cookie = Cookie::from_set_cookie(
    ///     "session=abc123; Domain=.example.com; Path=/v1; Max-Age=60; Secure",
    ///     &url,
    ///     1_000_000_000_000,
    /// )?;
    /// assert_eq!(cookie.header_pair(), "session=abc123");
    /// assert_eq!(cookie.domain, "example.com");
    /// assert_eq!(cookie.path, "/v1");
    /// assert_eq!(cookie.expires, Some(1_060_000_000_000), "sixty seconds past now");
    /// assert!(cookie.secure && !cookie.host_only);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_set_cookie(header: &str, url: &Url, now_ns: i64) -> Result<Self> {
        let (pair, attributes) = match header.find(';') {
            Some(index) => (&header[..index], &header[index + 1..]),
            None => (header, ""),
        };
        let Some(equals) = pair.find('=') else {
            return Err(refusal(0, "expected `name=value` opening a Set-Cookie"));
        };
        let name = pair[..equals].trim();
        if name.is_empty() {
            return Err(refusal(0, "expected a cookie name before `=`"));
        }
        if let Some(offset) = name.bytes().position(|byte| !is_token_byte(byte)) {
            return Err(refusal(
                pair.find(name).unwrap_or(0) + offset,
                "a cookie name is a token: no separators, spaces or controls",
            ));
        }
        let value = pair[equals + 1..].trim();
        if let Some(offset) = value.bytes().position(|byte| byte < 0x20 || byte == 0x7f) {
            return Err(refusal(
                equals + 1 + pair[equals + 1..].find(value).unwrap_or(0) + offset,
                "a cookie value holds no control byte",
            ));
        }
        let value = value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .unwrap_or(value);

        let host = url.hostname().unwrap_or_default().to_ascii_lowercase();
        let mut cookie = Self {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: host.clone(),
            host_only: true,
            path: default_path(url)?,
            expires: None,
            secure: false,
            http_only: false,
        };
        let mut max_age: Option<i64> = None;
        let mut expires: Option<i64> = None;
        let mut cover: Option<String> = None;
        let mut cursor = pair.len() + 1;
        for attribute in attributes.split(';') {
            let start = cursor;
            cursor += attribute.len() + 1;
            let (attribute_name, attribute_value) = match attribute.find('=') {
                Some(index) => (attribute[..index].trim(), attribute[index + 1..].trim()),
                None => (attribute.trim(), ""),
            };
            if attribute_name.eq_ignore_ascii_case("expires") {
                if let Some(instant) = parse_cookie_date(attribute_value) {
                    expires = Some(instant);
                }
            } else if attribute_name.eq_ignore_ascii_case("max-age") {
                if let Some(seconds) = parse_max_age(attribute_value) {
                    max_age = Some(seconds);
                }
            } else if attribute_name.eq_ignore_ascii_case("domain") {
                let stated = attribute_value.trim_start_matches('.');
                if !stated.is_empty() {
                    let stated = stated.to_ascii_lowercase();
                    if !domain_matches(&host, &stated) {
                        return Err(refusal(
                            start,
                            "the Domain attribute does not cover the host that set the cookie",
                        ));
                    }
                    cover = Some(stated);
                }
            } else if attribute_name.eq_ignore_ascii_case("path") {
                if attribute_value.starts_with('/') {
                    cookie.path = attribute_value.to_owned();
                }
            } else if attribute_name.eq_ignore_ascii_case("secure") {
                cookie.secure = true;
            } else if attribute_name.eq_ignore_ascii_case("httponly") {
                cookie.http_only = true;
            }
        }
        if let Some(cover) = cover {
            cookie.domain = cover;
            cookie.host_only = false;
        }
        cookie.expires = match max_age {
            Some(seconds) if seconds <= 0 => Some(i64::MIN),
            Some(seconds) => Some(
                seconds
                    .saturating_mul(NANOS_PER_SECOND)
                    .saturating_add(now_ns),
            ),
            None => expires,
        };
        Ok(cookie)
    }

    /// Whether the cookie has lapsed at `now_ns`; a session cookie never has.
    #[must_use]
    pub fn is_expired(&self, now_ns: i64) -> bool {
        self.expires.is_some_and(|expires| expires <= now_ns)
    }

    /// Whether a request to `url` at `now_ns` carries this cookie: the
    /// section 5.4 test - not lapsed, the host equal to the domain (host-only)
    /// or under it, the request path equal to or below the cookie path, and
    /// `https` where the cookie is secure.
    #[must_use]
    pub fn matches(&self, url: &Url, now_ns: i64) -> bool {
        if self.is_expired(now_ns) {
            return false;
        }
        if self.secure && url.scheme().as_str() != "https" {
            return false;
        }
        let host = url.hostname().unwrap_or_default().to_ascii_lowercase();
        let host_matches = if self.host_only {
            host == self.domain
        } else {
            domain_matches(&host, &self.domain)
        };
        if !host_matches {
            return false;
        }
        let request_path = url.path().as_str();
        let request_path = if request_path.starts_with('/') {
            request_path
        } else {
            "/"
        };
        path_matches(request_path, &self.path)
    }

    /// The `name=value` pair the `Cookie` header carries.
    #[must_use]
    pub fn header_pair(&self) -> String {
        format!("{}={}", self.name, self.value)
    }
}

/// The cookies a session holds, one per `(name, domain, path)`.
#[derive(Clone, Debug, Default)]
pub struct CookieJar {
    cookies: Vec<Stored>,
    /// The creation counter the `Cookie` header orders equal-length paths by.
    next_order: u64,
}

/// One stored cookie and when it arrived relative to the others.
#[derive(Clone, Debug)]
struct Stored {
    cookie: Cookie,
    order: u64,
}

impl CookieJar {
    /// An empty jar.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many cookies are held, lapsed ones included until evicted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// The held cookies, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Cookie> {
        self.cookies.iter().map(|stored| &stored.cookie)
    }

    /// Store `cookie`, replacing the one of the same name, domain and path
    /// and keeping that one's place in the creation order, as section 5.3
    /// step 11 asks.
    pub fn set(&mut self, cookie: Cookie) {
        let same = |stored: &Stored| {
            stored.cookie.name == cookie.name
                && stored.cookie.domain == cookie.domain
                && stored.cookie.path == cookie.path
        };
        if let Some(stored) = self.cookies.iter_mut().find(|stored| same(stored)) {
            stored.cookie = cookie;
            return;
        }
        let order = self.next_order;
        self.next_order += 1;
        self.cookies.push(Stored { cookie, order });
    }

    /// Store every `Set-Cookie` field of `headers` set by `url` at `now_ns`,
    /// evicting what has lapsed, and answer how many were stored.
    ///
    /// A field that does not parse is ignored, as section 5.2 asks of a user
    /// agent: one malformed cookie must not lose the response it rode on.
    /// [`Cookie::from_set_cookie`] is where the refusal is heard.
    pub fn set_from_headers(&mut self, url: &Url, headers: &Headers, now_ns: i64) -> usize {
        let mut stored = 0;
        for field in headers.set_cookies() {
            if let Ok(cookie) = Cookie::from_set_cookie(field, url, now_ns) {
                self.set(cookie);
                stored += 1;
            }
        }
        self.remove_expired(now_ns);
        stored
    }

    /// The `Cookie` header value a request to `url` at `now_ns` carries:
    /// every matching cookie, longer paths first and earlier cookies first
    /// among equal paths, joined with `; `. `None` when nothing matches.
    #[must_use]
    pub fn header_for(&self, url: &Url, now_ns: i64) -> Option<String> {
        let mut matching: Vec<&Stored> = self
            .cookies
            .iter()
            .filter(|stored| stored.cookie.matches(url, now_ns))
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by(|left, right| {
            right
                .cookie
                .path
                .len()
                .cmp(&left.cookie.path.len())
                .then(left.order.cmp(&right.order))
        });
        let mut header = String::new();
        for (index, stored) in matching.iter().enumerate() {
            if index > 0 {
                header.push_str("; ");
            }
            let _ = write!(header, "{}={}", stored.cookie.name, stored.cookie.value);
        }
        Some(header)
    }

    /// Evict every cookie lapsed at `now_ns` and answer how many went.
    pub fn remove_expired(&mut self, now_ns: i64) -> usize {
        let before = self.cookies.len();
        self.cookies
            .retain(|stored| !stored.cookie.is_expired(now_ns));
        before - self.cookies.len()
    }

    /// Drop every cookie.
    pub fn clear(&mut self) {
        self.cookies.clear();
    }
}

/// The section 5.1.3 domain-match: `host` is `domain` or ends with
/// `.domain`, and is a host name rather than an address.
fn domain_matches(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }
    if is_address(host) {
        return false;
    }
    host.len() > domain.len()
        && host.ends_with(domain)
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}

/// Whether `host` is an IPv4 or IPv6 literal, which no cover domain reaches.
fn is_address(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
        || host
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .is_some_and(|inner| inner.parse::<std::net::Ipv6Addr>().is_ok())
}

/// The section 5.1.4 path-match.
fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    if !request_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/') || request_path.as_bytes()[cookie_path.len()] == b'/'
}

/// The section 5.1.4 default path of `url`: its path up to, not including,
/// the rightmost `/`, and `/` for a path that is empty, relative or has no
/// second `/`.
fn default_path(url: &Url) -> Result<String> {
    let path = url.path_text(false)?;
    if !path.starts_with('/') {
        return Ok("/".to_owned());
    }
    match path.rfind('/') {
        Some(0) | None => Ok("/".to_owned()),
        Some(index) => Ok(path[..index].to_owned()),
    }
}

/// A `Max-Age` attribute value: an optional `-` and digits, else ignored.
fn parse_max_age(value: &str) -> Option<i64> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(value.parse::<i64>().unwrap_or(if value.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    }))
}

/// The section 5.1.1 cookie-date algorithm, answering UTC nanoseconds.
///
/// Tokens are split on the delimiters the section lists, and each token is
/// tried as the time, the day, the month and the year in that order, the
/// first match of each kind kept; a two-digit year is folded into 1970-2069.
/// `None` when a component is missing or the date does not exist.
fn parse_cookie_date(text: &str) -> Option<i64> {
    let mut time: Option<(u32, u32, u32)> = None;
    let mut day: Option<u32> = None;
    let mut month: Option<u32> = None;
    let mut year: Option<i32> = None;
    for token in text
        .split(is_date_delimiter)
        .filter(|token| !token.is_empty())
    {
        if time.is_none() {
            if let Some(found) = parse_time(token) {
                time = Some(found);
                continue;
            }
        }
        if day.is_none() {
            if let Some(found) = leading_digits(token, 1, 2) {
                day = Some(found);
                continue;
            }
        }
        if month.is_none() {
            if let Some(found) = parse_month(token) {
                month = Some(found);
                continue;
            }
        }
        if year.is_none() {
            if let Some(found) = leading_digits(token, 2, 4) {
                year = Some(i32::try_from(found).ok()?);
            }
        }
    }
    let (hour, minute, second) = time?;
    let (day, month, mut year) = (day?, month?, year?);
    if (70..=99).contains(&year) {
        year += 1900;
    } else if (0..=69).contains(&year) {
        year += 2000;
    }
    if year < 1601 || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))?;
    seconds.checked_mul(NANOS_PER_SECOND)
}

/// The cookie-date delimiters: HTAB, and the printable non-alphanumerics
/// other than `:`.
fn is_date_delimiter(character: char) -> bool {
    matches!(character, '\t' | ' '..='/' | ';'..='@' | '['..='`' | '{'..='~')
}

/// `hh:mm:ss` with one or two digits each, followed by a non-digit or the end.
fn parse_time(token: &str) -> Option<(u32, u32, u32)> {
    let mut parts = token.splitn(3, ':');
    let hour = digits_then_boundary(parts.next()?, true)?;
    let minute = digits_then_boundary(parts.next()?, true)?;
    let second = digits_then_boundary(parts.next()?, false)?;
    Some((hour, minute, second))
}

/// One or two digits making up the whole token, or, for the last time part,
/// followed by a non-digit.
fn digits_then_boundary(part: &str, whole: bool) -> Option<u32> {
    let count = part.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=2).contains(&count) || (whole && count != part.len()) {
        return None;
    }
    part[..count].parse().ok()
}

/// The token's leading digits when they number between `min` and `max` and
/// are followed by a non-digit or the end.
fn leading_digits(token: &str, min: usize, max: usize) -> Option<u32> {
    let count = token.bytes().take_while(u8::is_ascii_digit).count();
    if count < min || count > max {
        return None;
    }
    token[..count].parse().ok()
}

/// The month a token opens with, case-insensitively.
fn parse_month(token: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let head = token.get(..3)?;
    MONTHS
        .iter()
        .position(|month| head.eq_ignore_ascii_case(month))
        .and_then(|index| u32::try_from(index + 1).ok())
}

/// How many days `month` of `year` has.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

/// RFC 9110 token bytes, which a cookie name is made of.
const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// A `Set-Cookie` refusal at `position`.
fn refusal(position: usize, reason: &str) -> Error {
    Error::Parse {
        target: "http header",
        position,
        reason: format_smolstr!("Set-Cookie: {reason}"),
    }
}
