//! Category CRUD and immutable registry-owned message views.

use std::sync::Arc;

use napi::bindgen_prelude::{Generator, Result};
use napi_derive::napi;
use yggdryl::{FixCategory, FixRegistry, MsgType};

use crate::types::field::JsField;
use crate::{exact_i32, napi_error, ordering_value};

/// Lazy category definitions with a retained native registry.
#[napi(iterator, js_name = "FixDefinitionIterator")]
pub struct JsFixDefinitionIterator {
    pub(super) registry: Option<Arc<FixRegistry>>,
    pub(super) category: FixCategory,
    pub(super) index: usize,
}

impl Generator for JsFixDefinitionIterator {
    type Yield = JsField;
    type Next = ();
    type Return = ();

    fn next(&mut self, _: Option<Self::Next>) -> Option<Self::Yield> {
        let field = self
            .registry
            .as_ref()?
            .definition_at(self.category, self.index)
            .cloned();
        if field.is_some() {
            self.index += 1;
        } else {
            self.registry = None;
        }
        field.map(JsField::from_core)
    }

    fn complete(&mut self, _: Option<Self::Return>) -> Option<Self::Yield> {
        self.registry = None;
        None
    }
}

/// An immutable singleton view; retaining its registry keeps its index stable.
#[napi(js_name = "MsgType")]
pub struct JsMsgType {
    registry: Arc<FixRegistry>,
    pub(super) index: usize,
}

impl JsMsgType {
    pub(super) fn from_borrowed(registry: &Arc<FixRegistry>, message: &MsgType) -> Self {
        let index = registry
            .msgtypes()
            .position(|held| std::ptr::eq(held, message))
            .expect("a borrowed message belongs to its registry");
        Self {
            registry: Arc::clone(registry),
            index,
        }
    }

    fn inner(&self) -> &MsgType {
        self.registry
            .msgtype_at(self.index)
            .expect("a held registry preserves its message positions")
    }
}

#[napi]
impl JsMsgType {
    /// The native canonical name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner().name().to_owned()
    }

    /// The complete wire message code.
    #[napi]
    pub fn as_str(&self) -> String {
        self.inner().as_str().to_owned()
    }

    /// Project an independent copy of the native message Struct field.
    #[napi]
    pub fn as_field(&self) -> JsField {
        JsField::from_core(self.inner().as_field().clone())
    }

    /// Look up the unique repeating group for a native counter tag.
    #[napi]
    pub fn get_group_by_counter(&self, tag: f64) -> Result<Option<JsField>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self
            .inner()
            .get_group_by_counter(tag)
            .cloned()
            .map(JsField::from_core))
    }

    /// Compare the complete native values.
    #[napi]
    pub fn equals(&self, other: &JsMsgType) -> bool {
        self.inner() == other.inner()
    }

    /// Compare native message definitions using their total ordering.
    #[napi]
    pub fn compare(&self, other: &JsMsgType) -> i32 {
        ordering_value(self.inner().cmp(other.inner()))
    }

    /// Deterministic hash bits from the native value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner().stable_hash()
    }

    /// Clone the native value, retaining shared backing.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            index: self.index,
        }
    }

    /// Render the canonical native text.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner().as_str().to_owned()
    }

    /// Project the native JSON representation.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(self.inner().as_field()).map_err(napi_error)
    }
}

/// Lazy immutable message singletons with a retained native registry.
#[napi(iterator, js_name = "MsgTypeIterator")]
pub struct JsMsgTypeIterator {
    pub(super) registry: Option<Arc<FixRegistry>>,
    pub(super) index: usize,
}

impl Generator for JsMsgTypeIterator {
    type Yield = JsMsgType;
    type Next = ();
    type Return = ();

    fn next(&mut self, _: Option<Self::Next>) -> Option<Self::Yield> {
        let registry = self.registry.as_ref()?;
        if registry.msgtype_at(self.index).is_none() {
            self.registry = None;
            return None;
        }
        let result = JsMsgType {
            registry: Arc::clone(registry),
            index: self.index,
        };
        self.index += 1;
        Some(result)
    }

    fn complete(&mut self, _: Option<Self::Return>) -> Option<Self::Yield> {
        self.registry = None;
        None
    }
}
