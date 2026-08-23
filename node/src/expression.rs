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
use yggdryl::expression::{
    Bound as CoreBound, BoundStatement as CoreBoundStatement, Direction, NullsOrder, Operator,
    Selector, Statement as CoreStatement,
};
use yggdryl::{Expression as CoreExpression, Scalar};

use crate::arrow::JsBatchReader;
use crate::codec::JsScalar;
use crate::field::JsField;
use crate::napi_error;

/// The stable binding spelling of one ordering direction.
const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Ascending => "ascending",
        Direction::Descending => "descending",
    }
}

/// The stable binding spelling of an explicit null placement.
const fn nulls_name(nulls: NullsOrder) -> &'static str {
    match nulls {
        NullsOrder::First => "first",
        NullsOrder::Last => "last",
    }
}

/// Read a native Record of late-bound values once before binding.
fn supplied_parameters(parameters: Option<&JsScalar>) -> Result<Vec<(String, Scalar)>> {
    let Some(parameters) = parameters else {
        return Ok(Vec::new());
    };
    let entries = parameters.inner.as_record().ok_or_else(|| {
        Error::from_reason("statement parameters must be a Scalar record keyed by parameter name")
    })?;
    Ok(entries
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect())
}

/// Borrow the core parameter shape for exactly one bind call.
fn parameter_refs(parameters: &[(String, Scalar)]) -> Vec<(&str, Scalar)> {
    parameters
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect()
}

/// Take exactly one batch from the explicit materialized-batch bridge.
fn one_batch(reader: &mut JsBatchReader) -> Result<arrow_array::RecordBatch> {
    let mut reader = reader.take()?;
    let batch = reader
        .next()
        .ok_or_else(|| Error::from_reason("expected one Arrow RecordBatch, got an empty stream"))?
        .map_err(napi_error)?;
    match reader.next() {
        None => Ok(batch),
        Some(Ok(_)) => Err(Error::from_reason(
            "expected one Arrow RecordBatch, got more than one batch",
        )),
        Some(Err(error)) => Err(napi_error(error)),
    }
}

/// One unbound ordering key exposed for inspection.
#[napi(object, object_from_js = false)]
pub struct StatementOrder {
    /// The expression sorted by.
    pub expression: JsExpression,
    /// `ascending` or `descending`.
    #[napi(ts_type = "'ascending' | 'descending'")]
    pub direction: String,
    /// `first`, `last`, or `null` when the statement left the default implicit.
    #[napi(ts_type = "'first' | 'last' | null")]
    pub nulls: Option<String>,
}

/// One schema-resolved ordering key exposed for inspection.
#[napi(object, object_from_js = false)]
pub struct BoundStatementOrder {
    /// The resolved expression sorted by.
    pub expression: JsBound,
    /// `ascending` or `descending`.
    #[napi(ts_type = "'ascending' | 'descending'")]
    pub direction: String,
    /// `first`, `last`, or `null` when the statement left the default implicit.
    #[napi(ts_type = "'first' | 'last' | null")]
    pub nulls: Option<String>,
}

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
    /// The constant is a `Scalar`, which is the JavaScript spelling of the
    /// values JavaScript itself has none of - an exact decimal, a date, a
    /// timestamp at a resolution a `Date` cannot hold. `Scalar.fromJs` makes
    /// one out of an ordinary JavaScript value.
    #[napi(factory)]
    pub fn literal(value: &JsScalar) -> Self {
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

    /// Build `this + other` after the loader has inferred the public input.
    #[napi(js_name = "_addNative", skip_typescript)]
    pub fn add_native(&self, other: &JsExpression) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Add, other.inner.clone()),
        )
    }

    /// Build `this - other` after the loader has inferred the public input.
    #[napi(js_name = "_subtractNative", skip_typescript)]
    pub fn subtract_native(&self, other: &JsExpression) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Sub, other.inner.clone()),
        )
    }

    /// Build `this * other` after the loader has inferred the public input.
    #[napi(js_name = "_multiplyNative", skip_typescript)]
    pub fn multiply_native(&self, other: &JsExpression) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Mul, other.inner.clone()),
        )
    }

    /// Build `this / other` after the loader has inferred the public input.
    #[napi(js_name = "_divideNative", skip_typescript)]
    pub fn divide_native(&self, other: &JsExpression) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Div, other.inner.clone()),
        )
    }

    /// Build `this % other` after the loader has inferred the public input.
    #[napi(js_name = "_remainderNative", skip_typescript)]
    pub fn remainder_native(&self, other: &JsExpression) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Rem, other.inner.clone()),
        )
    }

    /// Build `-this`, folding a numeric literal in the native core.
    #[napi]
    pub fn negate(&self) -> Self {
        Self::from_core(self.inner.clone().neg())
    }

    /// Write this expression as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
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
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two expressions are the same tree.
    #[napi]
    pub fn equals(&self, other: Either<ClassInstance<'_, JsExpression>, String>) -> Result<bool> {
        Ok(self.inner == expression_from_input(other)?)
    }

    /// Compare two expression trees by the core's total structural order.
    #[napi]
    pub fn compare(&self, other: Either<ClassInstance<'_, JsExpression>, String>) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&expression_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical expression text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable expression tree.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
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
    pub fn eval(&self, row: &JsScalar) -> Result<JsScalar> {
        self.inner
            .eval(&row.inner)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Answer this predicate for one row, reading unknown as "no".
    ///
    /// Unknown is not true, so a row whose value is null does not pass a
    /// comparison against it.
    #[napi]
    pub fn matches(&self, row: &JsScalar) -> Result<bool> {
        self.inner.matches(&row.inner).map_err(napi_error)
    }

    /// The canonical text of the expression this resolved.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A projection list, a predicate, an ordering, and a limit.
#[napi(js_name = "Statement")]
pub struct JsStatement {
    inner: CoreStatement,
}

impl Clone for JsStatement {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Read a statement from another native statement or canonical text.
fn statement_from_input(
    value: Either<ClassInstance<'_, JsStatement>, String>,
) -> Result<CoreStatement> {
    match value {
        Either::A(statement) => Ok(statement.inner.clone()),
        Either::B(text) => text.parse().map_err(napi_error),
    }
}

#[napi]
impl JsStatement {
    /// Parse one statement from canonical text, or cheaply clone one.
    #[napi(constructor)]
    pub fn new(value: Either<ClassInstance<'_, JsStatement>, String>) -> Result<Self> {
        Ok(Self {
            inner: statement_from_input(value)?,
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

    /// The ordering keys, in priority order.
    #[napi(getter)]
    pub fn ordering(&self) -> Vec<StatementOrder> {
        self.inner
            .ordering()
            .iter()
            .map(|order| StatementOrder {
                expression: JsExpression::from_core(order.expression().clone()),
                direction: direction_name(order.direction()).to_owned(),
                nulls: order.nulls().map(|nulls| nulls_name(nulls).to_owned()),
            })
            .collect()
    }

    /// The row limit, when the statement had one.
    #[napi(getter)]
    pub fn limit(&self) -> Option<i64> {
        self.inner
            .limit()
            .and_then(|limit| i64::try_from(limit).ok())
    }

    /// Return whether this statement selects every input column unchanged.
    #[napi(getter)]
    pub fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Resolve every statement expression against one struct root schema.
    ///
    /// The loader converts an ordinary JavaScript parameter object into the
    /// shared native `Scalar::Record` before this redirect.
    #[napi(js_name = "_bindNative", skip_typescript)]
    pub fn bind_native(
        &self,
        schema: &JsField,
        parameters: Option<&JsScalar>,
    ) -> Result<JsBoundStatement> {
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(JsBoundStatement {
            inner: self
                .inner
                .bind_with(&schema.inner, &borrowed)
                .map_err(napi_error)?,
        })
    }

    /// Write this statement as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
    }

    /// The canonical text, which re-parses to this statement.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two statements describe the same operation.
    #[napi]
    pub fn equals(&self, other: Either<ClassInstance<'_, JsStatement>, String>) -> Result<bool> {
        Ok(self.inner == statement_from_input(other)?)
    }

    /// Compare two statements by the core's total structural order.
    #[napi]
    pub fn compare(&self, other: Either<ClassInstance<'_, JsStatement>, String>) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&statement_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical statement text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable statement.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// A statement resolved against one schema, ready for batch execution.
#[napi(js_name = "BoundStatement")]
pub struct JsBoundStatement {
    inner: CoreBoundStatement,
}

#[napi]
impl JsBoundStatement {
    /// The struct root the statement reads.
    #[napi(getter)]
    pub fn schema(&self) -> JsField {
        JsField::from_core(self.inner.schema().clone())
    }

    /// The struct root the statement publishes.
    #[napi(getter)]
    pub fn output(&self) -> JsField {
        JsField::from_core(self.inner.output().clone())
    }

    /// The bound projections, in output order. Empty means every column.
    #[napi(getter)]
    pub fn projections(&self) -> Vec<JsBound> {
        self.inner
            .projections()
            .iter()
            .cloned()
            .map(|inner| JsBound { inner })
            .collect()
    }

    /// The bound predicate, when the statement had one.
    #[napi(getter)]
    pub fn predicate(&self) -> Option<JsBound> {
        self.inner
            .predicate()
            .cloned()
            .map(|inner| JsBound { inner })
    }

    /// The bound ordering keys, in priority order.
    #[napi(getter)]
    pub fn ordering(&self) -> Vec<BoundStatementOrder> {
        self.inner
            .ordering()
            .iter()
            .map(|(bound, direction, nulls)| BoundStatementOrder {
                expression: JsBound {
                    inner: bound.clone(),
                },
                direction: direction_name(*direction).to_owned(),
                nulls: nulls.map(|nulls| nulls_name(nulls).to_owned()),
            })
            .collect()
    }

    /// The row limit, when the statement had one.
    #[napi(getter)]
    pub fn limit(&self) -> Option<i64> {
        self.inner
            .limit()
            .and_then(|limit| i64::try_from(limit).ok())
    }

    /// Return whether this statement selects every input column unchanged.
    #[napi(getter)]
    pub fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Lazily filter, project, and limit one native batch reader.
    #[napi(js_name = "_projectArrowReaderNative", skip_typescript)]
    pub fn project_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let projected = self
            .inner
            .clone()
            .project_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(
            projected,
            self.inner.output().name(),
        ))
    }

    /// Filter and project the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_projectArrowRecordBatchNative", skip_typescript)]
    pub fn project_arrow_record_batch_native(
        &self,
        reader: &mut JsBatchReader,
    ) -> Result<JsBatchReader> {
        let projected = self
            .inner
            .project(&one_batch(reader)?)
            .map_err(napi_error)?;
        let schema = projected.schema();
        Ok(JsBatchReader::from_core(
            yggdryl::arrow::batch_reader(schema, [projected]),
            self.inner.output().name(),
        ))
    }

    /// Sort the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_sortArrowRecordBatchNative", skip_typescript)]
    pub fn sort_arrow_record_batch_native(
        &self,
        reader: &mut JsBatchReader,
    ) -> Result<JsBatchReader> {
        let sorted = self.inner.sort(&one_batch(reader)?).map_err(napi_error)?;
        let schema = sorted.schema();
        Ok(JsBatchReader::from_core(
            yggdryl::arrow::batch_reader(schema, [sorted]),
            self.inner.schema().name(),
        ))
    }
}
