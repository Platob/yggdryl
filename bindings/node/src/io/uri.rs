//! The `yggdryl.uri` namespace — RFC 3986 URIs, absolute URLs, and their authority.
//!
//! Mirrors `yggdryl_core::io::uri`'s root URI types: [`Uri`] (a generic URI, doubling as a
//! POSIX-normalized filesystem path), [`Url`] (a URI guaranteed to carry a scheme), and
//! [`Authority`] (the `[user[:password]@]host[:port]` component). Each is a thin value
//! wrapper with the usual value-type surface — a byte codec (`serializeBytes` /
//! `deserializeBytes`), content equality (`equals`), a Java-style `hashCode`, and
//! `toString` — so it round-trips and works as a map key exactly like the core value.
//!
//! Parsing follows the core's decisions, which the caller sees through unchanged: paths are
//! standardized to POSIX forward slashes (a Windows drive path `C:\…` keeps its drive letter
//! in the path rather than being read as a one-letter scheme), IPv6 hosts stay bracketed
//! (`"[::1]"`), and a `Url` requires only a scheme (its authority stays optional, so
//! `mailto:a@b.com` is a valid URL with no host). Every parser/port failure surfaces as a
//! thrown `Error` carrying the core's guided text.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi::{Env, JsFunction, JsObject, JsUnknown};
use napi_derive::napi;

use yggdryl_core::io::uri as core;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// The IANA-registered default port for a well-known scheme (case-insensitive), or `null` if
/// the scheme has no registered default. Mirrors [`yggdryl_core::io::uri::default_port`].
#[napi(js_name = "defaultPort", namespace = "uri")]
pub fn default_port(scheme: String) -> Option<u16> {
    core::default_port(&scheme)
}

/// Per-field overrides for `Authority.copy`. Each present field replaces that component of the
/// copy; an absent (undefined) field keeps the current one.
#[napi(object)]
pub struct AuthorityCopyOptions {
    pub user: Option<String>,
    pub password: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// Per-field overrides for `Uri.copy` / `Url.copy`. Each present field replaces that component
/// of the copy (creating an authority where one is needed); an absent (undefined) field keeps
/// the current one.
#[napi(object)]
pub struct UriCopyOptions {
    pub scheme: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

/// The `[user[:password]@]host[:port]` authority component of a URI.
///
/// The userinfo is stored as flat `user` / `password` fields (no nested type), and the
/// `host` keeps the brackets of an IPv6 literal (`"[::1]"`).
#[napi(namespace = "uri")]
pub struct Authority {
    pub(crate) inner: core::Authority,
}

#[napi(namespace = "uri")]
impl Authority {
    /// Builds an authority from a required `host` and optional `user` / `password` / `port`.
    #[napi(constructor)]
    pub fn new(
        host: String,
        user: Option<String>,
        password: Option<String>,
        port: Option<u16>,
    ) -> Self {
        Self {
            inner: core::Authority::new(user.as_deref(), password.as_deref(), &host, port),
        }
    }

    /// Builds a bare `host`-only authority (no userinfo, no port).
    #[napi(factory)]
    pub fn from_host(host: String) -> Self {
        Self {
            inner: core::Authority::from_host(&host),
        }
    }

    /// The userinfo user, if any.
    #[napi(getter)]
    pub fn user(&self) -> Option<String> {
        self.inner.user().map(str::to_string)
    }

    /// The userinfo password, if any.
    #[napi(getter)]
    pub fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    /// The host (an empty string for an empty authority such as `file:///path`; an IPv6
    /// literal keeps its brackets).
    #[napi(getter)]
    pub fn host(&self) -> String {
        self.inner.host().to_string()
    }

    /// Whether the host is a bracketed IPv6 literal (`"[::1]"`).
    #[napi(getter)]
    pub fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped (`"[::1]"` → `"::1"`); a reg-name/IPv4 host
    /// passes through verbatim.
    #[napi(getter)]
    pub fn host_unbracketed(&self) -> String {
        self.inner.host_unbracketed().to_string()
    }

    /// The port, if any.
    #[napi(getter)]
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// Sets the userinfo user (pass `null` to clear it).
    #[napi]
    pub fn set_user(&mut self, user: Option<String>) {
        self.inner.set_user(user.as_deref());
    }

    /// Sets the userinfo password (pass `null` to clear it).
    #[napi]
    pub fn set_password(&mut self, password: Option<String>) {
        self.inner.set_password(password.as_deref());
    }

    /// Sets the host.
    #[napi]
    pub fn set_host(&mut self, host: String) {
        self.inner.set_host(&host);
    }

    /// Sets the port (pass `null` to clear it).
    #[napi]
    pub fn set_port(&mut self, port: Option<u16>) {
        self.inner.set_port(port);
    }

    /// An explicit copy of this authority, optionally overriding fields via
    /// `copy({ user, password, host, port })` — each present option replaces that field, an
    /// absent one is kept. With no argument it is a plain clone.
    #[napi]
    pub fn copy(&self, options: Option<AuthorityCopyOptions>) -> Self {
        let mut inner = self.inner.clone();
        if let Some(options) = options {
            if let Some(user) = options.user {
                inner = inner.with_user(Some(&user));
            }
            if let Some(password) = options.password {
                inner = inner.with_password(Some(&password));
            }
            if let Some(host) = options.host {
                inner = inner.with_host(&host);
            }
            if let Some(port) = options.port {
                inner = inner.with_port(Some(port));
            }
        }
        Self { inner }
    }

    /// Returns a copy with the userinfo user set (pass `null` to clear it).
    #[napi]
    pub fn with_user(&self, user: Option<String>) -> Self {
        Self {
            inner: self.inner.clone().with_user(user.as_deref()),
        }
    }

    /// Returns a copy with the userinfo password set (pass `null` to clear it).
    #[napi]
    pub fn with_password(&self, password: Option<String>) -> Self {
        Self {
            inner: self.inner.clone().with_password(password.as_deref()),
        }
    }

    /// Returns a copy with the host set.
    #[napi]
    pub fn with_host(&self, host: String) -> Self {
        Self {
            inner: self.inner.clone().with_host(&host),
        }
    }

    /// Returns a copy with the port set (pass `null` to clear it).
    #[napi]
    pub fn with_port(&self, port: Option<u16>) -> Self {
        Self {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Returns a copy overlaid by `other`: each field `other` sets wins, else this one's is kept.
    #[napi]
    pub fn merge_with(&self, other: &Authority) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    /// The canonical authority string as UTF-8 bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.to_string().into_bytes().into()
    }

    /// Content equality (equal iff `serializeBytes` are equal).
    #[napi]
    pub fn equals(&self, other: &Authority) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The canonical authority string, e.g. `"user:pass@example.com:8080"`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}

/// A generic RFC 3986 URI split into its components, doubling as a filesystem-path
/// abstraction. Any component may be absent; a bare path (no scheme, no authority) is a
/// perfectly good `Uri`.
#[napi(namespace = "uri")]
pub struct Uri {
    pub(crate) inner: core::Uri,
}

#[napi(namespace = "uri")]
impl Uri {
    /// Parses `s` into its RFC 3986 components, or normalizes a bare filesystem path,
    /// throwing a guided `Error` on a malformed scheme or an out-of-range port.
    #[napi(factory)]
    pub fn parse(s: String) -> napi::Result<Self> {
        core::Uri::parse_str(&s)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Builds a scheme-less, authority-less `Uri` from a filesystem path, rewriting every
    /// back-slash to a forward slash so the stored path is POSIX slash-based.
    #[napi(factory)]
    pub fn from_path(path: String) -> Self {
        Self {
            inner: core::Uri::from_path(&path),
        }
    }

    /// The scheme, if any.
    #[napi(getter)]
    pub fn scheme(&self) -> Option<String> {
        self.inner.scheme().map(str::to_string)
    }

    /// The authority, if any.
    #[napi(getter)]
    pub fn authority(&self) -> Option<Authority> {
        self.inner
            .authority()
            .map(|a| Authority { inner: a.clone() })
    }

    /// The userinfo user, if this URI has an authority carrying one.
    #[napi(getter)]
    pub fn user(&self) -> Option<String> {
        self.inner.user().map(str::to_string)
    }

    /// The userinfo password, if this URI has an authority carrying one.
    #[napi(getter)]
    pub fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    /// The host, if this URI has an authority (an IPv6 literal keeps its brackets).
    #[napi(getter)]
    pub fn host(&self) -> Option<String> {
        self.inner.host().map(str::to_string)
    }

    /// Whether this URI's host is a bracketed IPv6 literal (`false` if it has no authority).
    #[napi(getter)]
    pub fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped, if this URI has an authority — the bare
    /// address to hand to a socket API.
    #[napi(getter)]
    pub fn host_unbracketed(&self) -> Option<String> {
        self.inner.host_unbracketed().map(str::to_string)
    }

    /// The port as written, if any (see `portOrDefault` for the effective port).
    #[napi(getter)]
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// The default port registered for this URI's scheme, or `null` if scheme-less or the
    /// scheme has no known default.
    #[napi(getter)]
    pub fn default_port(&self) -> Option<u16> {
        self.inner.default_port()
    }

    /// The effective port to connect to: the explicit `port`, else the scheme's
    /// `defaultPort`. `null` when neither is known. Derived on read — the URI is untouched.
    #[napi(getter)]
    pub fn port_or_default(&self) -> Option<u16> {
        self.inner.port_or_default()
    }

    /// The path, always POSIX slash-normalized (possibly empty).
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().to_string()
    }

    /// The query, if any (the text after `?`, without the `?`).
    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner.query().map(str::to_string)
    }

    /// The fragment, if any (the text after `#`, without the `#`).
    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner.fragment().map(str::to_string)
    }

    /// The last non-empty path segment (the filename), or `null` for an empty or
    /// directory-like path (one ending in `/`).
    #[napi(getter)]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// The filename without its last extension (a leading dot marks a hidden file whose dot
    /// is not an extension separator, so its stem is the whole name).
    #[napi(getter)]
    pub fn stem(&self) -> Option<String> {
        self.inner.stem().map(str::to_string)
    }

    /// The last extension of the filename (without the dot), or `null` for a name with no
    /// extension, a trailing dot, or a hidden dotfile.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(str::to_string)
    }

    /// Every extension of a multi-dot filename, outermost-last
    /// (`archive.tar.gz` → `["tar", "gz"]`); empty for a name with no extension.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    // ---- builder mutators (return a new `Uri`) -----------------------------------------

    /// Returns a copy of this URI with the scheme set.
    #[napi]
    pub fn with_scheme(&self, scheme: String) -> Self {
        Self {
            inner: self.inner.clone().with_scheme(&scheme),
        }
    }

    /// Returns a copy of this URI with the whole authority replaced (pass `null` to drop it).
    #[napi]
    pub fn with_authority(&self, authority: Option<&Authority>) -> Self {
        Self {
            inner: self
                .inner
                .clone()
                .with_authority(authority.map(|a| a.inner.clone())),
        }
    }

    /// Returns a copy of this URI with the host set (creating an authority if absent).
    #[napi]
    pub fn with_host(&self, host: String) -> Self {
        Self {
            inner: self.inner.clone().with_host(&host),
        }
    }

    /// Returns a copy of this URI with the port set (creating an authority if absent).
    #[napi]
    pub fn with_port(&self, port: u16) -> Self {
        Self {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Returns a copy of this URI with the userinfo user set (creating an authority if absent).
    #[napi]
    pub fn with_user(&self, user: String) -> Self {
        Self {
            inner: self.inner.clone().with_user(&user),
        }
    }

    /// Returns a copy of this URI with the userinfo password set (creating an authority if absent).
    #[napi]
    pub fn with_password(&self, password: String) -> Self {
        Self {
            inner: self.inner.clone().with_password(&password),
        }
    }

    /// Returns a copy of this URI with the path set, re-normalized to POSIX slashes.
    #[napi]
    pub fn with_path(&self, path: String) -> Self {
        Self {
            inner: self.inner.clone().with_path(&path),
        }
    }

    /// Returns a copy of this URI with the query set.
    #[napi]
    pub fn with_query(&self, query: String) -> Self {
        Self {
            inner: self.inner.clone().with_query(&query),
        }
    }

    /// Returns a copy of this URI with the fragment set.
    #[napi]
    pub fn with_fragment(&self, fragment: String) -> Self {
        Self {
            inner: self.inner.clone().with_fragment(&fragment),
        }
    }

    // ---- in-place setters --------------------------------------------------------------

    /// Sets the scheme.
    #[napi]
    pub fn set_scheme(&mut self, scheme: String) {
        self.inner.set_scheme(&scheme);
    }

    /// Replaces the whole authority (pass `null` to drop it).
    #[napi]
    pub fn set_authority(&mut self, authority: Option<&Authority>) {
        self.inner.set_authority(authority.map(|a| a.inner.clone()));
    }

    /// Sets the host, creating an authority if this URI had none.
    #[napi]
    pub fn set_host(&mut self, host: String) {
        self.inner.set_host(&host);
    }

    /// Sets the port, creating an authority if this URI had none.
    #[napi]
    pub fn set_port(&mut self, port: u16) {
        self.inner.set_port(port);
    }

    /// Sets the userinfo user, creating an authority if this URI had none.
    #[napi]
    pub fn set_user(&mut self, user: String) {
        self.inner.set_user(&user);
    }

    /// Sets the userinfo password, creating an authority if this URI had none.
    #[napi]
    pub fn set_password(&mut self, password: String) {
        self.inner.set_password(&password);
    }

    /// Sets the path, re-normalizing back-slashes to forward slashes.
    #[napi]
    pub fn set_path(&mut self, path: String) {
        self.inner.set_path(&path);
    }

    /// Sets the query.
    #[napi]
    pub fn set_query(&mut self, query: String) {
        self.inner.set_query(&query);
    }

    /// Sets the fragment.
    #[napi]
    pub fn set_fragment(&mut self, fragment: String) {
        self.inner.set_fragment(&fragment);
    }

    // ---- byte codec + interchange ------------------------------------------------------

    /// The canonical URI string as UTF-8 bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Decodes a URI from the UTF-8 bytes produced by `serializeBytes` — the exact inverse.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        core::Uri::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Converts to a [`Url`], throwing a guided `Error` if this URI has no scheme.
    #[napi]
    pub fn to_url(&self) -> napi::Result<Url> {
        self.inner
            .to_url()
            .map(|inner| Url { inner })
            .map_err(to_error)
    }

    // ---- combinators (copy / joinpath / merge) -----------------------------------------

    /// An explicit copy of this URI, optionally overriding components via `copy({ scheme, user,
    /// password, host, port, path, query, fragment })` — each present option replaces that
    /// component (creating an authority where needed), an absent one is kept. With no argument it
    /// is a plain clone.
    #[napi]
    pub fn copy(&self, options: Option<UriCopyOptions>) -> Self {
        let mut inner = self.inner.clone();
        if let Some(options) = options {
            if let Some(scheme) = options.scheme {
                inner = inner.with_scheme(&scheme);
            }
            if let Some(user) = options.user {
                inner = inner.with_user(&user);
            }
            if let Some(password) = options.password {
                inner = inner.with_password(&password);
            }
            if let Some(host) = options.host {
                inner = inner.with_host(&host);
            }
            if let Some(port) = options.port {
                inner = inner.with_port(port);
            }
            if let Some(path) = options.path {
                inner = inner.with_path(&path);
            }
            if let Some(query) = options.query {
                inner = inner.with_query(&query);
            }
            if let Some(fragment) = options.fragment {
                inner = inner.with_fragment(&fragment);
            }
        }
        Self { inner }
    }

    /// Returns a copy with `path` joined lexically onto the path (one `/` at the seam, an
    /// absolute segment resets it, other components kept). Encoded like `setPath`.
    #[napi]
    pub fn joinpath(&self, path: String) -> Self {
        Self {
            inner: self.inner.joinpath(&path),
        }
    }

    /// Returns a copy overlaid by `other`: each component `other` sets wins, else this URI's
    /// is kept.
    #[napi]
    pub fn merge_with(&self, other: &Uri) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    // ---- query parameters (map access + CRUD) ------------------------------------------

    /// The first value of query parameter `key`, **decoded** by default; pass `true` for the
    /// stored (percent-encoded) form. `null` if absent.
    #[napi]
    pub fn param(&self, key: String, encoded: Option<bool>) -> Option<String> {
        if encoded.unwrap_or(false) {
            self.inner.param(&key).map(str::to_string)
        } else {
            self.inner
                .param_decoded(&key)
                .map(|value| value.into_owned())
        }
    }

    /// Every value of query parameter `key`, in order, decoded by default (`true` for stored).
    #[napi]
    pub fn param_all(&self, key: String, encoded: Option<bool>) -> Vec<String> {
        if encoded.unwrap_or(false) {
            self.inner
                .param_all(&key)
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            self.inner
                .param_all_decoded(&key)
                .into_iter()
                .map(|value| value.into_owned())
                .collect()
        }
    }

    /// All query parameters **grouped by key** as an ordered `Map` from each key to the array of
    /// its values, in **first-appearance** key order — e.g. `?a=1&b=2&a=3` →
    /// `Map { "a" => ["1", "3"], "b" => ["2"] }`. A `Map` (not a plain object) is used so that
    /// numeric-looking keys keep insertion order instead of being reordered numerically. Values
    /// are the stored (percent-encoded) form; use `param` / `paramAll` to decode.
    #[napi(ts_return_type = "Map<string, string[]>")]
    pub fn params(&self, env: Env) -> napi::Result<JsObject> {
        let map_ctor = env.get_global()?.get_named_property::<JsFunction>("Map")?;
        let map = map_ctor.new_instance::<JsUnknown>(&[])?;
        let set_fn = map.get_named_property::<JsFunction>("set")?;
        for (key, values) in self.inner.params_grouped() {
            let js_key = env.create_string(key)?;
            let mut js_values = env.create_array_with_length(values.len())?;
            for (index, value) in values.iter().enumerate() {
                js_values.set_element(index as u32, env.create_string(value)?)?;
            }
            set_fn.call(
                Some(&map),
                &[js_key.into_unknown(), js_values.into_unknown()],
            )?;
        }
        Ok(map)
    }

    /// Whether query parameter `key` is present.
    #[napi]
    pub fn has_param(&self, key: String) -> bool {
        self.inner.has_param(&key)
    }

    /// Sets query parameter `key` to `value` (first occurrence updated, later dupes dropped,
    /// or appended if absent). The value is stored verbatim.
    #[napi]
    pub fn set_param(&mut self, key: String, value: String) {
        self.inner.set_param(&key, &value);
    }

    /// Returns a copy with query parameter `key` set.
    #[napi]
    pub fn with_param(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.clone().with_param(&key, &value),
        }
    }

    /// Removes every occurrence of query parameter `key`; returns whether any were removed.
    #[napi]
    pub fn remove_param(&mut self, key: String) -> bool {
        self.inner.remove_param(&key)
    }

    /// Returns a copy with query parameter `key` removed.
    #[napi]
    pub fn without_param(&self, key: String) -> Self {
        Self {
            inner: self.inner.clone().without_param(&key),
        }
    }

    /// Bulk-updates query parameters from `[key, value]` pairs in one pass (last value wins
    /// per key). Pass `Object.entries(obj)` to apply an object.
    #[napi]
    pub fn set_params(&mut self, params: Vec<Vec<String>>) {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|pair| {
                (
                    pair.first().map(String::as_str).unwrap_or(""),
                    pair.get(1).map(String::as_str).unwrap_or(""),
                )
            })
            .collect();
        self.inner.set_params(&refs);
    }

    /// Returns a copy with the bulk update applied.
    #[napi]
    pub fn with_params(&self, params: Vec<Vec<String>>) -> Self {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|pair| {
                (
                    pair.first().map(String::as_str).unwrap_or(""),
                    pair.get(1).map(String::as_str).unwrap_or(""),
                )
            })
            .collect();
        Self {
            inner: self.inner.clone().with_params(&refs),
        }
    }

    /// Normalizes the query: drops empty tokens and stable-sorts parameters by key.
    #[napi]
    pub fn normalize_params(&mut self) {
        self.inner.normalize_params();
    }

    /// Returns a copy with the query normalized.
    #[napi]
    pub fn with_normalized_params(&self) -> Self {
        Self {
            inner: self.inner.clone().with_normalized_params(),
        }
    }

    /// Content equality (equal iff `serializeBytes` are equal).
    #[napi]
    pub fn equals(&self, other: &Uri) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The canonical URI string.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}

/// An **absolute** URI: a [`Uri`] guaranteed to carry a scheme. The authority stays
/// optional, so `mailto:user@host` and `file:/etc/x` are valid `Url`s with no `//`
/// authority — only the scheme is required, which is why `scheme` is never `null`.
#[napi(namespace = "uri")]
pub struct Url {
    pub(crate) inner: core::Url,
}

#[napi(namespace = "uri")]
impl Url {
    /// Parses `s` into an absolute URL, throwing a guided `Error` if it has no scheme (or on
    /// any other parse failure).
    #[napi(factory)]
    pub fn parse(s: String) -> napi::Result<Self> {
        core::Url::parse_str(&s)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The scheme (always present).
    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().to_string()
    }

    /// The authority — an empty `Authority` when the URL has none (a `mailto:` / `file:` URL);
    /// use `hasAuthority` to test presence.
    #[napi(getter)]
    pub fn authority(&self) -> Authority {
        Authority {
            inner: self.inner.authority(),
        }
    }

    /// Whether this URL carries a `//` authority.
    #[napi(getter)]
    pub fn has_authority(&self) -> bool {
        self.inner.has_authority()
    }

    /// The userinfo user, if any.
    #[napi(getter)]
    pub fn user(&self) -> Option<String> {
        self.inner.user().map(str::to_string)
    }

    /// The userinfo password, if any.
    #[napi(getter)]
    pub fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    /// The host — an empty string when the URL has no authority (an IPv6 literal keeps its
    /// brackets).
    #[napi(getter)]
    pub fn host(&self) -> String {
        self.inner.host().to_string()
    }

    /// Whether the host is a bracketed IPv6 literal (`false` if it has no authority).
    #[napi(getter)]
    pub fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped, if this URL has an authority.
    #[napi(getter)]
    pub fn host_unbracketed(&self) -> Option<String> {
        self.inner.host_unbracketed().map(str::to_string)
    }

    /// The port as written, if any (see `portOrDefault` for the effective port).
    #[napi(getter)]
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// The default port registered for this URL's scheme, or `null` if it has no known default.
    #[napi(getter)]
    pub fn default_port(&self) -> Option<u16> {
        self.inner.default_port()
    }

    /// The effective port to connect to: the explicit `port`, else the scheme's `defaultPort`.
    #[napi(getter)]
    pub fn port_or_default(&self) -> Option<u16> {
        self.inner.port_or_default()
    }

    /// The path, always POSIX slash-normalized.
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().to_string()
    }

    /// The query, if any.
    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner.query().map(str::to_string)
    }

    /// The fragment, if any.
    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner.fragment().map(str::to_string)
    }

    /// The last non-empty path segment (the filename), or `null` for a directory-like path.
    #[napi(getter)]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// The filename without its last extension.
    #[napi(getter)]
    pub fn stem(&self) -> Option<String> {
        self.inner.stem().map(str::to_string)
    }

    /// The last extension of the filename (without the dot).
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(str::to_string)
    }

    /// Every extension of a multi-dot filename, outermost-last.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    // ---- builder mutators (return a new `Url`) -----------------------------------------

    /// Returns a copy of this URL with the scheme set.
    #[napi]
    pub fn with_scheme(&self, scheme: String) -> Self {
        Self {
            inner: self.inner.clone().with_scheme(&scheme),
        }
    }

    /// Returns a copy of this URL with the whole authority replaced (pass `null` to drop it).
    #[napi]
    pub fn with_authority(&self, authority: Option<&Authority>) -> Self {
        Self {
            inner: self
                .inner
                .clone()
                .with_authority(authority.map(|a| a.inner.clone())),
        }
    }

    /// Returns a copy of this URL with the host set.
    #[napi]
    pub fn with_host(&self, host: String) -> Self {
        Self {
            inner: self.inner.clone().with_host(&host),
        }
    }

    /// Returns a copy of this URL with the port set.
    #[napi]
    pub fn with_port(&self, port: u16) -> Self {
        Self {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Returns a copy of this URL with the userinfo user set.
    #[napi]
    pub fn with_user(&self, user: String) -> Self {
        Self {
            inner: self.inner.clone().with_user(&user),
        }
    }

    /// Returns a copy of this URL with the userinfo password set.
    #[napi]
    pub fn with_password(&self, password: String) -> Self {
        Self {
            inner: self.inner.clone().with_password(&password),
        }
    }

    /// Returns a copy of this URL with the path set, re-normalized to POSIX slashes.
    #[napi]
    pub fn with_path(&self, path: String) -> Self {
        Self {
            inner: self.inner.clone().with_path(&path),
        }
    }

    /// Returns a copy of this URL with the query set.
    #[napi]
    pub fn with_query(&self, query: String) -> Self {
        Self {
            inner: self.inner.clone().with_query(&query),
        }
    }

    /// Returns a copy of this URL with the fragment set.
    #[napi]
    pub fn with_fragment(&self, fragment: String) -> Self {
        Self {
            inner: self.inner.clone().with_fragment(&fragment),
        }
    }

    // ---- in-place setters --------------------------------------------------------------

    /// Sets the scheme.
    #[napi]
    pub fn set_scheme(&mut self, scheme: String) {
        self.inner.set_scheme(&scheme);
    }

    /// Replaces the whole authority (pass `null` to drop it).
    #[napi]
    pub fn set_authority(&mut self, authority: Option<&Authority>) {
        self.inner.set_authority(authority.map(|a| a.inner.clone()));
    }

    /// Sets the host.
    #[napi]
    pub fn set_host(&mut self, host: String) {
        self.inner.set_host(&host);
    }

    /// Sets the port.
    #[napi]
    pub fn set_port(&mut self, port: u16) {
        self.inner.set_port(port);
    }

    /// Sets the userinfo user.
    #[napi]
    pub fn set_user(&mut self, user: String) {
        self.inner.set_user(&user);
    }

    /// Sets the userinfo password.
    #[napi]
    pub fn set_password(&mut self, password: String) {
        self.inner.set_password(&password);
    }

    /// Sets the path, re-normalizing back-slashes to forward slashes.
    #[napi]
    pub fn set_path(&mut self, path: String) {
        self.inner.set_path(&path);
    }

    /// Sets the query.
    #[napi]
    pub fn set_query(&mut self, query: String) {
        self.inner.set_query(&query);
    }

    /// Sets the fragment.
    #[napi]
    pub fn set_fragment(&mut self, fragment: String) {
        self.inner.set_fragment(&fragment);
    }

    // ---- byte codec + interchange ------------------------------------------------------

    /// The canonical URL string as UTF-8 bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Decodes a URL from the UTF-8 bytes produced by `serializeBytes`, throwing a guided
    /// `Error` if the decoded URI is not absolute (or on any parse failure).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        core::Url::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Converts to the underlying [`Uri`] (infallible — a URL is always a URI).
    #[napi]
    pub fn to_uri(&self) -> Uri {
        Uri {
            inner: self.inner.as_uri().clone(),
        }
    }

    // ---- combinators (copy / joinpath / merge) -----------------------------------------

    /// An explicit copy of this URL, optionally overriding components via `copy({ scheme, user,
    /// password, host, port, path, query, fragment })` — each present option replaces that
    /// component, an absent one is kept. With no argument it is a plain clone.
    #[napi]
    pub fn copy(&self, options: Option<UriCopyOptions>) -> Self {
        let mut inner = self.inner.clone();
        if let Some(options) = options {
            if let Some(scheme) = options.scheme {
                inner = inner.with_scheme(&scheme);
            }
            if let Some(user) = options.user {
                inner = inner.with_user(&user);
            }
            if let Some(password) = options.password {
                inner = inner.with_password(&password);
            }
            if let Some(host) = options.host {
                inner = inner.with_host(&host);
            }
            if let Some(port) = options.port {
                inner = inner.with_port(port);
            }
            if let Some(path) = options.path {
                inner = inner.with_path(&path);
            }
            if let Some(query) = options.query {
                inner = inner.with_query(&query);
            }
            if let Some(fragment) = options.fragment {
                inner = inner.with_fragment(&fragment);
            }
        }
        Self { inner }
    }

    /// Returns a copy with `path` joined lexically onto the path — see [`Uri.joinpath`]. The
    /// scheme is kept, so the result is still an absolute URL.
    #[napi]
    pub fn joinpath(&self, path: String) -> Self {
        Self {
            inner: self.inner.joinpath(&path),
        }
    }

    /// Returns a copy overlaid by `other`: each component `other` sets wins, else this URL's
    /// is kept.
    #[napi]
    pub fn merge_with(&self, other: &Url) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    // ---- query parameters (map access + CRUD) ------------------------------------------

    /// The first value of query parameter `key`, **decoded** by default; pass `true` for the
    /// stored (percent-encoded) form. `null` if absent.
    #[napi]
    pub fn param(&self, key: String, encoded: Option<bool>) -> Option<String> {
        if encoded.unwrap_or(false) {
            self.inner.param(&key).map(str::to_string)
        } else {
            self.inner
                .param_decoded(&key)
                .map(|value| value.into_owned())
        }
    }

    /// Every value of query parameter `key`, in order, decoded by default (`true` for stored).
    #[napi]
    pub fn param_all(&self, key: String, encoded: Option<bool>) -> Vec<String> {
        if encoded.unwrap_or(false) {
            self.inner
                .param_all(&key)
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            self.inner
                .param_all_decoded(&key)
                .into_iter()
                .map(|value| value.into_owned())
                .collect()
        }
    }

    /// All query parameters **grouped by key** as an ordered `Map` from each key to the array of
    /// its values, in **first-appearance** key order — e.g. `?a=1&b=2&a=3` →
    /// `Map { "a" => ["1", "3"], "b" => ["2"] }`. A `Map` (not a plain object) is used so that
    /// numeric-looking keys keep insertion order instead of being reordered numerically. Values
    /// are the stored (percent-encoded) form; use `param` / `paramAll` to decode.
    #[napi(ts_return_type = "Map<string, string[]>")]
    pub fn params(&self, env: Env) -> napi::Result<JsObject> {
        let map_ctor = env.get_global()?.get_named_property::<JsFunction>("Map")?;
        let map = map_ctor.new_instance::<JsUnknown>(&[])?;
        let set_fn = map.get_named_property::<JsFunction>("set")?;
        for (key, values) in self.inner.params_grouped() {
            let js_key = env.create_string(key)?;
            let mut js_values = env.create_array_with_length(values.len())?;
            for (index, value) in values.iter().enumerate() {
                js_values.set_element(index as u32, env.create_string(value)?)?;
            }
            set_fn.call(
                Some(&map),
                &[js_key.into_unknown(), js_values.into_unknown()],
            )?;
        }
        Ok(map)
    }

    /// Whether query parameter `key` is present.
    #[napi]
    pub fn has_param(&self, key: String) -> bool {
        self.inner.has_param(&key)
    }

    /// Sets query parameter `key` to `value` (first occurrence updated, later dupes dropped,
    /// or appended if absent). The value is stored verbatim.
    #[napi]
    pub fn set_param(&mut self, key: String, value: String) {
        self.inner.set_param(&key, &value);
    }

    /// Returns a copy with query parameter `key` set.
    #[napi]
    pub fn with_param(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.clone().with_param(&key, &value),
        }
    }

    /// Removes every occurrence of query parameter `key`; returns whether any were removed.
    #[napi]
    pub fn remove_param(&mut self, key: String) -> bool {
        self.inner.remove_param(&key)
    }

    /// Returns a copy with query parameter `key` removed.
    #[napi]
    pub fn without_param(&self, key: String) -> Self {
        Self {
            inner: self.inner.clone().without_param(&key),
        }
    }

    /// Bulk-updates query parameters from `[key, value]` pairs in one pass (last value wins
    /// per key). Pass `Object.entries(obj)` to apply an object.
    #[napi]
    pub fn set_params(&mut self, params: Vec<Vec<String>>) {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|pair| {
                (
                    pair.first().map(String::as_str).unwrap_or(""),
                    pair.get(1).map(String::as_str).unwrap_or(""),
                )
            })
            .collect();
        self.inner.set_params(&refs);
    }

    /// Returns a copy with the bulk update applied.
    #[napi]
    pub fn with_params(&self, params: Vec<Vec<String>>) -> Self {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|pair| {
                (
                    pair.first().map(String::as_str).unwrap_or(""),
                    pair.get(1).map(String::as_str).unwrap_or(""),
                )
            })
            .collect();
        Self {
            inner: self.inner.clone().with_params(&refs),
        }
    }

    /// Normalizes the query: drops empty tokens and stable-sorts parameters by key.
    #[napi]
    pub fn normalize_params(&mut self) {
        self.inner.normalize_params();
    }

    /// Returns a copy with the query normalized.
    #[napi]
    pub fn with_normalized_params(&self) -> Self {
        Self {
            inner: self.inner.clone().with_normalized_params(),
        }
    }

    /// Content equality (equal iff `serializeBytes` are equal).
    #[napi]
    pub fn equals(&self, other: &Url) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The canonical URL string.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}
