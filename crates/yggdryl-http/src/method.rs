//! [`Method`] — HTTP request method.

use std::fmt;

/// An HTTP request method.
///
/// The eight standard methods are recognised case-insensitively by
/// [`from_str`](Method::from_str); unknown strings are stored as
/// [`Custom`](Method::Custom).
///
/// ```
/// use yggdryl_http::Method;
///
/// assert_eq!(Method::Get.to_str(), "GET");
/// assert_eq!(Method::from_str("post"), Method::Post);
/// assert_eq!(Method::Custom("PURGE".into()).to_str(), "PURGE");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Method {
    /// `GET`
    #[default]
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `DELETE`
    Delete,
    /// `PATCH`
    Patch,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
    /// `TRACE`
    Trace,
    /// `CONNECT`
    Connect,
    /// A non-standard or extension method.
    Custom(String),
}

impl Method {
    /// The canonical uppercase string for this method.
    pub fn to_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Patch => "PATCH",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
            Method::Trace => "TRACE",
            Method::Connect => "CONNECT",
            Method::Custom(s) => s.as_str(),
        }
    }

    /// Parses a method string (case-insensitive for the nine standard methods).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            "PATCH" => Method::Patch,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            "TRACE" => Method::Trace,
            "CONNECT" => Method::Connect,
            _ => Method::Custom(s.to_string()),
        }
    }

    /// Returns the serialized bytes of the method string.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().as_bytes().to_vec()
    }

    /// Parses from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(s) => Method::from_str(s),
            Err(_) => Method::Custom(String::from_utf8_lossy(bytes).into_owned()),
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_str())
    }
}
