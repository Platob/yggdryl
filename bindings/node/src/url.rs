//! The `Url` napi class.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_url::{ToOutput, Url as CoreUrl};

use crate::media::MediaType;
use crate::mime::MimeType;
use crate::to_mapping;
use crate::uri::Uri;

/// A URL: a URI that always has an authority, split into `username`, `password`,
/// `host` and `port`.
#[napi]
pub struct Url {
    pub(crate) inner: CoreUrl,
}

#[napi]
impl Url {
    /// Parse `value` into a `Url`, throwing on failure. The scheme and any `%XX`
    /// escapes are validated.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreUrl::from_str(&value)
            .map(|inner| Url { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Parse a string (alias of the constructor).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        Url::new(value)
    }

    /// Build a `Url` from an object of components (`scheme` and `host` required;
    /// `username`, `password`, `port`, `path`, `query`, `fragment`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreUrl::from_mapping(&to_mapping(fields))
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

    /// The non-empty path segments; `encode` keeps the percent-encoded form.
    #[napi]
    pub fn parts(&self, encode: Option<bool>) -> Vec<String> {
        self.inner.parts(encode.unwrap_or(false))
    }

    /// The file name (last path segment).
    #[napi]
    pub fn name(&self, encode: Option<bool>) -> String {
        self.inner.name(encode.unwrap_or(false))
    }

    /// The file name without its extensions.
    #[napi]
    pub fn stem(&self, encode: Option<bool>) -> String {
        self.inner.stem(encode.unwrap_or(false))
    }

    /// The file name's extensions, e.g. `["tar", "gz"]`.
    #[napi]
    pub fn extensions(&self, encode: Option<bool>) -> Vec<String> {
        self.inner.extensions(encode.unwrap_or(false))
    }

    /// The media type stack inferred from the path's file extensions, or `null`.
    #[napi(js_name = "mediaType")]
    pub fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// The outermost MIME type inferred from the path's last extension, or `null`.
    #[napi(js_name = "mimeType")]
    pub fn mime_type(&self) -> Option<MimeType> {
        self.inner.mime_type().map(|inner| MimeType { inner })
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
