//! The HTTP request methods of RFC 9110 section 9.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// One request method of RFC 9110 section 9.
///
/// A method is read case-insensitively and written upper case; the wire
/// spelling is [`Method::as_str`]. The three properties a client decides by
/// are the section's own: a safe method reads without changing the resource
/// ([`is_safe`](Self::is_safe)), an idempotent one may be repeated
/// ([`is_idempotent`](Self::is_idempotent)), and three carry a request body
/// by convention ([`has_request_body`](Self::has_request_body)).
///
/// ```
/// use yggdryl::http::Method;
///
/// let method = Method::from_str("post").unwrap();
/// assert_eq!(method, Method::Post);
/// assert_eq!(method.as_str(), "POST");
/// assert!(!method.is_idempotent());
/// assert!(method.has_request_body());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Method {
    /// Transfer a current representation of the target resource.
    Get,
    /// The same as `GET`, but without the response content.
    Head,
    /// Perform resource-specific processing on the request content.
    Post,
    /// Replace every current representation with the request content.
    Put,
    /// Apply partial modifications to the resource (RFC 5789).
    Patch,
    /// Remove every current representation of the target resource.
    Delete,
    /// Describe the communication options for the target resource.
    Options,
    /// Perform a message loop-back test along the path to the target.
    Trace,
    /// Establish a tunnel to the server identified by the target.
    Connect,
}

impl Method {
    /// Every method in canonical order.
    pub const ALL: [Self; 9] = [
        Self::Get,
        Self::Head,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
        Self::Options,
        Self::Trace,
        Self::Connect,
    ];

    /// Parse a method name, case-insensitively.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `http method` naming the accepted
    /// vocabulary.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// The upper-case wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Trace => "TRACE",
            Self::Connect => "CONNECT",
        }
    }

    /// Whether the method is safe (RFC 9110 9.2.1): `GET`, `HEAD`, `OPTIONS`
    /// and `TRACE` read without changing the resource.
    pub const fn is_safe(self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options | Self::Trace)
    }

    /// Whether the method is idempotent (RFC 9110 9.2.2): every safe method,
    /// `PUT` and `DELETE` may be repeated with the same intended effect.
    pub const fn is_idempotent(self) -> bool {
        self.is_safe() || matches!(self, Self::Put | Self::Delete)
    }

    /// Whether the method carries a request body by convention: `POST`,
    /// `PUT` and `PATCH`.
    pub const fn has_request_body(self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch)
    }
}

impl AsRef<str> for Method {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Method {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Method {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|method| normalized.eq_ignore_ascii_case(method.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "http method",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|method| method.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl Serialize for Method {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Method {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}
