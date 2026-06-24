//! The `Uri` napi class.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_url::{FromInput, ToOutput, Uri as CoreUri};

use crate::media::MediaType;
use crate::mime::MimeType;
use crate::to_mapping;
use crate::url::Url;

/// A generic RFC 3986 URI: `scheme:[//authority]path[?query][#fragment]`.
#[napi]
pub struct Uri {
    pub(crate) inner: CoreUri,
}

#[napi]
impl Uri {
    /// Parse `value` into a `Uri`, throwing on failure. The scheme and any `%XX`
    /// escapes are validated.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreUri::from_str(&value)
            .map(|inner| Uri { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Parse a string (alias of the constructor).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        Uri::new(value)
    }

    /// Build a `Uri` from an object of components (`scheme`, `authority`, `path`,
    /// `query`, `fragment`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreUri::from_mapping(&to_mapping(fields))
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
