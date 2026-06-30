//! [`Cookie`] and [`CookieJar`] — simple HTTP cookie storage.
//!
//! `CookieJar` stores cookies in memory and applies them to outgoing requests.
//! It is thread-safe (behind an `Arc<Mutex<_>>` inside `HttpSession`) so a
//! session can be shared across threads without additional synchronisation.

use std::collections::HashMap;

/// A single HTTP cookie (name + value pair).
///
/// Domain, path, expiry, and secure/SameSite flags are not tracked: the jar
/// applies all stored cookies to every matching scheme+host request for
/// simplicity.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
}

impl Cookie {
    /// Constructs a cookie.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Cookie {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Parses a `Set-Cookie` header value into a `Cookie` (name and value only).
    ///
    /// Returns `None` if the header does not contain a `=`.
    pub fn from_set_cookie(header: &str) -> Option<Self> {
        // `Set-Cookie: name=value; Path=/; HttpOnly`
        let pair = header.split(';').next()?;
        let (name, value) = pair.split_once('=')?;
        Some(Cookie {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
        })
    }

    /// The `name=value` string used in `Cookie:` request headers.
    pub fn to_header_value(&self) -> String {
        format!("{}={}", self.name, self.value)
    }
}

/// A thread-safe in-memory cookie store.
///
/// Cookies are keyed by name; the last `Set-Cookie` for a given name wins.
/// `apply` builds the `Cookie:` header value for a request.
#[derive(Clone, Debug, Default)]
pub struct CookieJar {
    cookies: HashMap<String, String>,
}

impl CookieJar {
    /// An empty jar.
    pub fn new() -> Self {
        CookieJar::default()
    }

    /// Inserts or overwrites a cookie.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.cookies.insert(name.into(), value.into());
    }

    /// Removes a cookie by name.
    pub fn remove(&mut self, name: &str) {
        self.cookies.remove(name);
    }

    /// Returns the value for `name`, if present.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    /// Returns all cookies as a `Cookie: name=value; …` header value string,
    /// or `None` if the jar is empty.
    pub fn as_header_value(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        // Sort by name for deterministic output (easier to test, log, debug).
        let mut pairs: Vec<_> = self.cookies.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        Some(
            pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// Absorbs a `Set-Cookie` header value, updating or inserting the cookie.
    /// Returns `false` if the header could not be parsed.
    pub fn absorb_set_cookie(&mut self, header: &str) -> bool {
        if let Some(cookie) = Cookie::from_set_cookie(header) {
            self.cookies.insert(cookie.name, cookie.value);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jar_round_trips() {
        let mut jar = CookieJar::new();
        jar.set("session", "abc");
        jar.set("lang", "en");
        let header = jar.as_header_value().unwrap();
        // alphabetical
        assert!(header.contains("lang=en"));
        assert!(header.contains("session=abc"));
        jar.remove("session");
        assert_eq!(jar.as_header_value().unwrap(), "lang=en");
    }

    #[test]
    fn absorb_set_cookie_parses_attributes() {
        let mut jar = CookieJar::new();
        assert!(jar.absorb_set_cookie("token=xyz; Path=/; Secure; HttpOnly"));
        assert_eq!(jar.get("token"), Some("xyz"));
    }
}
