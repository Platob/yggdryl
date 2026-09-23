//! JavaScript's native view of the core [`ArrowCastPlan`]: the cast from one
//! field's layout to another, compiled once and applied to every column of
//! that layout.
//!
//! [`JsArrowCastPlan`] owns only the core plan and redirects every verb to it.
//! The loader publishes `compile` and `apply` over the private natives here,
//! because a source may arrive as an Apache Arrow JS schema and a serie is
//! handed back as its leaf's class.

use std::sync::Arc;

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use serde_json::{Value as JsonValue, json};
use yggdryl::{ArrowCastPlan, Field as CoreField};

use crate::field::JsField;
use crate::napi_error;
use crate::serie::JsSerie;

/// The cast from one field's layout to another, compiled once.
///
/// Every failure the two fields alone can produce is raised by `compile`,
/// before a row exists; `apply` then differs per column only in the rows it
/// reads.
#[napi(js_name = "ArrowCastPlan")]
pub struct JsArrowCastPlan {
    inner: ArrowCastPlan,
}

#[napi]
impl JsArrowCastPlan {
    /// Compile the cast from `source` to `target` under the three cast
    /// answers.
    #[napi(factory, js_name = "_compileNative", skip_typescript)]
    pub fn compile(
        source: &JsField,
        target: &JsField,
        safe: Option<bool>,
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, nullability.as_deref(), representation.as_deref())?;
        ArrowCastPlan::compile(&source.inner, &target.inner, options)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// Cast one column laid out as `source`.
    #[napi(js_name = "_applyNative", skip_typescript)]
    pub fn apply(&self, serie: &JsSerie) -> Result<JsSerie> {
        self.inner
            .apply(&serie.inner)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// Run the compiled cast over no rows, refusing an impossible cast now
    /// rather than on the first column.
    #[napi]
    pub fn preflight(&self) -> Result<()> {
        self.inner.preflight().map_err(napi_error)
    }

    /// The field a column must lay out as: its storage and its extension
    /// identity.
    #[napi(getter)]
    pub fn source(&self) -> Result<JsField> {
        CoreField::from_arrow_field_ref(Arc::clone(self.inner.as_source()))
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// The field every cast lands under.
    #[napi(getter)]
    pub fn target(&self) -> JsField {
        JsField::from_core(self.inner.as_target().clone())
    }

    /// The three cast answers this plan was compiled under, each spelled.
    #[napi(getter, skip_typescript)]
    pub fn options(&self) -> JsonValue {
        let options = self.inner.as_options();
        json!({
            "safe": options.is_safe(),
            "nullability": options.nullability().as_str(),
            "representation": options.representation().as_str(),
        })
    }

    /// Whether the plan hands every column of its source layout straight
    /// back.
    #[napi(getter)]
    pub fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }
}
