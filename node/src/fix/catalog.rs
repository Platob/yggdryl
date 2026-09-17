//! The immutable message definition a registry answers.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::MsgType;

use super::JsFixMsg;
use crate::text::codec::JsScalar;
use crate::types::field::JsField;
use crate::{exact_i32, napi_error, ordering_value};

/// An immutable message definition: a copy of the registry's own, so a
/// later mutation of the dictionary leaves it as it was answered.
#[napi(js_name = "MsgType")]
pub struct JsMsgType {
    inner: MsgType,
}

impl JsMsgType {
    pub(super) const fn from_core(inner: MsgType) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsMsgType {
    /// The native canonical name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// The complete wire message code.
    #[napi]
    pub fn as_str(&self) -> String {
        self.inner.as_str().to_owned()
    }

    /// Project an independent copy of the native message Struct field.
    #[napi]
    pub fn as_field(&self) -> JsField {
        JsField::from_core(self.inner.as_field().clone())
    }

    /// The compiled selection of non-null direct identifiers, in member order.
    ///
    /// Fields are independent mutable declaration copies; Scalars retain the
    /// message's actual types. Repeating groups are not traversed.
    #[napi(
        ts_args_type = "message: FixMsg",
        ts_return_type = "Array<[Field, Scalar]>"
    )]
    pub fn identifier_values(&self, message: &JsFixMsg) -> Vec<(JsField, JsScalar)> {
        self.inner
            .identifier_values(message.as_core())
            .map(|(field, value)| {
                (
                    JsField::from_core(field.clone()),
                    JsScalar::from_core(value.clone()),
                )
            })
            .collect()
    }

    /// Look up the unique repeating group for a native counter tag.
    #[napi]
    pub fn get_group_by_tag(&self, tag: f64) -> Result<Option<JsField>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self
            .inner
            .get_group_by_tag(tag)
            .cloned()
            .map(JsField::from_core))
    }

    /// Compare the complete native values.
    #[napi]
    pub fn equals(&self, other: &JsMsgType) -> bool {
        self.inner == other.inner
    }

    /// Compare native message definitions using their total ordering.
    #[napi]
    pub fn compare(&self, other: &JsMsgType) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
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

    /// Render the canonical native text.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.as_str().to_owned()
    }

    /// Project the native JSON representation.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(self.inner.as_field()).map_err(napi_error)
    }
}
