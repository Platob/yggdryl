//! Native configuration values and their lazy iterator.

use napi::bindgen_prelude::{Buffer, Generator, Result};
use napi_derive::napi;
use yggdryl::{Plugin, Plugins};

use super::{JsFixCodec, JsFixMsg};
use crate::napi_error;
use crate::text::codec::JsScalar;

/// One configuration: the `ObjectName` naming it and the attributes it states.
#[napi(js_name = "Plugin")]
pub struct JsPlugin {
    inner: Plugin,
}

#[napi]
impl JsPlugin {
    /// Construct from the parts a document states: the selected `ObjectName`
    /// and the attributes.
    ///
    /// What the Jolokia exchange wrapped them in is the transport's and no
    /// part of the configuration.
    #[napi(constructor, ts_args_type = "mbean: string | null, attributes: Scalar")]
    pub fn new(mbean: Option<String>, attributes: &JsScalar) -> Self {
        Self {
            inner: Plugin::new(mbean.as_deref(), attributes.inner.clone()),
        }
    }

    /// Every configuration a body names, lazily.
    ///
    /// Bytes that are not a Jolokia answer name none, and naming none is what
    /// they answer: reading is not refusing, so bytes that are not JSON at all
    /// iterate empty rather than throwing.
    #[napi]
    pub fn from_json_bytes(body: Buffer) -> JsPlugins {
        JsPlugins {
            inner: Plugin::from_json_bytes(&body),
        }
    }

    /// The same, over a document a caller already parsed.
    ///
    /// A document that is not a Jolokia answer, an answer that came back
    /// empty and an error-only answer all name no configuration, which is
    /// what they answer.
    #[napi]
    pub fn from_json_scalar(document: &JsScalar) -> JsPlugins {
        JsPlugins {
            inner: Plugin::from_json_scalar(&document.inner),
        }
    }

    /// Recover one configuration from a flat native message.
    #[napi(factory)]
    pub fn from_fixmsg(message: &JsFixMsg) -> Result<Self> {
        Plugin::from_fixmsg(&message.inner)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// This plugin as a message typed against `codec`'s dictionary.
    ///
    /// The same build every other reader funnels into, so a dictionary
    /// carrying the plugin fields types a port as a number and a flag as a
    /// boolean, and one that does not keeps every attribute as the text it
    /// arrived as.
    #[napi]
    pub fn into_fixmsg(&self, codec: &JsFixCodec) -> Result<JsFixMsg> {
        self.inner
            .into_fixmsg(&codec.inner)
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

    /// Two configurations are equal with the same `ObjectName` and attributes.
    #[napi]
    pub fn equals(&self, other: &JsPlugin) -> bool {
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

/// A lazy iterator over the configurations a document names.
#[napi(iterator, js_name = "Plugins")]
pub struct JsPlugins {
    inner: Plugins,
}

impl Generator for JsPlugins {
    type Yield = JsPlugin;
    type Next = ();
    type Return = ();
    fn next(&mut self, _: Option<Self::Next>) -> Option<Self::Yield> {
        self.inner.next().map(|inner| JsPlugin { inner })
    }
}
