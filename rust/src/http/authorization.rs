//! Who a request says it is: the `Authorization` header, or another header
//! carrying a credential.
//!
//! The credential itself is a [`Secret`](crate::auth::Secret), so a session
//! or a request holding one derives `Debug` and prints `<redacted>` where
//! the token would be; [`Authorization::header_value`] is the one place the
//! text is spelled out, for the wire.

use base64::Engine as _;
use smol_str::SmolStr;

use crate::Url;
use crate::auth::Secret;

/// The header name the `Basic` and `Bearer` schemes travel under.
const AUTHORIZATION: &str = "Authorization";

/// One credential and the scheme it is spelled in.
///
/// ```
/// use yggdryl::http::Authorization;
///
/// let basic = Authorization::basic("aladdin", "open sesame");
/// assert_eq!(basic.header_name(), "Authorization");
/// assert_eq!(basic.header_value(), "Basic YWxhZGRpbjpvcGVuIHNlc2FtZQ==");
/// assert_eq!(format!("{basic:?}"), "Basic { username: \"aladdin\", password: <redacted> }");
///
/// let key = Authorization::header("X-Api-Key", "k-123");
/// assert_eq!(key.header_name(), "X-Api-Key");
/// assert_eq!(key.header_value(), "k-123");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Authorization {
    /// RFC 7617: `Authorization: Basic base64(username:password)`.
    Basic {
        /// The user, spelled as given; a `:` inside it cannot round-trip.
        username: String,
        /// The password, never rendered.
        password: Secret,
    },
    /// RFC 6750: `Authorization: Bearer <token>`.
    Bearer(Secret),
    /// A credential under a header of the API's own naming, sent verbatim:
    /// `X-Api-Key`, `Private-Token`, or `Authorization` in a scheme this
    /// enum does not spell.
    Header {
        /// The header the credential travels under.
        name: SmolStr,
        /// The whole header value, never rendered.
        value: Secret,
    },
}

impl Authorization {
    /// A `Basic` credential.
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Basic {
            username: username.into(),
            password: Secret::new(password.into()),
        }
    }

    /// A `Bearer` token.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer(Secret::new(token.into()))
    }

    /// A credential sent verbatim under `name`.
    pub fn header(name: impl Into<SmolStr>, value: impl Into<String>) -> Self {
        Self::Header {
            name: name.into(),
            value: Secret::new(value.into()),
        }
    }

    /// The user information of `url` as a `Basic` credential: the user with
    /// its password, an absent password being empty. `None` when the URL
    /// names no user.
    #[must_use]
    pub fn from_url(url: &Url) -> Option<Self> {
        let user = url.user()?;
        let user = crate::uri::percent_decode(user, "url").ok()?;
        let password = match url.password() {
            Some(password) => crate::uri::percent_decode(password, "url").ok()?,
            None => std::borrow::Cow::Borrowed(""),
        };
        Some(Self::basic(user, password))
    }

    /// The header the credential travels under.
    #[must_use]
    pub fn header_name(&self) -> &str {
        match self {
            Self::Basic { .. } | Self::Bearer(_) => AUTHORIZATION,
            Self::Header { name, .. } => name,
        }
    }

    /// The header value, credential spelled out: the one door to the secret.
    #[must_use]
    pub fn header_value(&self) -> String {
        match self {
            Self::Basic { username, password } => {
                let pair = format!("{username}:{}", password.expose());
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(pair.as_bytes())
                )
            }
            Self::Bearer(token) => format!("Bearer {}", token.expose()),
            Self::Header { value, .. } => value.expose().to_owned(),
        }
    }
}
