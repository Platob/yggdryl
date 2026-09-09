//! Native configuration values and their lazy message iterators.

use napi::bindgen_prelude::{Buffer, Generator, Result};
use napi_derive::napi;
use yggdryl::{FixMessages, Ulconfig, Ulconfigs};

use super::{JsFixCodec, JsFixMsg};
use crate::napi_error;
use crate::text::codec::JsScalar;

/// One selected configuration; its shared source response remains native.
#[napi(js_name = "Ulconfig")]
pub struct JsUlconfig {
    inner: Ulconfig,
}

#[napi]
impl JsUlconfig {
    /// Construct from a selected `ObjectName`, attributes, and source response.
    #[napi(
        constructor,
        ts_args_type = "mbean: string | null, attributes: Scalar, envelope: Scalar"
    )]
    pub fn new(mbean: Option<String>, attributes: &JsScalar, envelope: &JsScalar) -> Self {
        Self {
            inner: Ulconfig::new(
                mbean.as_deref(),
                attributes.inner.clone(),
                envelope.inner.clone(),
            ),
        }
    }

    /// Parse and validate a response before returning its lazy configurations.
    #[napi]
    pub fn from_json_bytes(body: Buffer) -> Result<JsUlconfigs> {
        Ulconfig::from_json_bytes(&body)
            .map(|inner| JsUlconfigs { inner })
            .map_err(napi_error)
    }

    /// Validate a native response and iterate its selected configurations.
    #[napi]
    pub fn from_json_scalar(document: &JsScalar) -> Result<JsUlconfigs> {
        Ulconfig::from_json_scalar(&document.inner)
            .map(|inner| JsUlconfigs { inner })
            .map_err(napi_error)
    }

    /// Recover one configuration from a flat native message.
    #[napi(factory)]
    pub fn from_fixmsg(message: &JsFixMsg) -> Result<Self> {
        Ulconfig::from_fixmsg(&message.inner)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// Convert this selected configuration to one flat native message.
    #[napi]
    pub fn into_fixmsg(&self, codec: &JsFixCodec, enrich: Option<bool>) -> Result<JsFixMsg> {
        self.inner
            .into_fixmsg(&codec.inner, enrich.unwrap_or(false))
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// The selected actual `ObjectName`, when present.
    #[napi(getter)]
    pub fn mbean(&self) -> Option<String> {
        self.inner.mbean().map(ToOwned::to_owned)
    }
    /// The type property of the selected `ObjectName`.
    #[napi(getter)]
    pub fn mbean_type(&self) -> Option<String> {
        self.inner.mbean_type().map(ToOwned::to_owned)
    }
    /// The declared plugin type.
    #[napi(getter)]
    pub fn plugin_type(&self) -> Option<String> {
        self.inner.plugin_type().map(ToOwned::to_owned)
    }
    /// The native canonical name.
    #[napi(getter)]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(ToOwned::to_owned)
    }
    /// The declared configuration version.
    #[napi(getter)]
    pub fn version(&self) -> Option<String> {
        self.inner.version().map(ToOwned::to_owned)
    }
    /// The declared configuration category.
    #[napi(getter)]
    pub fn category(&self) -> Option<String> {
        self.inner.category().map(ToOwned::to_owned)
    }
    /// The declared configuration state.
    #[napi(getter)]
    pub fn state(&self) -> Option<String> {
        self.inner.state().map(ToOwned::to_owned)
    }

    /// Look up one native configuration attribute.
    #[napi]
    pub fn get(&self, name: String) -> Option<JsScalar> {
        self.inner.get(&name).cloned().map(JsScalar::from_core)
    }

    /// Share the selected native attribute value.
    #[napi]
    pub fn as_attributes(&self) -> JsScalar {
        JsScalar::from_core(self.inner.as_attributes().clone())
    }

    /// Shares the complete source response, which may contain sibling values.
    #[napi]
    pub fn as_envelope(&self) -> JsScalar {
        JsScalar::from_core(self.inner.as_envelope().clone())
    }

    /// Compare the complete native values.
    #[napi]
    pub fn equals(&self, other: &JsUlconfig) -> bool {
        self.inner == other.inner
    }

    /// Deterministic hash bits from the native value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Clone the native value, retaining shared backing.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// A lazy iterator of validated native configurations.
#[napi(iterator, js_name = "Ulconfigs")]
pub struct JsUlconfigs {
    inner: Ulconfigs,
}

impl Generator for JsUlconfigs {
    type Yield = JsUlconfig;
    type Next = ();
    type Return = ();
    fn next(&mut self, _: Option<Self::Next>) -> Option<Self::Yield> {
        self.inner.next().map(|inner| JsUlconfig { inner })
    }
}

/// A native fallible cursor; the JavaScript loader supplies `Symbol.iterator`.
#[napi(js_name = "FixMessages")]
pub struct JsFixMessages {
    inner: FixMessages,
}

impl JsFixMessages {
    pub(super) const fn from_core(inner: FixMessages) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsFixMessages {
    /// Advance the native cursor, propagating an error or returning null at its end.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<FixMsg>")]
    pub fn next(&mut self) -> Result<Option<JsFixMsg>> {
        self.inner
            .next()
            .transpose()
            .map(|value| value.map(JsFixMsg::from_core))
            .map_err(napi_error)
    }
}
