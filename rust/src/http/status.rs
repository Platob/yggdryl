//! The HTTP response status codes of RFC 9110 section 15.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// One response status code, `100..=599`.
///
/// The code is the fact; the reason phrase is the IANA registry's text for
/// it ([`reason`](Self::reason)), empty for a code the registry does not
/// list, and never read back from the wire into the value. The class
/// predicates are the hundreds digit; [`is_retryable`](Self::is_retryable)
/// names the seven answers a client may repeat an idempotent request after.
/// Serde carries the integer alone.
///
/// ```
/// use yggdryl::http::Status;
///
/// let status = Status::new(404).unwrap();
/// assert_eq!(status, Status::NOT_FOUND);
/// assert_eq!(status.reason(), "Not Found");
/// assert_eq!(status.to_string(), "404 Not Found");
/// assert!(status.is_client_error() && !status.is_retryable());
/// assert_eq!(serde_json::to_string(&status).unwrap(), "404");
/// assert!(Status::new(600).is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Status(u16);

impl Status {
    /// `100 Continue`.
    pub const CONTINUE: Self = Self(100);
    /// `200 OK`.
    pub const OK: Self = Self(200);
    /// `201 Created`.
    pub const CREATED: Self = Self(201);
    /// `202 Accepted`.
    pub const ACCEPTED: Self = Self(202);
    /// `204 No Content`.
    pub const NO_CONTENT: Self = Self(204);
    /// `206 Partial Content`.
    pub const PARTIAL_CONTENT: Self = Self(206);
    /// `301 Moved Permanently`.
    pub const MOVED_PERMANENTLY: Self = Self(301);
    /// `302 Found`.
    pub const FOUND: Self = Self(302);
    /// `303 See Other`.
    pub const SEE_OTHER: Self = Self(303);
    /// `304 Not Modified`.
    pub const NOT_MODIFIED: Self = Self(304);
    /// `307 Temporary Redirect`.
    pub const TEMPORARY_REDIRECT: Self = Self(307);
    /// `308 Permanent Redirect`.
    pub const PERMANENT_REDIRECT: Self = Self(308);
    /// `400 Bad Request`.
    pub const BAD_REQUEST: Self = Self(400);
    /// `401 Unauthorized`.
    pub const UNAUTHORIZED: Self = Self(401);
    /// `403 Forbidden`.
    pub const FORBIDDEN: Self = Self(403);
    /// `404 Not Found`.
    pub const NOT_FOUND: Self = Self(404);
    /// `405 Method Not Allowed`.
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    /// `408 Request Timeout`.
    pub const REQUEST_TIMEOUT: Self = Self(408);
    /// `409 Conflict`.
    pub const CONFLICT: Self = Self(409);
    /// `412 Precondition Failed`.
    pub const PRECONDITION_FAILED: Self = Self(412);
    /// `416 Range Not Satisfiable`.
    pub const RANGE_NOT_SATISFIABLE: Self = Self(416);
    /// `425 Too Early`.
    pub const TOO_EARLY: Self = Self(425);
    /// `429 Too Many Requests`.
    pub const TOO_MANY_REQUESTS: Self = Self(429);
    /// `500 Internal Server Error`.
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    /// `501 Not Implemented`.
    pub const NOT_IMPLEMENTED: Self = Self(501);
    /// `502 Bad Gateway`.
    pub const BAD_GATEWAY: Self = Self(502);
    /// `503 Service Unavailable`.
    pub const SERVICE_UNAVAILABLE: Self = Self(503);
    /// `504 Gateway Timeout`.
    pub const GATEWAY_TIMEOUT: Self = Self(504);

    /// Every code the IANA registry lists, with its reason phrase.
    pub const REGISTERED: [(u16, &'static str); 61] = [
        (100, "Continue"),
        (101, "Switching Protocols"),
        (102, "Processing"),
        (103, "Early Hints"),
        (200, "OK"),
        (201, "Created"),
        (202, "Accepted"),
        (203, "Non-Authoritative Information"),
        (204, "No Content"),
        (205, "Reset Content"),
        (206, "Partial Content"),
        (207, "Multi-Status"),
        (208, "Already Reported"),
        (226, "IM Used"),
        (300, "Multiple Choices"),
        (301, "Moved Permanently"),
        (302, "Found"),
        (303, "See Other"),
        (304, "Not Modified"),
        (305, "Use Proxy"),
        (307, "Temporary Redirect"),
        (308, "Permanent Redirect"),
        (400, "Bad Request"),
        (401, "Unauthorized"),
        (402, "Payment Required"),
        (403, "Forbidden"),
        (404, "Not Found"),
        (405, "Method Not Allowed"),
        (406, "Not Acceptable"),
        (407, "Proxy Authentication Required"),
        (408, "Request Timeout"),
        (409, "Conflict"),
        (410, "Gone"),
        (411, "Length Required"),
        (412, "Precondition Failed"),
        (413, "Content Too Large"),
        (414, "URI Too Long"),
        (415, "Unsupported Media Type"),
        (416, "Range Not Satisfiable"),
        (417, "Expectation Failed"),
        (421, "Misdirected Request"),
        (422, "Unprocessable Content"),
        (423, "Locked"),
        (424, "Failed Dependency"),
        (425, "Too Early"),
        (426, "Upgrade Required"),
        (428, "Precondition Required"),
        (429, "Too Many Requests"),
        (431, "Request Header Fields Too Large"),
        (451, "Unavailable For Legal Reasons"),
        (500, "Internal Server Error"),
        (501, "Not Implemented"),
        (502, "Bad Gateway"),
        (503, "Service Unavailable"),
        (504, "Gateway Timeout"),
        (505, "HTTP Version Not Supported"),
        (506, "Variant Also Negotiates"),
        (507, "Insufficient Storage"),
        (508, "Loop Detected"),
        (510, "Not Extended"),
        (511, "Network Authentication Required"),
    ];

    /// A status from its code.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `http status` for a code outside
    /// `100..=599`.
    pub fn new(code: u16) -> Result<Self> {
        if (100..=599).contains(&code) {
            Ok(Self(code))
        } else {
            Err(Error::Parse {
                target: "http status",
                position: 0,
                reason: format_smolstr!("expected a status code in 100..=599, got {code}"),
            })
        }
    }

    /// Parse `200` or `200 OK`: the decimal code, then an optional reason
    /// phrase, which is ignored.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `http status` when the text does
    /// not open with a code in `100..=599`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// The numeric code.
    pub const fn code(self) -> u16 {
        self.0
    }

    /// The IANA reason phrase, or the empty string for an unregistered code.
    pub fn reason(self) -> &'static str {
        Self::REGISTERED
            .iter()
            .find(|(code, _)| *code == self.0)
            .map_or("", |(_, reason)| reason)
    }

    /// `1xx`: the request was received and processing continues.
    pub const fn is_informational(self) -> bool {
        self.0 / 100 == 1
    }

    /// `2xx`: the request was received, understood and accepted.
    pub const fn is_success(self) -> bool {
        self.0 / 100 == 2
    }

    /// `3xx`: further action is needed to complete the request.
    pub const fn is_redirect(self) -> bool {
        self.0 / 100 == 3
    }

    /// `4xx`: the request seems to be in error.
    pub const fn is_client_error(self) -> bool {
        self.0 / 100 == 4
    }

    /// `5xx`: the server failed to fulfil an apparently valid request.
    pub const fn is_server_error(self) -> bool {
        self.0 / 100 == 5
    }

    /// Whether the same request may be sent again: `408`, `425`, `429`,
    /// `500`, `502`, `503` and `504`.
    pub const fn is_retryable(self) -> bool {
        matches!(self.0, 408 | 425 | 429 | 500 | 502 | 503 | 504)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = self.reason();
        if reason.is_empty() {
            write!(formatter, "{}", self.0)
        } else {
            write!(formatter, "{} {reason}", self.0)
        }
    }
}

impl FromStr for Status {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let text = value.trim();
        let (digits, rest) = text.split_at(text.len().min(3));
        let is_code = digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit());
        let has_phrase_or_end = rest.is_empty() || rest.starts_with(' ');
        if !(is_code && has_phrase_or_end) {
            return Err(Error::Parse {
                target: "http status",
                position: 0,
                reason: format_smolstr!(
                    "expected a three-digit status code, optionally followed by a reason phrase, got {value:?}"
                ),
            });
        }
        let code = digits.parse::<u16>().map_err(|_| Error::Parse {
            target: "http status",
            position: 0,
            reason: format_smolstr!("expected a three-digit status code, got {digits:?}"),
        })?;
        Self::new(code)
    }
}

impl TryFrom<u16> for Status {
    type Error = Error;

    fn try_from(code: u16) -> Result<Self> {
        Self::new(code)
    }
}

impl From<Status> for u16 {
    fn from(status: Status) -> Self {
        status.0
    }
}

impl Serialize for Status {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for Status {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let code = u16::deserialize(deserializer)?;
        Self::new(code).map_err(serde::de::Error::custom)
    }
}
