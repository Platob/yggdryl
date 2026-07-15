//! [`Authority`] — the `[user[:password]@]host[:port]` component of a URI.

use core::fmt;
use core::fmt::Write as _;

use super::HashWrite;

/// The authority component of a URI: `[ userinfo "@" ] host [ ":" port ]`, where
/// `userinfo` is `user [ ":" password ]`.
///
/// DESIGN: the userinfo is stored as **flat** `user` / `password` fields rather than a
/// nested `UserInfo` type — one fewer public type to replicate across the FFI bindings,
/// and the four accessors read the same in every language. The
/// `host` is stored verbatim, including the brackets of an IPv6 literal (`"[::1]"`).
///
/// ```
/// use yggdryl_core::io::Authority;
///
/// let a = Authority::new(Some("user"), Some("pass"), "example.com", Some(8080));
/// assert_eq!(a.user(), Some("user"));
/// assert_eq!(a.host(), "example.com");
/// assert_eq!(a.port(), Some(8080));
/// assert_eq!(a.to_string(), "user:pass@example.com:8080");
/// ```
#[derive(Debug, Clone, Default)]
pub struct Authority {
    user: Option<String>,
    password: Option<String>,
    host: String,
    port: Option<u16>,
}

impl Authority {
    /// Builds an authority from its parts.
    ///
    /// ```
    /// use yggdryl_core::io::Authority;
    ///
    /// let a = Authority::new(None, None, "localhost", Some(80));
    /// assert_eq!(a.to_string(), "localhost:80");
    /// ```
    pub fn new(user: Option<&str>, password: Option<&str>, host: &str, port: Option<u16>) -> Self {
        Self {
            user: user.map(str::to_string),
            password: password.map(str::to_string),
            host: host.to_string(),
            port,
        }
    }

    /// Builds a bare `host`-only authority (no userinfo, no port).
    ///
    /// ```
    /// use yggdryl_core::io::Authority;
    ///
    /// assert_eq!(Authority::from_host("example.com").host(), "example.com");
    /// ```
    pub fn from_host(host: &str) -> Self {
        Self {
            host: host.to_string(),
            ..Self::default()
        }
    }

    /// The userinfo user, if any.
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The userinfo password, if any.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// The host (an empty string for an empty authority such as `file:///path`). An IPv6
    /// literal keeps its brackets (`"[::1]"`); use [`host_unbracketed`](Authority::host_unbracketed)
    /// for the bare address.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Whether the host is a bracketed IPv6 (or IP-future) literal, e.g. `"[::1]"` — the one
    /// host form RFC 3986 wraps in `[` `]`. An unterminated `"[::1"` (no closing bracket) is
    /// **not** counted, matching how the parser keeps it verbatim as a plain host.
    ///
    /// ```
    /// use yggdryl_core::io::Authority;
    ///
    /// assert!(Authority::from_host("[::1]").host_is_ipv6());
    /// assert!(!Authority::from_host("example.com").host_is_ipv6());
    /// ```
    pub fn host_is_ipv6(&self) -> bool {
        self.host.len() >= 2 && self.host.starts_with('[') && self.host.ends_with(']')
    }

    /// The host with the IPv6 literal's brackets stripped — `"[::1]"` → `"::1"` — so it can
    /// be handed straight to a socket / resolver API; a reg-name or IPv4 host is returned
    /// verbatim. Zero-copy: it borrows the stored host.
    ///
    /// ```
    /// use yggdryl_core::io::Authority;
    ///
    /// assert_eq!(Authority::from_host("[2001:db8::1]").host_unbracketed(), "2001:db8::1");
    /// assert_eq!(Authority::from_host("example.com").host_unbracketed(), "example.com");
    /// ```
    pub fn host_unbracketed(&self) -> &str {
        if self.host_is_ipv6() {
            &self.host[1..self.host.len() - 1]
        } else {
            &self.host
        }
    }

    /// The port, if any.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Sets the userinfo user.
    pub fn set_user(&mut self, user: Option<&str>) {
        self.user = user.map(str::to_string);
    }

    /// Sets the userinfo password.
    pub fn set_password(&mut self, password: Option<&str>) {
        self.password = password.map(str::to_string);
    }

    /// Sets the host.
    pub fn set_host(&mut self, host: &str) {
        self.host = host.to_string();
    }

    /// Sets the port.
    pub fn set_port(&mut self, port: Option<u16>) {
        self.port = port;
    }

    // ---- builder mutators + combinators --------------------------------------------

    /// An explicit copy of this authority — the cross-language name for a clone.
    pub fn copy(&self) -> Authority {
        self.clone()
    }

    /// Returns this authority with the userinfo user set (pass `None` to clear it).
    pub fn with_user(mut self, user: Option<&str>) -> Self {
        self.set_user(user);
        self
    }

    /// Returns this authority with the userinfo password set (pass `None` to clear it).
    pub fn with_password(mut self, password: Option<&str>) -> Self {
        self.set_password(password);
        self
    }

    /// Returns this authority with the host set.
    pub fn with_host(mut self, host: &str) -> Self {
        self.set_host(host);
        self
    }

    /// Returns this authority with the port set (pass `None` to clear it).
    pub fn with_port(mut self, port: Option<u16>) -> Self {
        self.set_port(port);
        self
    }

    /// Returns a copy of this authority **overlaid** by `other`: each field `other` sets (a
    /// `Some` user/password/port, or a non-empty host) wins, otherwise this authority's is
    /// kept. Handy for patching just the port or credentials of a base authority.
    ///
    /// ```
    /// use yggdryl_core::io::Authority;
    ///
    /// let base = Authority::new(Some("svc"), Some("secret"), "db", Some(5432));
    /// let patch = Authority::from_host("replica"); // only the host is set
    /// assert_eq!(base.merge_with(&patch).to_string(), "svc:secret@replica:5432");
    /// ```
    pub fn merge_with(&self, other: &Authority) -> Authority {
        Authority {
            user: other.user.clone().or_else(|| self.user.clone()),
            password: other.password.clone().or_else(|| self.password.clone()),
            host: if other.host.is_empty() {
                self.host.clone()
            } else {
                other.host.clone()
            },
            port: other.port.or(self.port),
        }
    }

    /// An upper bound on the byte length of this authority's canonical rendering, so an
    /// owning [`Uri`](super::Uri) can pre-size its buffer. The port digits are over-counted
    /// (at most 5) — it never under-allocates.
    pub(crate) fn encoded_len(&self) -> usize {
        let mut len = self.host.len();
        if self.user.is_some() || self.password.is_some() {
            len += self.user.as_deref().map_or(0, str::len);
            len += self.password.as_deref().map_or(0, |p| p.len() + 1); // ":password"
            len += 1; // "@"
        }
        if self.port.is_some() {
            len += 6; // ":" + up to five port digits
        }
        len
    }

    /// The canonical rendering built into a pre-sized buffer (one allocation), the single
    /// source of the value-semantics comparison.
    fn to_canonical(&self) -> String {
        let mut buffer = String::with_capacity(self.encoded_len());
        let _ = write!(buffer, "{self}");
        buffer
    }
}

impl fmt::Display for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Render the userinfo when EITHER a user or a password is present — a password with
        // no user (reachable via `set_password`/`with_password`) must not be silently dropped
        // by `Display`/`serialize_bytes` (it renders as `:password@`, which re-parses to the
        // same canonical form, so the credential round-trips).
        if self.user.is_some() || self.password.is_some() {
            if let Some(user) = &self.user {
                write!(f, "{user}")?;
            }
            if let Some(password) = &self.password {
                write!(f, ":{password}")?;
            }
            f.write_str("@")?;
        }
        f.write_str(&self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

// Value semantics by canonical string: two authorities are equal
// iff they render identically, and equal values hash equal.
impl PartialEq for Authority {
    fn eq(&self, other: &Self) -> bool {
        // Pre-sized canonical strings (one allocation each): a password with no user and
        // `user = Some("")` render alike, so identity is the rendering, not the components.
        self.to_canonical() == other.to_canonical()
    }
}

impl Eq for Authority {}

impl core::hash::Hash for Authority {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Stream the canonical rendering into the hasher without allocating a `String`.
        let _ = write!(HashWrite(&mut *state), "{self}");
        state.write_u8(0xff);
    }
}
