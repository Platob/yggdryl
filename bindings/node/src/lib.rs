//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers around [`yggdryl_url::Uri`] and [`yggdryl_url::Url`];
//! all parsing lives in the shared Rust core so the Node and Python bindings stay
//! in lockstep.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_url::{
    percent_decode, percent_encode, FromInput, Mapping, ToOutput, Uri as CoreUri, Url as CoreUrl,
};
use yggdryl_version::Version as CoreVersion;

/// Converts a JS object (`HashMap`) into the core ordered [`Mapping`].
fn to_mapping(fields: HashMap<String, String>) -> Mapping {
    fields.into_iter().collect()
}

/// URL-safe percent-encode `input` (e.g. a space becomes `%20`).
#[napi(js_name = "percentEncode")]
pub fn percent_encode_js(input: String) -> String {
    percent_encode(&input)
}

/// Percent-decode `input`, throwing on a malformed escape.
#[napi(js_name = "percentDecode")]
pub fn percent_decode_js(input: String) -> Result<String> {
    percent_decode(&input).map_err(|e| Error::from_reason(e.to_string()))
}

/// A generic RFC 3986 URI: `scheme:[//authority]path[?query][#fragment]`.
#[napi]
pub struct Uri {
    inner: CoreUri,
}

#[napi]
impl Uri {
    /// Parse `value` into a `Uri`, throwing on failure. With `safe = false` the
    /// scheme and `%XX` escapes are not validated.
    #[napi(constructor)]
    pub fn new(value: String, safe: Option<bool>) -> Result<Self> {
        CoreUri::from_str(&value, safe.unwrap_or(true))
            .map(|inner| Uri { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Parse a string (alias of the constructor).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String, safe: Option<bool>) -> Result<Self> {
        Uri::new(value, safe)
    }

    /// Build a `Uri` from an object of components (`scheme`, `authority`, `path`,
    /// `query`, `fragment`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>, safe: Option<bool>) -> Result<Self> {
        CoreUri::from_mapping(&to_mapping(fields), safe.unwrap_or(true))
            .map(|inner| Uri { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build a `Uri` directly from its parts (no string parsing).
    #[napi(factory, js_name = "fromParts")]
    pub fn from_parts(
        scheme: String,
        path: Option<String>,
        authority: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Self {
        Uri {
            inner: CoreUri::from_parts(
                scheme,
                authority,
                path.unwrap_or_default(),
                query,
                fragment,
            ),
        }
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    /// `copy()` clones; `copy(null, null, '/x')` clones with one field changed.
    #[napi]
    pub fn copy(
        &self,
        scheme: Option<String>,
        authority: Option<String>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Uri {
        Uri {
            inner: self.inner.copy(scheme, authority, path, query, fragment),
        }
    }

    /// Return a copy with the scheme replaced.
    #[napi(js_name = "withScheme")]
    pub fn with_scheme(&self, scheme: String) -> Uri {
        Uri {
            inner: self.inner.clone().with_scheme(scheme),
        }
    }

    /// Return a copy with the authority set.
    #[napi(js_name = "withAuthority")]
    pub fn with_authority(&self, authority: String) -> Uri {
        Uri {
            inner: self.inner.clone().with_authority(authority),
        }
    }

    /// Return a copy with the authority removed.
    #[napi(js_name = "withoutAuthority")]
    pub fn without_authority(&self) -> Uri {
        Uri {
            inner: self.inner.clone().without_authority(),
        }
    }

    /// Return a copy with the path replaced.
    #[napi(js_name = "withPath")]
    pub fn with_path(&self, path: String) -> Uri {
        Uri {
            inner: self.inner.clone().with_path(path),
        }
    }

    /// Return a copy with the query set.
    #[napi(js_name = "withQuery")]
    pub fn with_query(&self, query: String) -> Uri {
        Uri {
            inner: self.inner.clone().with_query(query),
        }
    }

    /// Return a copy with the query removed.
    #[napi(js_name = "withoutQuery")]
    pub fn without_query(&self) -> Uri {
        Uri {
            inner: self.inner.clone().without_query(),
        }
    }

    /// Return a copy with the fragment set.
    #[napi(js_name = "withFragment")]
    pub fn with_fragment(&self, fragment: String) -> Uri {
        Uri {
            inner: self.inner.clone().with_fragment(fragment),
        }
    }

    /// Return a copy with the fragment removed.
    #[napi(js_name = "withoutFragment")]
    pub fn without_fragment(&self) -> Uri {
        Uri {
            inner: self.inner.clone().without_fragment(),
        }
    }

    /// Return the query as an object of key -> values; `decode` percent-decodes.
    #[napi(js_name = "params")]
    pub fn params(&self, decode: Option<bool>) -> HashMap<String, Vec<String>> {
        self.inner
            .params(decode.unwrap_or(true))
            .into_iter()
            .collect()
    }

    /// Return a copy whose query is built from `params`; `encode` percent-encodes.
    #[napi(js_name = "withParams")]
    pub fn with_params(&self, params: HashMap<String, Vec<String>>, encode: Option<bool>) -> Uri {
        Uri {
            inner: self
                .inner
                .clone()
                .with_params(&params.into_iter().collect(), encode.unwrap_or(true)),
        }
    }

    /// Return a copy with `key` set to `values`, adding or replacing it.
    #[napi(js_name = "addParam")]
    pub fn add_param(&self, key: String, values: Vec<String>, encode: Option<bool>) -> Uri {
        Uri {
            inner: self.inner.add_param(key, values, encode.unwrap_or(true)),
        }
    }

    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().to_string()
    }

    #[napi(getter)]
    pub fn authority(&self) -> Option<String> {
        self.inner.authority().map(str::to_string)
    }

    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().to_string()
    }

    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner.query().map(str::to_string)
    }

    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner.fragment().map(str::to_string)
    }

    /// Base scheme before any `+` extension (e.g. `https` for `https+zip`).
    #[napi(getter, js_name = "schemeBase")]
    pub fn scheme_base(&self) -> String {
        self.inner.scheme_base().to_string()
    }

    /// The `+`-separated scheme extensions (e.g. `["zip"]`).
    #[napi(getter, js_name = "schemeExt")]
    pub fn scheme_ext(&self) -> Vec<String> {
        self.inner
            .scheme_ext()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Build a `Uri` from a `Url`.
    #[napi(factory, js_name = "fromUrl")]
    pub fn from_url(url: &Url) -> Uri {
        Uri {
            inner: CoreUri::from_url(&url.inner),
        }
    }

    /// Parse this URI into a `Url` (requires an authority and host).
    #[napi(js_name = "toUrl")]
    pub fn to_url(&self) -> Result<Url> {
        self.inner
            .to_url()
            .map(|inner| Url { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Decoded values of one query parameter, or `null`.
    #[napi(js_name = "getParam")]
    pub fn get_param(&self, key: String) -> Option<Vec<String>> {
        self.inner.get_param(&key)
    }

    /// Whether the query has a parameter named `key`.
    #[napi(js_name = "hasParam")]
    pub fn has_param(&self, key: String) -> bool {
        self.inner.has_param(&key)
    }

    /// Return a copy with one parameter created or replaced (single update).
    #[napi(js_name = "setParam")]
    pub fn set_param(&self, key: String, values: Vec<String>, encode: Option<bool>) -> Uri {
        Uri {
            inner: self.inner.set_param(key, values, encode.unwrap_or(true)),
        }
    }

    /// Return a copy with every entry of `params` set, others untouched (bulk).
    #[napi(js_name = "setParams")]
    pub fn set_params(&self, params: HashMap<String, Vec<String>>, encode: Option<bool>) -> Uri {
        Uri {
            inner: self
                .inner
                .set_params(&params.into_iter().collect(), encode.unwrap_or(true)),
        }
    }

    /// Return a copy with one parameter removed (single delete).
    #[napi(js_name = "removeParam")]
    pub fn remove_param(&self, key: String, encode: Option<bool>) -> Uri {
        Uri {
            inner: self.inner.remove_param(&key, encode.unwrap_or(true)),
        }
    }

    /// Return a copy with several parameters removed (bulk delete).
    #[napi(js_name = "removeParams")]
    pub fn remove_params(&self, keys: Vec<String>, encode: Option<bool>) -> Uri {
        Uri {
            inner: self.inner.remove_params(&keys, encode.unwrap_or(true)),
        }
    }

    /// Return a copy with the entire query removed.
    #[napi(js_name = "clearParams")]
    pub fn clear_params(&self) -> Uri {
        Uri {
            inner: self.inner.clear_params(),
        }
    }

    /// Render the URI; `encode` (default) percent-encodes, else decodes.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self, encode: Option<bool>) -> String {
        self.inner.to_str(encode.unwrap_or(true))
    }

    /// Render to a component object (the inverse of `fromMapping`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> std::collections::HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }
}

/// A URL: a URI that always has an authority, split into `username`, `password`,
/// `host` and `port`.
#[napi]
pub struct Url {
    inner: CoreUrl,
}

#[napi]
impl Url {
    /// Parse `value` into a `Url`, throwing on failure. With `safe = false` the
    /// scheme and `%XX` escapes are not validated.
    #[napi(constructor)]
    pub fn new(value: String, safe: Option<bool>) -> Result<Self> {
        CoreUrl::from_str(&value, safe.unwrap_or(true))
            .map(|inner| Url { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Parse a string (alias of the constructor).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String, safe: Option<bool>) -> Result<Self> {
        Url::new(value, safe)
    }

    /// Build a `Url` from an object of components (`scheme` and `host` required;
    /// `username`, `password`, `port`, `path`, `query`, `fragment`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>, safe: Option<bool>) -> Result<Self> {
        CoreUrl::from_mapping(&to_mapping(fields), safe.unwrap_or(true))
            .map(|inner| Url { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build a `Url` directly from its parts (no string parsing).
    #[napi(factory, js_name = "fromParts")]
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        scheme: String,
        host: String,
        port: Option<u16>,
        username: Option<String>,
        password: Option<String>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Self {
        Url {
            inner: CoreUrl::from_parts(
                scheme,
                username,
                password,
                host,
                port,
                path.unwrap_or_default(),
                query,
                fragment,
            ),
        }
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    /// `copy()` clones; `copy(null, …, 443)` clones with one field changed.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn copy(
        &self,
        scheme: Option<String>,
        username: Option<String>,
        password: Option<String>,
        host: Option<String>,
        port: Option<u16>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Url {
        Url {
            inner: self.inner.copy(
                scheme, username, password, host, port, path, query, fragment,
            ),
        }
    }

    /// Return a copy with the scheme replaced.
    #[napi(js_name = "withScheme")]
    pub fn with_scheme(&self, scheme: String) -> Url {
        Url {
            inner: self.inner.clone().with_scheme(scheme),
        }
    }

    /// Return a copy with the username set.
    #[napi(js_name = "withUsername")]
    pub fn with_username(&self, username: String) -> Url {
        Url {
            inner: self.inner.clone().with_username(username),
        }
    }

    /// Return a copy with the password set.
    #[napi(js_name = "withPassword")]
    pub fn with_password(&self, password: String) -> Url {
        Url {
            inner: self.inner.clone().with_password(password),
        }
    }

    /// Return a copy with username and password removed.
    #[napi(js_name = "withoutUserinfo")]
    pub fn without_userinfo(&self) -> Url {
        Url {
            inner: self.inner.clone().without_userinfo(),
        }
    }

    /// Return a copy with the host replaced.
    #[napi(js_name = "withHost")]
    pub fn with_host(&self, host: String) -> Url {
        Url {
            inner: self.inner.clone().with_host(host),
        }
    }

    /// Return a copy with the port set.
    #[napi(js_name = "withPort")]
    pub fn with_port(&self, port: u16) -> Url {
        Url {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Return a copy with the port removed.
    #[napi(js_name = "withoutPort")]
    pub fn without_port(&self) -> Url {
        Url {
            inner: self.inner.clone().without_port(),
        }
    }

    /// Return a copy with the path replaced.
    #[napi(js_name = "withPath")]
    pub fn with_path(&self, path: String) -> Url {
        Url {
            inner: self.inner.clone().with_path(path),
        }
    }

    /// Return a copy with the query set.
    #[napi(js_name = "withQuery")]
    pub fn with_query(&self, query: String) -> Url {
        Url {
            inner: self.inner.clone().with_query(query),
        }
    }

    /// Return a copy with the query removed.
    #[napi(js_name = "withoutQuery")]
    pub fn without_query(&self) -> Url {
        Url {
            inner: self.inner.clone().without_query(),
        }
    }

    /// Return a copy with the fragment set.
    #[napi(js_name = "withFragment")]
    pub fn with_fragment(&self, fragment: String) -> Url {
        Url {
            inner: self.inner.clone().with_fragment(fragment),
        }
    }

    /// Return a copy with the fragment removed.
    #[napi(js_name = "withoutFragment")]
    pub fn without_fragment(&self) -> Url {
        Url {
            inner: self.inner.clone().without_fragment(),
        }
    }

    /// Return the query as an object of key -> values; `decode` percent-decodes.
    #[napi(js_name = "params")]
    pub fn params(&self, decode: Option<bool>) -> HashMap<String, Vec<String>> {
        self.inner
            .params(decode.unwrap_or(true))
            .into_iter()
            .collect()
    }

    /// Return a copy whose query is built from `params`; `encode` percent-encodes.
    #[napi(js_name = "withParams")]
    pub fn with_params(&self, params: HashMap<String, Vec<String>>, encode: Option<bool>) -> Url {
        Url {
            inner: self
                .inner
                .clone()
                .with_params(&params.into_iter().collect(), encode.unwrap_or(true)),
        }
    }

    /// Return a copy with `key` set to `values`, adding or replacing it.
    #[napi(js_name = "addParam")]
    pub fn add_param(&self, key: String, values: Vec<String>, encode: Option<bool>) -> Url {
        Url {
            inner: self.inner.add_param(key, values, encode.unwrap_or(true)),
        }
    }

    /// Return this URL viewed as a generic `Uri`.
    #[napi(js_name = "toUri")]
    pub fn to_uri(&self) -> Uri {
        Uri {
            inner: self.inner.to_uri(),
        }
    }

    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().to_string()
    }

    #[napi(getter)]
    pub fn username(&self) -> Option<String> {
        self.inner.username().map(str::to_string)
    }

    #[napi(getter)]
    pub fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    #[napi(getter)]
    pub fn host(&self) -> String {
        self.inner.host().to_string()
    }

    #[napi(getter)]
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().to_string()
    }

    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner.query().map(str::to_string)
    }

    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner.fragment().map(str::to_string)
    }

    #[napi(getter)]
    pub fn authority(&self) -> String {
        self.inner.authority()
    }

    /// Base scheme before any `+` extension (e.g. `https` for `https+zip`).
    #[napi(getter, js_name = "schemeBase")]
    pub fn scheme_base(&self) -> String {
        self.inner.scheme_base().to_string()
    }

    /// The `+`-separated scheme extensions (e.g. `["zip"]`).
    #[napi(getter, js_name = "schemeExt")]
    pub fn scheme_ext(&self) -> Vec<String> {
        self.inner
            .scheme_ext()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Build a `Url` from a `Uri` (requires an authority and host).
    #[napi(factory, js_name = "fromUri")]
    pub fn from_uri(uri: &Uri) -> Result<Url> {
        CoreUrl::from_uri(&uri.inner)
            .map(|inner| Url { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Decoded values of one query parameter, or `null`.
    #[napi(js_name = "getParam")]
    pub fn get_param(&self, key: String) -> Option<Vec<String>> {
        self.inner.get_param(&key)
    }

    /// Whether the query has a parameter named `key`.
    #[napi(js_name = "hasParam")]
    pub fn has_param(&self, key: String) -> bool {
        self.inner.has_param(&key)
    }

    /// Return a copy with one parameter created or replaced (single update).
    #[napi(js_name = "setParam")]
    pub fn set_param(&self, key: String, values: Vec<String>, encode: Option<bool>) -> Url {
        Url {
            inner: self.inner.set_param(key, values, encode.unwrap_or(true)),
        }
    }

    /// Return a copy with every entry of `params` set, others untouched (bulk).
    #[napi(js_name = "setParams")]
    pub fn set_params(&self, params: HashMap<String, Vec<String>>, encode: Option<bool>) -> Url {
        Url {
            inner: self
                .inner
                .set_params(&params.into_iter().collect(), encode.unwrap_or(true)),
        }
    }

    /// Return a copy with one parameter removed (single delete).
    #[napi(js_name = "removeParam")]
    pub fn remove_param(&self, key: String, encode: Option<bool>) -> Url {
        Url {
            inner: self.inner.remove_param(&key, encode.unwrap_or(true)),
        }
    }

    /// Return a copy with several parameters removed (bulk delete).
    #[napi(js_name = "removeParams")]
    pub fn remove_params(&self, keys: Vec<String>, encode: Option<bool>) -> Url {
        Url {
            inner: self.inner.remove_params(&keys, encode.unwrap_or(true)),
        }
    }

    /// Return a copy with the entire query removed.
    #[napi(js_name = "clearParams")]
    pub fn clear_params(&self) -> Url {
        Url {
            inner: self.inner.clear_params(),
        }
    }

    /// Render the URL; `encode` (default) percent-encodes, else decodes.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self, encode: Option<bool>) -> String {
        self.inner.to_str(encode.unwrap_or(true))
    }

    /// Render to a component object (the inverse of `fromMapping`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> std::collections::HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }
}

/// A generic `major.minor.patch` version, ordered numerically.
#[napi]
pub struct Version {
    inner: CoreVersion,
}

#[napi]
impl Version {
    /// Construct from components.
    #[napi(constructor)]
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Version {
            inner: CoreVersion::new(major as u64, minor as u64, patch as u64),
        }
    }

    /// Parse a `major[.minor[.patch]]` string, throwing on failure. With
    /// `safe = false` extra components are ignored and junk becomes `0`.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String, safe: Option<bool>) -> Result<Self> {
        CoreVersion::from_str(&value, safe.unwrap_or(true))
            .map(|inner| Version { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build a `Version` from an object of components (`major`, `minor`, `patch`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>, safe: Option<bool>) -> Result<Self> {
        CoreVersion::from_mapping(&to_mapping(fields), safe.unwrap_or(true))
            .map(|inner| Version { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    #[napi]
    pub fn copy(&self, major: Option<u32>, minor: Option<u32>, patch: Option<u32>) -> Version {
        Version {
            inner: self.inner.copy(
                major.map(u64::from),
                minor.map(u64::from),
                patch.map(u64::from),
            ),
        }
    }

    /// Return a copy with the major component replaced.
    #[napi(js_name = "withMajor")]
    pub fn with_major(&self, major: u32) -> Version {
        Version {
            inner: self.inner.with_major(major as u64),
        }
    }

    /// Return a copy with the minor component replaced.
    #[napi(js_name = "withMinor")]
    pub fn with_minor(&self, minor: u32) -> Version {
        Version {
            inner: self.inner.with_minor(minor as u64),
        }
    }

    /// Return a copy with the patch component replaced.
    #[napi(js_name = "withPatch")]
    pub fn with_patch(&self, patch: u32) -> Version {
        Version {
            inner: self.inner.with_patch(patch as u64),
        }
    }

    /// Render to a component object (the inverse of `fromMapping`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> std::collections::HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    #[napi(getter)]
    pub fn major(&self) -> u32 {
        self.inner.major() as u32
    }

    #[napi(getter)]
    pub fn minor(&self) -> u32 {
        self.inner.minor() as u32
    }

    #[napi(getter)]
    pub fn patch(&self) -> u32 {
        self.inner.patch() as u32
    }

    /// Compare with another version: `-1`, `0` or `1`.
    #[napi]
    pub fn compare(&self, other: &Version) -> i32 {
        match self.inner.cmp(&other.inner) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `true` if the two versions are equal.
    #[napi]
    pub fn equals(&self, other: &Version) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
    }
}
