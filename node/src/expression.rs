//! Node.js view of the filter, which is the same tree in every language.
//!
//! One class, `Expression`, carrying the tree the Rust crate carries, and
//! `Bound`, the compiled form. The text is the same text: a predicate written
//! here parses through the same grammar a Rust reader parses it with, so it
//! can cross a wire as a string and mean the same thing on the other side.
//!
//! Wherever a filter is accepted, a `string` is accepted too and *parses*. It
//! is never taken as a string literal, because a filter that silently matched
//! everything would be the worst failure this layer could have.

use napi::bindgen_prelude::{ClassInstance, Either, Error, Result};
use napi_derive::napi;
use yggdryl::Expression as CoreExpression;
use yggdryl::expression::{Bound as CoreBound, Selector, Statement as CoreStatement};

use crate::codec::JsCodecValue;
use crate::field::JsField;
use crate::napi_error;

/// Read a filter from an `Expression` or from text that parses as one.
///
/// The type is written out rather than aliased because NAPI generates
/// TypeScript from the signature it sees, and an alias it cannot follow would
/// leave `Expression | string` unspellable on the JavaScript side.
pub(crate) fn expression_from_input(
    value: Either<ClassInstance<'_, JsExpression>, String>,
) -> Result<CoreExpression> {
    match value {
        Either::A(expression) => Ok(expression.inner.clone()),
        Either::B(text) => text.parse().map_err(napi_error),
    }
}

/// One recursive, typed filter and projection tree.
#[napi(js_name = "Expression")]
pub struct JsExpression {
    pub(crate) inner: CoreExpression,
}

impl Clone for JsExpression {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsExpression {
    pub(crate) const fn from_core(inner: CoreExpression) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsExpression {
    /// Parse one expression from its canonical text, or clone one.
    #[napi(constructor)]
    pub fn new(value: Either<ClassInstance<'_, JsExpression>, String>) -> Result<Self> {
        Ok(Self {
            inner: expression_from_input(value)?,
        })
    }

    /// Parse one expression from its canonical text.
    #[napi(factory)]
    pub fn parse(text: String) -> Result<Self> {
        Ok(Self {
            inner: text.parse().map_err(napi_error)?,
        })
    }

    /// Read one expression from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self {
            inner: CoreExpression::from_json(&document).map_err(napi_error)?,
        })
    }

    /// Name one top-level column.
    #[napi(factory)]
    pub fn column(name: String) -> Self {
        Self {
            inner: CoreExpression::column(name),
        }
    }

    /// Hold one constant.
    ///
    /// The constant is a `Value`, which is the JavaScript spelling of the
    /// values JavaScript itself has none of - an exact decimal, a date, a
    /// timestamp at a resolution a `Date` cannot hold. `Value.fromJs` makes
    /// one out of an ordinary JavaScript value.
    #[napi(factory)]
    pub fn literal(value: &JsCodecValue) -> Self {
        Self {
            inner: CoreExpression::literal(value.inner.clone()),
        }
    }

    /// Name one holder attribute, such as `size`, or `partition` with a column.
    #[napi(factory)]
    pub fn attribute(name: String, key: Option<String>) -> Result<Self> {
        let selector = match key {
            Some(key) if name.eq_ignore_ascii_case("partition") => {
                Selector::Partition(key.as_str().into())
            }
            Some(_) => {
                return Err(Error::from_reason(
                    "expected a key only for the partition attribute",
                ));
            }
            None => Selector::from_name(&name).ok_or_else(|| {
                Error::from_reason(format!(
                    "expected one of the holder attributes {}, got {name:?}",
                    Selector::vocabulary()
                ))
            })?,
        };
        Ok(Self {
            inner: CoreExpression::attribute(selector),
        })
    }

    /// Name one late-bound value.
    #[napi(factory)]
    pub fn parameter(name: String) -> Self {
        Self {
            inner: CoreExpression::parameter(name),
        }
    }

    /// The expression that is true for every row.
    #[napi(factory)]
    pub fn always_true() -> Self {
        Self {
            inner: CoreExpression::always_true(),
        }
    }

    /// The expression that is true for no row.
    #[napi(factory)]
    pub fn always_false() -> Self {
        Self {
            inner: CoreExpression::always_false(),
        }
    }

    /// Every top-level column this expression reads, in first-seen order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this expression reads, in first-seen order.
    #[napi(getter)]
    pub fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this expression names, in first-seen order.
    #[napi(getter)]
    pub fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// The top-level `and` operands, flattened.
    #[napi]
    pub fn conjuncts(&self) -> Vec<JsExpression> {
        self.inner
            .conjuncts()
            .into_iter()
            .map(JsExpression::from_core)
            .collect()
    }

    /// How deep this expression nests, counting itself as one level.
    #[napi(getter)]
    pub fn depth(&self) -> u32 {
        u32::try_from(self.inner.depth()).unwrap_or(u32::MAX)
    }

    /// Build `this and other`.
    #[napi]
    pub fn and(&self, other: Either<ClassInstance<'_, JsExpression>, String>) -> Result<Self> {
        Ok(Self {
            inner: self.inner.clone().and(expression_from_input(other)?),
        })
    }

    /// Build `this or other`.
    #[napi]
    pub fn or(&self, other: Either<ClassInstance<'_, JsExpression>, String>) -> Result<Self> {
        Ok(Self {
            inner: self.inner.clone().or(expression_from_input(other)?),
        })
    }

    /// Build `not this`.
    #[napi]
    pub fn not(&self) -> Self {
        Self {
            inner: self.inner.clone().not(),
        }
    }

    /// Write this expression as a structural JSON document.
    #[napi]
    pub fn to_json(&self) -> Result<String> {
        self.inner.to_json().map_err(napi_error)
    }

    /// Resolve this expression against a struct root schema.
    #[napi]
    pub fn bind(&self, schema: &JsField) -> Result<JsBound> {
        Ok(JsBound {
            inner: self.inner.bind(&schema.inner).map_err(napi_error)?,
        })
    }

    /// The output field this expression produces against a schema.
    #[napi]
    pub fn field(&self, schema: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.field(&schema.inner).map_err(napi_error)?,
        ))
    }

    /// The canonical text, which re-parses to this expression.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two expressions are the same tree.
    #[napi]
    pub fn equals(&self, other: Either<ClassInstance<'_, JsExpression>, String>) -> Result<bool> {
        Ok(self.inner == expression_from_input(other)?)
    }
}

/// One expression resolved against one schema, ready to answer.
#[napi(js_name = "Bound")]
pub struct JsBound {
    inner: CoreBound,
}

#[napi]
impl JsBound {
    /// The expression as it stands after substitution, folding, and ordering.
    #[napi(getter)]
    pub fn expression(&self) -> JsExpression {
        JsExpression::from_core(self.inner.expression().clone())
    }

    /// The output field this expression produces.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.field().clone())
    }

    /// Return whether this expression answers a boolean.
    #[napi(getter)]
    pub fn is_predicate(&self) -> bool {
        self.inner.is_predicate()
    }

    /// The schema column names this expression reads, in index order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.column_names()
    }

    /// Return whether answering this expression requires reading rows.
    #[napi(getter)]
    pub fn reads_rows(&self) -> bool {
        self.inner.reads_rows()
    }

    /// Evaluate this expression for one row of column values, in schema order.
    #[napi]
    pub fn eval(&self, row: &JsCodecValue) -> Result<JsCodecValue> {
        self.inner
            .eval(&row.inner)
            .map(JsCodecValue::from_core)
            .map_err(napi_error)
    }

    /// Answer this predicate for one row, reading unknown as "no".
    ///
    /// Unknown is not true, so a row whose value is null does not pass a
    /// comparison against it.
    #[napi]
    pub fn matches(&self, row: &JsCodecValue) -> Result<bool> {
        self.inner.matches(&row.inner).map_err(napi_error)
    }

    /// The canonical text of the expression this resolved.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A projection list, a predicate, an ordering, and a limit.
#[napi(js_name = "Statement")]
pub struct JsStatement {
    inner: CoreStatement,
}

#[napi]
impl JsStatement {
    /// Parse one statement from its canonical text.
    #[napi(constructor)]
    pub fn new(text: String) -> Result<Self> {
        Ok(Self {
            inner: text.parse().map_err(napi_error)?,
        })
    }

    /// Read one statement from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self {
            inner: CoreStatement::from_json(&document).map_err(napi_error)?,
        })
    }

    /// The names this statement publishes, in output order. Empty means `*`.
    #[napi(getter)]
    pub fn projections(&self) -> Vec<String> {
        self.inner
            .projections()
            .iter()
            .map(|projection| projection.name().to_string())
            .collect()
    }

    /// The predicate, when the statement had a `where`.
    #[napi(getter)]
    pub fn predicate(&self) -> Option<JsExpression> {
        self.inner.predicate().cloned().map(JsExpression::from_core)
    }

    /// The row limit, when the statement had one.
    #[napi(getter)]
    pub fn limit(&self) -> Option<i64> {
        self.inner
            .limit()
            .and_then(|limit| i64::try_from(limit).ok())
    }

    /// Write this statement as a structural JSON document.
    #[napi]
    pub fn to_json(&self) -> Result<String> {
        self.inner.to_json().map_err(napi_error)
    }

    /// The canonical text, which re-parses to this statement.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }
}
