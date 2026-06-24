//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers around [`yggdryl_core::Uri`] and [`yggdryl_core::Url`];
//! all parsing lives in the shared Rust core so the Node and Python bindings stay
//! in lockstep.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{Uri as CoreUri, Url as CoreUrl};

/// A generic RFC 3986 URI: `scheme:[//authority]path[?query][#fragment]`.
#[napi]
pub struct Uri {
    inner: CoreUri,
}

#[napi]
impl Uri {
    /// Parse `value` into a `Uri`, throwing on failure.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreUri::parse(&value)
            .map(|inner| Uri { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Alias for the constructor.
    #[napi(factory)]
    pub fn parse(value: String) -> Result<Self> {
        Uri::new(value)
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

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
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
    /// Parse `value` into a `Url`, throwing on failure.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreUrl::parse(&value)
            .map(|inner| Url { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Alias for the constructor.
    #[napi(factory)]
    pub fn parse(value: String) -> Result<Self> {
        Url::new(value)
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

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
    }
}
