//! The case-insensitive [`HttpHeaders`] type — the one place all header logic lives.

use std::time::Duration;

/// A case-insensitive, order-preserving multimap of HTTP headers.
///
/// Names are matched case-insensitively (so `Content-Type` and `content-type`
/// are the same header) while their original spelling and insertion order are
/// preserved, and a name may carry several values (as `Set-Cookie` does).
/// Every request, response and stream in the crate carries one, and all header
/// parsing — `Retry-After`, the response size from `Content-Range` /
/// `Content-Length`, the merge of session defaults under per-request overrides —
/// lives here.
///
/// ```
/// use yggdryl_http::HttpHeaders;
///
/// let mut headers = HttpHeaders::new();
/// headers.insert("Content-Type", "text/plain");
/// assert_eq!(headers.get("content-type"), Some("text/plain"));
/// headers.set("content-type", "application/json"); // replaces
/// assert_eq!(headers.get("Content-Type"), Some("application/json"));
/// assert!(headers.contains("CONTENT-TYPE"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpHeaders {
    entries: Vec<(String, String)>,
}

impl HttpHeaders {
    /// Creates an empty header set.
    pub fn new() -> HttpHeaders {
        HttpHeaders {
            entries: Vec::new(),
        }
    }

    /// Builds a header set from `(name, value)` pairs, preserving their order.
    pub fn from_mapping<I>(pairs: I) -> HttpHeaders
    where
        I: IntoIterator<Item = (String, String)>,
    {
        HttpHeaders {
            entries: pairs.into_iter().collect(),
        }
    }

    /// The first value for `name` (case-insensitive), if any.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Every value for `name` (case-insensitive), in insertion order.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// Whether any value is present for `name` (case-insensitive).
    pub fn contains(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(name))
    }

    /// Appends a `(name, value)` pair, keeping any existing values for `name`.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.entries.push((name.into(), value.into()));
    }

    /// Replaces every value for `name` (case-insensitive) with the single
    /// `value`, keeping the first slot's position (appending if `name` is new).
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        let mut placed = false;
        self.entries.retain_mut(|(key, slot)| {
            if !key.eq_ignore_ascii_case(&name) {
                return true;
            }
            if placed {
                return false; // drop the surplus duplicates
            }
            placed = true;
            *slot = value.clone();
            true
        });
        if !placed {
            self.entries.push((name, value));
        }
    }

    /// Removes every value for `name` (case-insensitive).
    pub fn remove(&mut self, name: &str) {
        self.entries
            .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
    }

    /// Iterates the `(name, value)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// The number of `(name, value)` pairs (counting duplicates).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no headers.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merges `self` (the session defaults) under `overrides` (a request's
    /// headers): a default is kept only when `overrides` carries no value of the
    /// same name, then the overrides are appended. The single place a per-request
    /// header wins over a session default, case-insensitively.
    pub fn merge(&self, overrides: &HttpHeaders) -> HttpHeaders {
        let mut merged = HttpHeaders {
            entries: Vec::with_capacity(self.entries.len() + overrides.entries.len()),
        };
        for (name, value) in &self.entries {
            if !overrides.contains(name) {
                merged.entries.push((name.clone(), value.clone()));
            }
        }
        merged.entries.extend(overrides.entries.iter().cloned());
        merged
    }

    /// The first value for `name` parsed as a `u64`, if present and valid.
    pub fn get_u64(&self, name: &str) -> Option<u64> {
        self.get(name).and_then(|value| value.trim().parse().ok())
    }

    /// The `Retry-After` delay (given in seconds), if present and valid.
    pub fn retry_after(&self) -> Option<Duration> {
        self.get_u64("retry-after").map(Duration::from_secs)
    }

    /// The total resource size: the total in a `Content-Range`
    /// (`bytes a-b/total`) when present, else `Content-Length`.
    pub fn content_size(&self) -> Option<u64> {
        if let Some(range) = self.get("content-range") {
            if let Some((_, total)) = range.rsplit_once('/') {
                if let Ok(total) = total.trim().parse() {
                    return Some(total);
                }
            }
        }
        self.get_u64("content-length")
    }
}

impl From<&ureq::http::HeaderMap> for HttpHeaders {
    fn from(headers: &ureq::http::HeaderMap) -> HttpHeaders {
        HttpHeaders::from_mapping(headers.iter().map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        }))
    }
}
