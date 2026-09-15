//! Node.js view of expressions, which are the same trees in every language.
//!
//! The layers the Rust crate has, one class each: `Term` is the tree a
//! `where` or a projection is built from, `Bound` a term compiled against one
//! schema, `Filter` and `Selector` the two clauses, `Plan` the sections of one
//! read or write, and `Expression` whichever of those a piece of text turns
//! out to be - a clause, a plan, or a `;`-separated sequence. `Records`
//! streams native rows through any of them.
//!
//! The text is the same text: a predicate written here parses through the
//! same grammar a Rust reader parses it with, so it can cross a wire as a
//! string and mean the same thing on the other side. Wherever a clause is
//! accepted, a `string` is accepted too and *parses*. It is never taken as a
//! string literal, because a filter that silently matched everything would be
//! the worst failure this layer could have.

use napi::bindgen_prelude::{ClassInstance, Either, Either3, Either4, Either5, Error, Result};
use napi_derive::napi;
use yggdryl::expression::{
    Attribute, Bound as CoreBound, BoundSelector as CoreBoundSelector,
    Comparison as CoreComparison, FieldSegment as CoreSegment, Function as CoreFunction, IntoPlan,
    Operator, Ordering as CoreOrdering, Plan as CorePlan, Projection as CoreProjection,
    Records as CoreRecords, Source as CoreSource, Target as CoreTarget, Term as CoreTerm,
    Verb as CoreVerb, Write as CoreWrite,
};
use yggdryl::{
    Expression as CoreExpression, Field as CoreField, Filter as CoreFilter, Scalar,
    Selector as CoreSelector,
};

use crate::iomedia::JsBatchReader;
use crate::napi_error;
use crate::text::codec::JsScalar;
use crate::types::datatype::{DataTypeInput, dtype_from_input};
use crate::types::field::{JsField, MetadataEntry};

// ---------------------------------------------------------------------------
// Reading JavaScript values as the core's
// ---------------------------------------------------------------------------

// The input aliases below are for the readers in this module. A `#[napi]`
// signature spells the union out instead, because NAPI generates TypeScript
// from the tokens it sees and an alias it cannot follow would leave the union
// unspellable on the JavaScript side.

/// A term, or the text of one.
pub(crate) type TermInput<'a> = Either<ClassInstance<'a, JsTerm>, String>;
/// A filter, a term, or the text of a predicate.
pub(crate) type FilterInput<'a> =
    Either3<ClassInstance<'a, JsFilter>, ClassInstance<'a, JsTerm>, String>;
/// A selector, a term, the text of a projection list, or projection texts.
pub(crate) type SelectorInput<'a> = Either4<
    ClassInstance<'a, JsSelector>,
    ClassInstance<'a, JsTerm>,
    String,
    Vec<Either<ClassInstance<'a, JsTerm>, String>>,
>;
/// A plan, a clause, a field, or the text of a plan.
pub(crate) type PlanInput<'a> = Either5<
    ClassInstance<'a, JsPlan>,
    ClassInstance<'a, JsSelector>,
    ClassInstance<'a, JsFilter>,
    ClassInstance<'a, JsField>,
    String,
>;
/// An expression, a plan, a clause, or the text of one.
pub(crate) type ExpressionInput<'a> = Either5<
    ClassInstance<'a, JsExpression>,
    ClassInstance<'a, JsPlan>,
    ClassInstance<'a, JsSelector>,
    ClassInstance<'a, JsFilter>,
    String,
>;

/// Read a term from a `Term` or from text that parses as one.
pub(crate) fn term_from_input(value: TermInput<'_>) -> Result<CoreTerm> {
    match value {
        Either::A(term) => Ok(term.inner.clone()),
        Either::B(text) => text.parse().map_err(napi_error),
    }
}

/// Read a filter from a `Filter`, a `Term`, or the text of a predicate.
pub(crate) fn filter_from_input(value: FilterInput<'_>) -> Result<CoreFilter> {
    match value {
        Either3::A(filter) => Ok(filter.inner.clone()),
        Either3::B(term) => Ok(CoreFilter::new(term.inner.clone())),
        Either3::C(text) => text.parse().map_err(napi_error),
    }
}

/// Read a selector from a `Selector`, a `Term`, text, or projection texts.
pub(crate) fn selector_from_input(value: SelectorInput<'_>) -> Result<CoreSelector> {
    match value {
        Either4::A(selector) => Ok(selector.inner.clone()),
        Either4::B(term) => Ok(CoreSelector::from(CoreProjection::new(term.inner.clone()))),
        Either4::C(text) => text.parse().map_err(napi_error),
        Either4::D(projections) => {
            let mut selected = Vec::with_capacity(projections.len());
            for projection in projections {
                selected.push(match projection {
                    Either::A(term) => CoreProjection::new(term.inner.clone()),
                    Either::B(text) => text.parse::<CoreProjection>().map_err(napi_error)?,
                });
            }
            Ok(CoreSelector::new(selected))
        }
    }
}

/// Read a plan from a `Plan`, a clause, a `Field`, or the text of a plan.
pub(crate) fn plan_from_input(value: PlanInput<'_>) -> Result<CorePlan> {
    match value {
        Either5::A(plan) => Ok(plan.inner.clone()),
        Either5::B(selector) => Ok(CorePlan::from(selector.inner.clone())),
        Either5::C(filter) => Ok(CorePlan::from(filter.inner.clone())),
        Either5::D(field) => Ok(CorePlan::from_field(&field.inner)),
        Either5::E(text) => text.as_str().into_plan().map_err(napi_error),
    }
}

/// Read an expression from an `Expression`, a plan, a clause, or text.
pub(crate) fn expression_from_input(value: ExpressionInput<'_>) -> Result<CoreExpression> {
    match value {
        Either5::A(expression) => Ok(expression.inner.clone()),
        Either5::B(plan) => Ok(plan.inner.clone().into_expression()),
        Either5::C(selector) => Ok(CoreExpression::Selector(selector.inner.clone())),
        Either5::D(filter) => Ok(CoreExpression::Filter(filter.inner.clone())),
        Either5::E(text) => text.parse().map_err(napi_error),
    }
}

/// Read one target from the text a location is spelled as.
fn target_from_text(text: &str) -> Result<CoreTarget> {
    CoreTarget::parse(text).map_err(napi_error)
}

/// Read one ordering key spelled as the grammar spells it: a term, then an
/// optional `asc` or `desc`, then an optional `nulls first` or `nulls last`.
fn ordering_from_text(text: &str) -> Result<CoreOrdering> {
    let plan: CorePlan = format!("order by {text}").parse().map_err(napi_error)?;
    let mut keys = plan.ordering().to_vec();
    match keys.len() {
        1 => Ok(keys.swap_remove(0)),
        _ => Err(Error::from_reason(format!(
            "expected one ordering key, got {text:?}"
        ))),
    }
}

/// Read a write verb in any spelling the grammar reads.
fn verb_from_text(value: &str) -> Result<CoreVerb> {
    let spelling = value
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(match spelling.as_str() {
        "insert" | "insert into" | "append" | "append into" | "append to" => CoreVerb::Insert,
        "insert overwrite"
        | "insert overwrite into"
        | "overwrite"
        | "overwrite into"
        | "replace"
        | "replace into" => CoreVerb::Overwrite,
        "upsert" | "upsert into" | "merge" | "merge into" => CoreVerb::Upsert,
        "delete" | "delete from" => CoreVerb::Delete,
        _ => {
            return Err(Error::from_reason(format!(
                "unknown write verb {value:?}; expected \"insert into\", \"insert overwrite\", \
                 \"upsert into\", \"delete from\", or one of their aliases"
            )));
        }
    })
}

/// Read one comparison from the grammar's own spelling of it.
fn comparison_from_text(value: &str) -> Result<CoreComparison> {
    CoreComparison::ALL
        .into_iter()
        .find(|comparison| comparison.as_str().eq_ignore_ascii_case(value))
        .ok_or_else(|| {
            Error::from_reason(format!(
                "unknown comparison {value:?}; expected one of {}",
                CoreComparison::ALL.map(CoreComparison::as_str).join(", ")
            ))
        })
}

/// Read one holder attribute, `partition` taking the column it reads.
fn attribute_from_name(name: &str, key: Option<&str>) -> Result<Attribute> {
    match key {
        Some(key) if name.eq_ignore_ascii_case("partition") => Ok(Attribute::Partition(key.into())),
        Some(_) => Err(Error::from_reason(
            "expected a key only for the partition attribute",
        )),
        None => Attribute::from_name(name).ok_or_else(|| {
            Error::from_reason(format!(
                "expected one of the holder attributes {}, got {name:?}",
                Attribute::vocabulary()
            ))
        }),
    }
}

/// Read a native Record of late-bound values once before binding.
fn supplied_parameters(parameters: Option<&JsScalar>) -> Result<Vec<(String, Scalar)>> {
    let Some(parameters) = parameters else {
        return Ok(Vec::new());
    };
    let entries = parameters.inner.as_record().ok_or_else(|| {
        Error::from_reason("parameters must be a Scalar record keyed by parameter name")
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

/// Read native rows out of one Scalar sequence: a record is taken as is, and
/// a mapping keyed by text becomes the record it names.
fn rows_from_scalar(rows: &JsScalar) -> Result<Vec<Scalar>> {
    let held = rows.inner.as_sequence().ok_or_else(|| {
        Error::from_reason("rows must be a Scalar sequence of records, one per row")
    })?;
    let mut records = Vec::with_capacity(held.len());
    for row in held {
        if let Some(entries) = row.as_mapping() {
            let mut named = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                let key = key.as_str().ok_or_else(|| {
                    Error::from_reason("a row mapping must be keyed by column name")
                })?;
                named.push((key.to_owned(), value.clone()));
            }
            records.push(Scalar::from_record(named).map_err(napi_error)?);
        } else {
            records.push(row.clone());
        }
    }
    Ok(records)
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

/// One batch as the reader it is, under the root name the bridge carries.
fn batch_reader(batch: arrow_array::RecordBatch, root_name: &str) -> JsBatchReader {
    let schema = batch.schema();
    JsBatchReader::from_core(yggdryl::arrow::batch_reader(schema, [batch]), root_name)
}

// ---------------------------------------------------------------------------
// Term
// ---------------------------------------------------------------------------

/// One recursive, typed tree: a column, a constant, a comparison, a function.
#[napi(js_name = "Term")]
pub struct JsTerm {
    pub(crate) inner: CoreTerm,
}

impl Clone for JsTerm {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsTerm {
    pub(crate) const fn from_core(inner: CoreTerm) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTerm {
    /// Parse one term from its canonical text, or clone one.
    #[napi(constructor)]
    pub fn new(value: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(term_from_input(value)?))
    }

    /// Parse one term from its canonical text.
    #[napi(factory)]
    pub fn parse(text: String) -> Result<Self> {
        Ok(Self::from_core(text.parse().map_err(napi_error)?))
    }

    /// Read one term from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self::from_core(
            CoreTerm::from_json(&document).map_err(napi_error)?,
        ))
    }

    /// Name one top-level column.
    #[napi(factory)]
    pub fn column(name: String) -> Self {
        Self::from_core(CoreTerm::column(name))
    }

    /// Hold one constant.
    ///
    /// The constant is a `Scalar`, which is the JavaScript spelling of the
    /// values JavaScript itself has none of - an exact decimal, a date, a
    /// timestamp at a resolution a `Date` cannot hold. `Scalar.fromJs` makes
    /// one out of an ordinary JavaScript value.
    #[napi(factory)]
    pub fn literal(value: &JsScalar) -> Self {
        Self::from_core(CoreTerm::literal(value.inner.clone()))
    }

    /// Hold a constant in an explicitly named datatype, checked against it.
    #[napi(factory)]
    pub fn typed_literal(dtype: DataTypeInput<'_>, value: &JsScalar) -> Result<Self> {
        CoreTerm::typed_literal(dtype_from_input(dtype)?, value.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Name one holder attribute, such as `size`, or `partition` with a column.
    #[napi(factory)]
    pub fn attribute(name: String, key: Option<String>) -> Result<Self> {
        Ok(Self::from_core(CoreTerm::attribute(attribute_from_name(
            &name,
            key.as_deref(),
        )?)))
    }

    /// Name one late-bound value.
    #[napi(factory)]
    pub fn parameter(name: String) -> Self {
        Self::from_core(CoreTerm::parameter(name))
    }

    /// The term that is true for every row.
    #[napi(factory)]
    pub fn always_true() -> Self {
        Self::from_core(CoreTerm::always_true())
    }

    /// The term that is true for no row.
    #[napi(factory)]
    pub fn always_false() -> Self {
        Self::from_core(CoreTerm::always_false())
    }

    /// Conjoin many terms into one flattened node; empty is true.
    #[napi(factory)]
    pub fn all(operands: Vec<Either<ClassInstance<'_, JsTerm>, String>>) -> Result<Self> {
        let operands = operands
            .into_iter()
            .map(term_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(CoreTerm::all(operands)))
    }

    /// Disjoin many terms into one flattened node; empty is false.
    #[napi(factory)]
    pub fn any(operands: Vec<Either<ClassInstance<'_, JsTerm>, String>>) -> Result<Self> {
        let operands = operands
            .into_iter()
            .map(term_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(CoreTerm::any(operands)))
    }

    /// Call one function of the closed scalar set over terms or their text.
    #[napi(factory)]
    pub fn call(
        name: String,
        arguments: Vec<Either<ClassInstance<'_, JsTerm>, String>>,
    ) -> Result<Self> {
        let function = CoreFunction::resolve(&name).map_err(napi_error)?;
        let arguments = arguments
            .into_iter()
            .map(term_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(CoreTerm::call(function, arguments)))
    }

    /// Every top-level column this term reads, in first-seen order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this term reads, in first-seen order.
    #[napi(getter)]
    pub fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this term names, in first-seen order.
    #[napi(getter)]
    pub fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// The top-level `and` operands, flattened.
    #[napi]
    pub fn conjuncts(&self) -> Vec<JsTerm> {
        self.inner
            .conjuncts()
            .into_iter()
            .map(JsTerm::from_core)
            .collect()
    }

    /// How deep this term nests, counting itself as one level.
    #[napi(getter)]
    pub fn depth(&self) -> u32 {
        u32::try_from(self.inner.depth()).unwrap_or(u32::MAX)
    }

    /// The number of nodes this term holds.
    #[napi(getter)]
    pub fn node_count(&self) -> u32 {
        u32::try_from(self.inner.node_count()).unwrap_or(u32::MAX)
    }

    /// Whether this node is a constant.
    #[napi(getter)]
    pub fn is_literal(&self) -> bool {
        self.inner.is_literal()
    }

    /// The column name this node reads directly, or `null`.
    #[napi(getter)]
    pub fn as_column(&self) -> Option<String> {
        self.inner.as_column().map(ToOwned::to_owned)
    }

    /// Whether this node is the constant true; an empty `all` counts.
    #[napi(getter)]
    pub fn is_always_true(&self) -> bool {
        self.inner.is_always_true()
    }

    /// Whether this node is the constant false; an empty `any` counts.
    #[napi(getter)]
    pub fn is_always_false(&self) -> bool {
        self.inner.is_always_false()
    }

    /// Whether this term reads any holder attribute.
    #[napi(getter)]
    pub fn has_attributes(&self) -> bool {
        self.inner.has_attributes()
    }

    /// Refuse a term past the depth or node limit, before a walk.
    #[napi]
    pub fn check_budget(&self) -> Result<()> {
        self.inner.check_budget().map_err(napi_error)
    }

    /// This term with the same answer and fewer nodes.
    #[napi]
    pub fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// The tree of this term, drawn one node per line.
    #[napi]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Build `this and other`.
    #[napi]
    pub fn and(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().and(term_from_input(other)?),
        ))
    }

    /// Build `this or other`.
    #[napi]
    pub fn or(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().or(term_from_input(other)?),
        ))
    }

    /// Build `not this`.
    #[napi]
    pub fn not(&self) -> Self {
        Self::from_core(self.inner.clone().not())
    }

    /// Compare this term with another under a named comparison.
    ///
    /// The vocabulary is the grammar's own - `=`, `<>`, `<`, `<=`, `>`, `>=`,
    /// `is distinct from`, `is not distinct from`.
    #[napi]
    pub fn comparison(
        &self,
        comparison: String,
        other: Either<ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        let comparison = comparison_from_text(&comparison)?;
        Ok(Self::from_core(
            self.inner
                .clone()
                .compare(comparison, term_from_input(other)?),
        ))
    }

    /// `this = other`.
    #[napi]
    pub fn eq(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().eq(term_from_input(other)?),
        ))
    }

    /// `this <> other`.
    #[napi]
    pub fn ne(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().ne(term_from_input(other)?),
        ))
    }

    /// `this < other`.
    #[napi]
    pub fn lt(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().lt(term_from_input(other)?),
        ))
    }

    /// `this <= other`.
    #[napi]
    pub fn le(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().le(term_from_input(other)?),
        ))
    }

    /// `this > other`.
    #[napi]
    pub fn gt(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().gt(term_from_input(other)?),
        ))
    }

    /// `this >= other`.
    #[napi]
    pub fn ge(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().ge(term_from_input(other)?),
        ))
    }

    /// `this in (...)`, over the terms or texts given.
    #[napi]
    pub fn is_in(&self, values: Vec<Either<ClassInstance<'_, JsTerm>, String>>) -> Result<Self> {
        let values = values
            .into_iter()
            .map(term_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(self.inner.clone().is_in(values)))
    }

    /// `this between low and high`, inclusive at both ends.
    #[napi]
    pub fn between(
        &self,
        low: Either<ClassInstance<'_, JsTerm>, String>,
        high: Either<ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .between(term_from_input(low)?, term_from_input(high)?),
        ))
    }

    /// `this is null`, which answers true or false and never unknown.
    #[napi]
    pub fn is_null(&self) -> Self {
        Self::from_core(self.inner.clone().is_null())
    }

    /// `this is not null`.
    #[napi]
    pub fn is_not_null(&self) -> Self {
        Self::from_core(self.inner.clone().is_not_null())
    }

    /// `this like pattern`, with SQL's `%` and `_` wildcards.
    #[napi]
    pub fn like(&self, pattern: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().like(term_from_input(pattern)?),
        ))
    }

    /// `this ilike pattern`, folding ASCII case.
    #[napi]
    pub fn ilike(&self, pattern: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().ilike(term_from_input(pattern)?),
        ))
    }

    /// `this glob pattern`, under the `.gitignore` path rule.
    #[napi]
    pub fn glob(&self, pattern: Either<ClassInstance<'_, JsTerm>, String>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().glob(term_from_input(pattern)?),
        ))
    }

    /// Cast this term to a datatype, refusing what it cannot hold.
    #[napi]
    pub fn cast(&self, dtype: DataTypeInput<'_>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().cast(dtype_from_input(dtype)?),
        ))
    }

    /// Cast this term to a datatype, nulling what it cannot hold.
    #[napi]
    pub fn try_cast(&self, dtype: DataTypeInput<'_>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().try_cast(dtype_from_input(dtype)?),
        ))
    }

    /// Read a struct child by name, resolved case-insensitively.
    #[napi]
    pub fn child(&self, name: String) -> Self {
        Self::from_core(self.inner.clone().child(name))
    }

    /// Read a list element by position, counting back from the end when
    /// negative.
    #[napi]
    pub fn at(&self, index: i64) -> Self {
        Self::from_core(self.inner.clone().at(index))
    }

    /// Read a run of list elements, `start` inclusive and `end` exclusive;
    /// either bound counts back from the end when negative, and an absent
    /// one is the list's own edge.
    #[napi]
    pub fn slice(&self, start: Option<i64>, end: Option<i64>) -> Self {
        Self::from_core(self.inner.clone().slice(start, end))
    }

    /// Read a map value by key.
    #[napi]
    pub fn key(&self, key: &JsScalar) -> Result<Self> {
        let segment = CoreSegment::key(key.inner.clone()).map_err(napi_error)?;
        Ok(Self::from_core(
            self.inner.clone().path([segment]).map_err(napi_error)?,
        ))
    }

    /// Build `this + other` after the loader has inferred the public input.
    #[napi(js_name = "_addNative", skip_typescript)]
    pub fn add_native(&self, other: &JsTerm) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Add, other.inner.clone()),
        )
    }

    /// Build `this - other` after the loader has inferred the public input.
    #[napi(js_name = "_subtractNative", skip_typescript)]
    pub fn subtract_native(&self, other: &JsTerm) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Sub, other.inner.clone()),
        )
    }

    /// Build `this * other` after the loader has inferred the public input.
    #[napi(js_name = "_multiplyNative", skip_typescript)]
    pub fn multiply_native(&self, other: &JsTerm) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Mul, other.inner.clone()),
        )
    }

    /// Build `this / other` after the loader has inferred the public input.
    #[napi(js_name = "_divideNative", skip_typescript)]
    pub fn divide_native(&self, other: &JsTerm) -> Self {
        Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Div, other.inner.clone()),
        )
    }

    /// Build `this % other` after the loader has inferred the public input.
    #[napi(js_name = "_remainderNative", skip_typescript)]
    pub fn remainder_native(&self, other: &JsTerm) -> Self {
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

    /// Write this term as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
    }

    /// Resolve this term against a struct root schema.
    ///
    /// The loader converts an ordinary JavaScript parameter object into the
    /// shared native nested `Record` before this redirect.
    #[napi(js_name = "_bindNative", skip_typescript)]
    pub fn bind_native(&self, schema: &JsField, parameters: Option<&JsScalar>) -> Result<JsBound> {
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(JsBound {
            inner: self
                .inner
                .bind_with(&schema.inner, &borrowed)
                .map_err(napi_error)?,
        })
    }

    /// The output field this term produces against a schema.
    #[napi]
    pub fn field(&self, schema: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.field(&schema.inner).map_err(napi_error)?,
        ))
    }

    /// The canonical text, which re-parses to this term.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two terms are the same tree.
    #[napi]
    pub fn equals(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<bool> {
        Ok(self.inner == term_from_input(other)?)
    }

    /// Compare two terms by the core's total structural order.
    #[napi]
    pub fn compare(&self, other: Either<ClassInstance<'_, JsTerm>, String>) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&term_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical term text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable tree.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Bound
// ---------------------------------------------------------------------------

/// The two halves of a predicate a partition layout splits it into.
#[napi(object, object_from_js = false)]
pub struct PartitionSplit {
    /// The conjuncts a partition path answers.
    pub answerable: JsFilter,
    /// The conjuncts the rows have to answer.
    pub remaining: JsFilter,
}

/// One term resolved against one schema, ready to answer.
#[napi(js_name = "Bound")]
pub struct JsBound {
    inner: CoreBound,
}

#[napi]
impl JsBound {
    /// The term as it stands after substitution, folding, and ordering.
    #[napi(getter)]
    pub fn term(&self) -> JsTerm {
        JsTerm::from_core(self.inner.term().clone())
    }

    /// The output field this term produces.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.field().clone())
    }

    /// The struct root this term was bound against.
    #[napi(getter)]
    pub fn schema(&self) -> JsField {
        JsField::from_core(self.inner.schema().clone())
    }

    /// Return whether this term answers a boolean.
    #[napi(getter)]
    pub fn is_predicate(&self) -> bool {
        self.inner.is_predicate()
    }

    /// The schema column names this term reads, in index order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.column_names()
    }

    /// The schema column indices this term reads, ascending: what a reader
    /// decodes and nothing else.
    #[napi(getter)]
    pub fn column_indices(&self) -> Vec<u32> {
        self.inner
            .column_indices()
            .into_iter()
            .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
            .collect()
    }

    /// Return whether answering this term requires reading rows.
    #[napi(getter)]
    pub fn reads_rows(&self) -> bool {
        self.inner.reads_rows()
    }

    /// Evaluate this term for one row of column values, in schema order.
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

    /// Split this predicate into the part a partition layout answers and the
    /// rest.
    #[napi]
    pub fn partition_split(&self) -> PartitionSplit {
        let residual = self.inner.partition_split();
        PartitionSplit {
            answerable: JsFilter::from_core(residual.answerable().clone()),
            remaining: JsFilter::from_core(residual.remaining().clone()),
        }
    }

    /// Keep the rows of every batch a native reader yields, lazily.
    #[napi(js_name = "_filterArrowReaderNative", skip_typescript)]
    pub fn filter_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let kept = self.inner.clone().filter_reader(reader.take()?);
        Ok(JsBatchReader::from_core(kept, self.inner.schema().name()))
    }

    /// Keep the rows of the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_filterArrowBatchNative", skip_typescript)]
    pub fn filter_arrow_batch_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let kept = self.inner.filter(&one_batch(reader)?).map_err(napi_error)?;
        Ok(batch_reader(kept, self.inner.schema().name()))
    }

    /// The plan this term runs, drawn one node per line with its datatype
    /// and cost.
    #[napi]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// The canonical text of the term this resolved.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

/// A `where` clause: one predicate over rows.
#[napi(js_name = "Filter")]
pub struct JsFilter {
    pub(crate) inner: CoreFilter,
}

impl Clone for JsFilter {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsFilter {
    pub(crate) const fn from_core(inner: CoreFilter) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsFilter {
    /// Read one filter from a predicate's text, a `Term`, or another filter;
    /// the `where` keyword is optional.
    #[napi(constructor)]
    pub fn new(
        value: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        Ok(Self::from_core(filter_from_input(value)?))
    }

    /// Parse one filter from its canonical text.
    #[napi(factory)]
    pub fn parse(text: String) -> Result<Self> {
        Ok(Self::from_core(text.parse().map_err(napi_error)?))
    }

    /// Read one filter from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self::from_core(
            CoreFilter::from_json(&document).map_err(napi_error)?,
        ))
    }

    /// The filter that keeps every row.
    #[napi(factory)]
    pub fn always_true() -> Self {
        Self::from_core(CoreFilter::always_true())
    }

    /// The filter that keeps no row.
    #[napi(factory)]
    pub fn always_false() -> Self {
        Self::from_core(CoreFilter::always_false())
    }

    /// Conjoin many filters into one; empty keeps every row.
    #[napi(factory)]
    pub fn all(
        operands: Vec<Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>>,
    ) -> Result<Self> {
        let operands = operands
            .into_iter()
            .map(filter_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(CoreFilter::all(operands)))
    }

    /// Disjoin many filters into one; empty keeps no row.
    #[napi(factory)]
    pub fn any(
        operands: Vec<Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>>,
    ) -> Result<Self> {
        let operands = operands
            .into_iter()
            .map(filter_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(CoreFilter::any(operands)))
    }

    /// The predicate this filter keeps rows by.
    #[napi(getter)]
    pub fn term(&self) -> JsTerm {
        JsTerm::from_core(self.inner.term().clone())
    }

    /// Whether this filter keeps every row.
    #[napi(getter)]
    pub fn is_always_true(&self) -> bool {
        self.inner.is_always_true()
    }

    /// Whether this filter keeps no row.
    #[napi(getter)]
    pub fn is_always_false(&self) -> bool {
        self.inner.is_always_false()
    }

    /// Whether this filter reads any holder attribute.
    #[napi(getter)]
    pub fn has_attributes(&self) -> bool {
        self.inner.has_attributes()
    }

    /// Every top-level column this filter reads, in first-seen order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this filter reads, in first-seen order.
    #[napi(getter)]
    pub fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this filter names, in first-seen order.
    #[napi(getter)]
    pub fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// The top-level `and` operands, each its own filter.
    #[napi]
    pub fn conjuncts(&self) -> Vec<JsFilter> {
        self.inner
            .conjuncts()
            .into_iter()
            .map(JsFilter::from_core)
            .collect()
    }

    /// This filter with the same answer and fewer nodes.
    #[napi]
    pub fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse a filter past the depth or node limit, before a walk.
    #[napi]
    pub fn check_budget(&self) -> Result<()> {
        self.inner.check_budget().map_err(napi_error)
    }

    /// Build `this and other`.
    #[napi]
    pub fn and(
        &self,
        other: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().and(filter_from_input(other)?),
        ))
    }

    /// Build `this or other`.
    #[napi]
    pub fn or(
        &self,
        other: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        Ok(Self::from_core(
            self.inner.clone().or(filter_from_input(other)?),
        ))
    }

    /// Build `not this`.
    #[napi]
    pub fn not(&self) -> Self {
        Self::from_core(self.inner.clone().not())
    }

    /// Resolve this filter against a struct root schema, as a predicate.
    #[napi(js_name = "_bindNative", skip_typescript)]
    pub fn bind_native(&self, schema: &JsField, parameters: Option<&JsScalar>) -> Result<JsBound> {
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(JsBound {
            inner: self
                .inner
                .bind_with(&schema.inner, &borrowed)
                .map_err(napi_error)?,
        })
    }

    /// The struct root the kept rows have: the schema itself, once the
    /// predicate types against it.
    #[napi]
    pub fn apply_field(&self, root: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.apply_field(&root.inner).map_err(napi_error)?,
        ))
    }

    /// Keep the rows of every batch a native reader yields, lazily.
    #[napi(js_name = "_applyArrowReaderNative", skip_typescript)]
    pub fn apply_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let kept = self
            .inner
            .apply_arrow_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(kept, &root))
    }

    /// Keep the rows of the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_applyArrowBatchNative", skip_typescript)]
    pub fn apply_arrow_batch_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let kept = self
            .inner
            .apply_arrow_batch(&one_batch(reader)?)
            .map_err(napi_error)?;
        Ok(batch_reader(kept, &root))
    }

    /// The native rows this filter keeps, bound once against `schema` or the
    /// schema the first record implies; `rows` is a Scalar sequence of
    /// records.
    #[napi(js_name = "_applyRecordsNative", skip_typescript)]
    pub fn apply_records_native(
        &self,
        rows: &JsScalar,
        schema: Option<&JsField>,
    ) -> Result<JsRecords> {
        let rows = rows_from_scalar(rows)?;
        let schema = schema.map(|field| field.inner.clone());
        Ok(JsRecords::from_core(
            self.inner
                .apply_records(schema.as_ref(), rows)
                .map_err(napi_error)?,
        ))
    }

    /// The tree of this filter, drawn one node per line.
    #[napi]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this filter as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
    }

    /// The canonical text, which re-parses to this filter.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two filters are the same predicate.
    #[napi]
    pub fn equals(
        &self,
        other: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<bool> {
        Ok(self.inner == filter_from_input(other)?)
    }

    /// Compare two filters by the core's total structural order.
    #[napi]
    pub fn compare(
        &self,
        other: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&filter_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical filter text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable filter.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Selector
// ---------------------------------------------------------------------------

/// A `select` clause: the columns published, computed, declared or excluded.
#[napi(js_name = "Selector")]
pub struct JsSelector {
    pub(crate) inner: CoreSelector,
}

impl Clone for JsSelector {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsSelector {
    pub(crate) const fn from_core(inner: CoreSelector) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsSelector {
    /// Read one selector from a projection list's text, a `Term`, another
    /// selector, or projection texts; the `select` keyword is optional.
    #[napi(constructor)]
    pub fn new(
        value: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
    ) -> Result<Self> {
        Ok(Self::from_core(selector_from_input(value)?))
    }

    /// Parse one selector from its canonical text.
    #[napi(factory)]
    pub fn parse(text: String) -> Result<Self> {
        Ok(Self::from_core(text.parse().map_err(napi_error)?))
    }

    /// Read one selector from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self::from_core(
            CoreSelector::from_json(&document).map_err(napi_error)?,
        ))
    }

    /// `select *`: every column, unchanged.
    #[napi(factory)]
    pub fn all() -> Self {
        Self::from_core(CoreSelector::all())
    }

    /// `select * exclude (...)`: every column but the named ones.
    #[napi(factory)]
    pub fn all_except(names: Vec<String>) -> Self {
        Self::from_core(CoreSelector::all_except(names))
    }

    /// The named columns, in that order, unchanged.
    #[napi(factory)]
    pub fn from_columns(names: Vec<String>) -> Self {
        Self::from_core(CoreSelector::from_columns(names))
    }

    /// The selector a struct root declares: one column per child, with its
    /// datatype, nullability, metadata, and any `transform:` it carries.
    #[napi(factory)]
    pub fn from_field(field: &JsField) -> Self {
        Self::from_core(CoreSelector::from_field(&field.inner))
    }

    /// Each projection, as its canonical text.
    #[napi(getter)]
    pub fn projections(&self) -> Vec<String> {
        self.inner
            .projections()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The names this selector publishes, in output order; empty for `*`.
    #[napi(getter)]
    pub fn names(&self) -> Vec<String> {
        self.inner
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    }

    /// The column names `select * exclude (...)` drops.
    #[napi(getter)]
    pub fn excluded(&self) -> Vec<String> {
        self.inner
            .excluded()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Whether this is `select *` with nothing excluded.
    #[napi(getter)]
    pub fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Whether every projection is a bare column.
    #[napi(getter)]
    pub fn is_columns(&self) -> bool {
        self.inner.is_columns()
    }

    /// How many projections this selector holds.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// Every top-level column this selector reads, in first-seen order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this selector reads, in first-seen order.
    #[napi(getter)]
    pub fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this selector names, in first-seen order.
    #[napi(getter)]
    pub fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// This selector with one more projection, a term or its text.
    #[napi]
    pub fn with_projection(
        &self,
        projection: Either<ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        let projection = match projection {
            Either::A(term) => CoreProjection::new(term.inner.clone()),
            Either::B(text) => text.parse().map_err(napi_error)?,
        };
        Ok(Self::from_core(self.inner.with_projection(projection)))
    }

    /// This selector without the projections that are these bare columns.
    #[napi]
    pub fn without_columns(&self, names: Vec<String>) -> Self {
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        Self::from_core(self.inner.without_columns(&borrowed))
    }

    /// This selector with every term simplified and every self alias dropped.
    #[napi]
    pub fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse a selector past the depth or node limit, before a walk.
    #[napi]
    pub fn check_budget(&self) -> Result<()> {
        self.inner.check_budget().map_err(napi_error)
    }

    /// Resolve every projection against one struct root schema.
    #[napi(js_name = "_bindNative", skip_typescript)]
    pub fn bind_native(
        &self,
        schema: &JsField,
        parameters: Option<&JsScalar>,
    ) -> Result<JsBoundSelector> {
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(JsBoundSelector {
            inner: self
                .inner
                .bind_with(&schema.inner, &borrowed)
                .map_err(napi_error)?,
        })
    }

    /// The struct root this selector publishes from `root`.
    #[napi]
    pub fn apply_field(&self, root: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.apply_field(&root.inner).map_err(napi_error)?,
        ))
    }

    /// The struct root this selector publishes from `root`, carrying the
    /// selector itself as each column's `transform:` declaration, so
    /// `Selector.fromField` of the answer is this selector again.
    #[napi]
    pub fn into_field(&self, root: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.into_field(&root.inner).map_err(napi_error)?,
        ))
    }

    /// Project every batch a native reader yields, lazily.
    #[napi(js_name = "_applyArrowReaderNative", skip_typescript)]
    pub fn apply_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let projected = self
            .inner
            .apply_arrow_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(projected, &root))
    }

    /// Project the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_applyArrowBatchNative", skip_typescript)]
    pub fn apply_arrow_batch_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let projected = self
            .inner
            .apply_arrow_batch(&one_batch(reader)?)
            .map_err(napi_error)?;
        Ok(batch_reader(projected, &root))
    }

    /// The rows this selector publishes from native records, bound once
    /// against `schema` or the schema the first record implies.
    #[napi(js_name = "_applyRecordsNative", skip_typescript)]
    pub fn apply_records_native(
        &self,
        rows: &JsScalar,
        schema: Option<&JsField>,
    ) -> Result<JsRecords> {
        let rows = rows_from_scalar(rows)?;
        let schema = schema.map(|field| field.inner.clone());
        Ok(JsRecords::from_core(
            self.inner
                .apply_records(schema.as_ref(), rows)
                .map_err(napi_error)?,
        ))
    }

    /// The tree of this selector, one branch per projection.
    #[napi]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this selector as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
    }

    /// The canonical text, which re-parses to this selector.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two selectors publish the same columns.
    #[napi]
    pub fn equals(
        &self,
        other: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
    ) -> Result<bool> {
        Ok(self.inner == selector_from_input(other)?)
    }

    /// Compare two selectors by the core's total structural order.
    #[napi]
    pub fn compare(
        &self,
        other: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
    ) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&selector_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical selector text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable selector.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// A selector resolved against one schema, ready for batches.
#[napi(js_name = "BoundSelector")]
pub struct JsBoundSelector {
    inner: CoreBoundSelector,
}

#[napi]
impl JsBoundSelector {
    /// The struct root the selector reads.
    #[napi(getter)]
    pub fn schema(&self) -> JsField {
        JsField::from_core(self.inner.schema().clone())
    }

    /// The struct root the selector publishes.
    #[napi(getter)]
    pub fn output(&self) -> JsField {
        JsField::from_core(self.inner.output().clone())
    }

    /// The bound projections, in output order; empty for `select *`.
    #[napi(getter)]
    pub fn projections(&self) -> Vec<JsBound> {
        self.inner
            .projections()
            .iter()
            .cloned()
            .map(|inner| JsBound { inner })
            .collect()
    }

    /// Whether this selector publishes every input column unchanged.
    #[napi(getter)]
    pub fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Whether applying this selector changes nothing, so a batch is handed
    /// back as it is.
    #[napi(getter)]
    pub fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// The row this selector publishes from one row of column values, in
    /// schema order.
    #[napi]
    pub fn apply_row(&self, row: &JsScalar) -> Result<JsScalar> {
        self.inner
            .apply_scalar(&row.inner)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Lazily project every batch a native reader yields.
    #[napi(js_name = "_applyArrowReaderNative", skip_typescript)]
    pub fn apply_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let projected = self
            .inner
            .apply_arrow_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(
            projected,
            self.inner.output().name(),
        ))
    }

    /// Project the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_applyArrowBatchNative", skip_typescript)]
    pub fn apply_arrow_batch_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let projected = self
            .inner
            .apply_arrow_batch(&one_batch(reader)?)
            .map_err(napi_error)?;
        Ok(batch_reader(projected, self.inner.output().name()))
    }

    /// The bound plan, drawn one node per line with datatypes and costs.
    #[napi]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// The struct root this selector publishes, as text.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.output().to_string()
    }
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// One `order by` key exposed for inspection.
#[napi(object, object_from_js = false)]
pub struct PlanOrder {
    /// The term ordered by.
    pub term: JsTerm,
    /// `asc` or `desc`.
    #[napi(ts_type = "'asc' | 'desc'")]
    pub direction: String,
    /// `first` or `last`: where nulls go.
    #[napi(ts_type = "'first' | 'last'")]
    pub nulls: String,
}

/// The sections of one read or write: what it creates, writes, selects,
/// reads from, keeps, orders, and how many rows.
#[napi(js_name = "Plan")]
pub struct JsPlan {
    pub(crate) inner: CorePlan,
}

impl Clone for JsPlan {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsPlan {
    pub(crate) const fn from_core(inner: CorePlan) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsPlan {
    /// Read one plan from its text, a clause, a `Field` (the `create`
    /// section it declares), or another plan; nothing is the empty plan.
    #[napi(constructor)]
    pub fn new(
        value: Option<
            Either5<
                ClassInstance<'_, JsPlan>,
                ClassInstance<'_, JsSelector>,
                ClassInstance<'_, JsFilter>,
                ClassInstance<'_, JsField>,
                String,
            >,
        >,
    ) -> Result<Self> {
        Ok(Self::from_core(match value {
            Some(value) => plan_from_input(value)?,
            None => CorePlan::new(),
        }))
    }

    /// Parse one plan from its canonical text.
    #[napi(factory)]
    pub fn parse(text: String) -> Result<Self> {
        Ok(Self::from_core(text.parse().map_err(napi_error)?))
    }

    /// Read one plan from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self::from_core(
            CorePlan::from_json(&document).map_err(napi_error)?,
        ))
    }

    /// The plan a struct root is: a `create` section declaring it.
    #[napi(factory)]
    pub fn from_field(field: &JsField) -> Self {
        Self::from_core(CorePlan::from_field(&field.inner))
    }

    /// The `create` section's target, spelled as the grammar spells it.
    #[napi(getter)]
    pub fn create_target(&self) -> Option<String> {
        self.inner.create_target().map(ToString::to_string)
    }

    /// The `create` section's schema, when the plan has one.
    #[napi(getter)]
    pub fn schema(&self) -> Option<JsSelector> {
        self.inner.schema().cloned().map(JsSelector::from_core)
    }

    /// The name of the struct root this plan declares or reads.
    #[napi(getter)]
    pub fn root_name(&self) -> String {
        self.inner.root_name().to_owned()
    }

    /// The metadata the `create` section declares on the root, `with (...)`.
    #[napi(getter)]
    pub fn root_metadata(&self) -> Vec<MetadataEntry> {
        self.inner
            .root_metadata()
            .iter()
            .map(|(key, value)| MetadataEntry {
                key: key.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    }

    /// The write verb - `insert into`, `insert overwrite`, `upsert into`,
    /// `delete from` - or `null` for a read.
    #[napi(getter)]
    pub fn verb(&self) -> Option<String> {
        self.inner
            .write_section()
            .map(|write| write.verb().as_str().to_owned())
    }

    /// The write section's target, when the write names one.
    #[napi(getter)]
    pub fn write_target(&self) -> Option<String> {
        self.inner
            .write_section()
            .and_then(CoreWrite::target)
            .map(ToString::to_string)
    }

    /// The keys an upsert matches stored rows on; empty otherwise.
    #[napi(getter)]
    pub fn merge_by(&self) -> JsSelector {
        JsSelector::from_core(self.inner.merge_by().clone())
    }

    /// The `select` section; `select *` when absent.
    #[napi(getter)]
    pub fn selector(&self) -> JsSelector {
        JsSelector::from_core(self.inner.selector().clone())
    }

    /// The `from` section, spelled as the grammar spells it, or `null`.
    #[napi(getter)]
    pub fn source(&self) -> Option<String> {
        self.inner.source().map(ToString::to_string)
    }

    /// The plan the `from` section reads, when it is one in parentheses.
    #[napi(getter)]
    pub fn source_plan(&self) -> Option<JsPlan> {
        match self.inner.source() {
            Some(CoreSource::Plan(plan)) => Some(Self::from_core((**plan).clone())),
            _ => None,
        }
    }

    /// The `where` section; always true when absent.
    #[napi(getter)]
    pub fn filter(&self) -> JsFilter {
        JsFilter::from_core(self.inner.filter_section().clone())
    }

    /// The `order by` keys, in priority order.
    #[napi(getter)]
    pub fn ordering(&self) -> Vec<PlanOrder> {
        self.inner
            .ordering()
            .iter()
            .map(|key| PlanOrder {
                term: JsTerm::from_core(key.term().clone()),
                direction: if key.is_descending() { "desc" } else { "asc" }.to_owned(),
                nulls: if key.is_nulls_first() {
                    "first"
                } else {
                    "last"
                }
                .to_owned(),
            })
            .collect()
    }

    /// The row limit, when the plan has one.
    #[napi(getter)]
    pub fn limit(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.row_limit().map(|limit| limit as f64)
    }

    /// The rows skipped before the first kept, when the plan has an offset.
    #[napi(getter)]
    pub fn offset(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.row_offset().map(|offset| offset as f64)
    }

    /// Whether the plan has no section at all.
    #[napi(getter)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether applying the plan changes nothing: no write, no schema, and
    /// read sections that keep every row and column.
    #[napi(getter)]
    pub fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// This plan with a `create` section: the schema it declares, and a
    /// target's text or `null` for the handle the plan is given to.
    #[napi]
    pub fn with_create(
        &self,
        schema: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
        target: Option<String>,
    ) -> Result<Self> {
        let target = target.as_deref().map(target_from_text).transpose()?;
        Ok(Self::from_core(
            self.inner
                .clone()
                .create(target, selector_from_input(schema)?),
        ))
    }

    /// This plan with a write section: a verb in any spelling the grammar
    /// reads, a target's text or `null` for the handle the plan is given
    /// to, and the keys an upsert matches on.
    #[napi]
    pub fn with_write(
        &self,
        verb: String,
        target: Option<String>,
        merge_by: Option<
            Either4<
                ClassInstance<'_, JsSelector>,
                ClassInstance<'_, JsTerm>,
                String,
                Vec<Either<ClassInstance<'_, JsTerm>, String>>,
            >,
        >,
    ) -> Result<Self> {
        let mut write = CoreWrite::new(verb_from_text(&verb)?);
        if let Some(target) = target {
            write = write.into(target_from_text(&target)?);
        }
        if let Some(merge_by) = merge_by {
            write = write.by(selector_from_input(merge_by)?);
        }
        Ok(Self::from_core(self.inner.clone().write(write)))
    }

    /// This plan with a `select` section, replacing any it carries.
    #[napi]
    pub fn with_select(
        &self,
        selector: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
    ) -> Result<Self> {
        let mut plan = self.inner.clone();
        plan.set_selector(selector_from_input(selector)?);
        Ok(Self::from_core(plan))
    }

    /// This plan reading from a target's text or from another plan.
    #[napi]
    pub fn with_source(&self, source: Either<ClassInstance<'_, JsPlan>, String>) -> Result<Self> {
        let source = match source {
            Either::A(plan) => CoreSource::Plan(Box::new(plan.inner.clone())),
            Either::B(text) => CoreSource::Target(target_from_text(&text)?),
        };
        Ok(Self::from_core(self.inner.clone().read_from(source)))
    }

    /// This plan with a `where` section, replacing any it carries.
    #[napi]
    pub fn with_filter(
        &self,
        filter: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        let mut plan = self.inner.clone();
        plan.set_filter(filter_from_input(filter)?);
        Ok(Self::from_core(plan))
    }

    /// This plan with an `order by`, replacing any it carries; each key is
    /// spelled as the grammar spells it, `"size desc nulls first"`.
    #[napi]
    pub fn with_ordering(&self, keys: Vec<String>) -> Result<Self> {
        let keys = keys
            .iter()
            .map(|key| ordering_from_text(key))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(self.inner.clone().order_by(keys)))
    }

    /// This plan with a row limit; `null` lifts it.
    #[napi]
    pub fn with_limit(&self, limit: Option<f64>) -> Result<Self> {
        let limit = limit
            .map(|limit| crate::exact_u64(limit, "limit"))
            .transpose()?;
        Ok(Self::from_core(self.inner.clone().limit(limit)))
    }

    /// This plan skipping `offset` rows before the first kept; `null` lifts it.
    #[napi]
    pub fn with_offset(&self, offset: Option<f64>) -> Result<Self> {
        let offset = offset
            .map(|offset| crate::exact_u64(offset, "offset"))
            .transpose()?;
        Ok(Self::from_core(self.inner.clone().offset(offset)))
    }

    /// This plan matching stored rows on these keys, which makes it an
    /// upsert when it has no write section.
    #[napi]
    pub fn with_merge_by(
        &self,
        merge_by: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
    ) -> Result<Self> {
        let mut plan = self.inner.clone();
        plan.set_merge_by(selector_from_input(merge_by)?);
        Ok(Self::from_core(plan))
    }

    /// The struct root the `create` section declares, or `null`.
    #[napi]
    pub fn field(&self) -> Result<Option<JsField>> {
        Ok(self
            .inner
            .field()
            .map_err(napi_error)?
            .map(JsField::from_core))
    }

    /// The struct root this plan publishes from `root`.
    #[napi]
    pub fn field_from(&self, root: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.field_from(&root.inner).map_err(napi_error)?,
        ))
    }

    /// Every top-level column this plan reads, in first-seen order.
    #[napi(getter)]
    pub fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// The stored columns a read has to decode, or `null` for all of them.
    #[napi(getter)]
    pub fn read_columns(&self) -> Option<Vec<String>> {
        self.inner.read_columns()
    }

    /// Every holder attribute this plan reads, in first-seen order.
    #[napi(getter)]
    pub fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this plan names, in first-seen order.
    #[napi(getter)]
    pub fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// The sections that shape a read - `select`, `where`, `order by`,
    /// `limit`, `offset` - as a plan of their own.
    #[napi]
    pub fn read_sections(&self) -> Self {
        Self::from_core(self.inner.read_sections())
    }

    /// This plan with every section simplified.
    #[napi]
    pub fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse a plan past the depth or node limit, before a walk.
    #[napi]
    pub fn check_budget(&self) -> Result<()> {
        self.inner.check_budget().map_err(napi_error)
    }

    /// This plan as an expression: a lone `select` is a `Selector`, a lone
    /// `where` a `Filter`, anything else the plan itself.
    #[napi]
    pub fn into_expression(&self) -> JsExpression {
        JsExpression::from_core(self.inner.clone().into_expression())
    }

    /// Run this plan from its own `from` source.
    ///
    /// A target source is read through its holder with the read sections
    /// pushed down; a nested plan runs first; no source is the empty stream.
    /// A plan with a `create` or write section writes and yields the empty
    /// stream under the schema it wrote.
    #[napi]
    pub fn execute(&self) -> Result<JsBatchReader> {
        let reader = self.inner.execute().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, self.inner.root_name()))
    }

    /// Apply this plan to every batch a native reader yields.
    #[napi(js_name = "_applyArrowReaderNative", skip_typescript)]
    pub fn apply_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let applied = self
            .inner
            .apply_arrow_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(applied, &root))
    }

    /// Apply this plan to the one batch carried by this private Arrow bridge.
    #[napi(js_name = "_applyArrowBatchNative", skip_typescript)]
    pub fn apply_arrow_batch_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let expression = CoreExpression::Plan(Box::new(self.inner.clone()));
        let applied = expression
            .apply_arrow_batch(&one_batch(reader)?)
            .map_err(napi_error)?;
        Ok(batch_reader(applied, &root))
    }

    /// The native rows this plan publishes, run through the vectorized tier.
    #[napi(js_name = "_applyRecordsNative", skip_typescript)]
    pub fn apply_records_native(
        &self,
        rows: &JsScalar,
        schema: Option<&JsField>,
    ) -> Result<JsRecords> {
        let rows = rows_from_scalar(rows)?;
        let schema = schema.map(|field| field.inner.clone());
        let expression = CoreExpression::Plan(Box::new(self.inner.clone()));
        Ok(JsRecords::from_core(
            expression
                .apply_records(schema.as_ref(), rows)
                .map_err(napi_error)?,
        ))
    }

    /// The tree of this plan, one branch per section in the order they run.
    #[napi]
    pub fn explain(&self) -> String {
        CoreExpression::Plan(Box::new(self.inner.clone())).explain()
    }

    /// Write this plan as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
    }

    /// The canonical text, which re-parses to this plan.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two plans describe the same operation.
    #[napi]
    pub fn equals(
        &self,
        other: Either5<
            ClassInstance<'_, JsPlan>,
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsFilter>,
            ClassInstance<'_, JsField>,
            String,
        >,
    ) -> Result<bool> {
        Ok(self.inner == plan_from_input(other)?)
    }

    /// Compare two plans by the core's total structural order.
    #[napi]
    pub fn compare(
        &self,
        other: Either5<
            ClassInstance<'_, JsPlan>,
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsFilter>,
            ClassInstance<'_, JsField>,
            String,
        >,
    ) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&plan_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical plan text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable plan.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Expression
// ---------------------------------------------------------------------------

/// A clause, a plan, or a sequence of plans: whatever one piece of
/// expression text is.
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
    /// Read one expression from its text, a clause, a plan, or another
    /// expression.
    #[napi(constructor)]
    pub fn new(
        value: Either5<
            ClassInstance<'_, JsExpression>,
            ClassInstance<'_, JsPlan>,
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsFilter>,
            String,
        >,
    ) -> Result<Self> {
        Ok(Self::from_core(expression_from_input(value)?))
    }

    /// Parse one expression from its canonical text.
    #[napi(factory)]
    pub fn parse(text: String) -> Result<Self> {
        Ok(Self::from_core(text.parse().map_err(napi_error)?))
    }

    /// Read one expression from its structural JSON document.
    #[napi(factory)]
    pub fn from_json(document: String) -> Result<Self> {
        Ok(Self::from_core(
            CoreExpression::from_json(&document).map_err(napi_error)?,
        ))
    }

    /// The `select` expression of one selector.
    #[napi(factory)]
    pub fn select(
        selector: Either4<
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsTerm>,
            String,
            Vec<Either<ClassInstance<'_, JsTerm>, String>>,
        >,
    ) -> Result<Self> {
        Ok(Self::from_core(CoreExpression::Selector(
            selector_from_input(selector)?,
        )))
    }

    /// The `where` expression of one filter.
    #[napi(factory)]
    pub fn filter(
        filter: Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>,
    ) -> Result<Self> {
        Ok(Self::from_core(CoreExpression::Filter(filter_from_input(
            filter,
        )?)))
    }

    /// The expression of one plan: a lone clause collapses into it.
    #[napi(factory)]
    pub fn plan(
        plan: Either5<
            ClassInstance<'_, JsPlan>,
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsFilter>,
            ClassInstance<'_, JsField>,
            String,
        >,
    ) -> Result<Self> {
        Ok(Self::from_core(plan_from_input(plan)?.into_expression()))
    }

    /// The sequence of expressions, applied in order; one is itself.
    #[napi(factory)]
    pub fn sequence(
        steps: Vec<
            Either5<
                ClassInstance<'_, JsExpression>,
                ClassInstance<'_, JsPlan>,
                ClassInstance<'_, JsSelector>,
                ClassInstance<'_, JsFilter>,
                String,
            >,
        >,
    ) -> Result<Self> {
        let steps = steps
            .into_iter()
            .map(expression_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_core(CoreExpression::sequence(steps)))
    }

    /// Which kind this expression is: `select`, `where`, `plan`, or
    /// `sequence`.
    #[napi(getter, ts_return_type = "'select' | 'where' | 'plan' | 'sequence'")]
    pub fn kind(&self) -> String {
        match &self.inner {
            CoreExpression::Selector(_) => "select",
            CoreExpression::Filter(_) => "where",
            CoreExpression::Plan(_) => "plan",
            CoreExpression::Sequence(_) => "sequence",
        }
        .to_owned()
    }

    /// Whether this is a `select` clause.
    #[napi(getter)]
    pub fn is_selector(&self) -> bool {
        self.inner.is_selector()
    }

    /// Whether this is a `where` clause.
    #[napi(getter)]
    pub fn is_filter(&self) -> bool {
        self.inner.is_filter()
    }

    /// Whether this is a plan with more than one section.
    #[napi(getter)]
    pub fn is_plan(&self) -> bool {
        self.inner.is_plan()
    }

    /// Whether this is a sequence of expressions.
    #[napi(getter)]
    pub fn is_sequence(&self) -> bool {
        self.inner.is_sequence()
    }

    /// Whether applying this expression changes nothing.
    #[napi(getter)]
    pub fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// The selector, when this is a `select` clause.
    #[napi]
    pub fn as_selector(&self) -> Option<JsSelector> {
        self.inner.as_selector().cloned().map(JsSelector::from_core)
    }

    /// The filter, when this is a `where` clause.
    #[napi]
    pub fn as_filter(&self) -> Option<JsFilter> {
        self.inner.as_filter().cloned().map(JsFilter::from_core)
    }

    /// The plan, when this is one.
    #[napi]
    pub fn as_plan(&self) -> Option<JsPlan> {
        self.inner.as_plan().cloned().map(JsPlan::from_core)
    }

    /// The steps of a sequence; any other expression is its own one step.
    #[napi(getter)]
    pub fn steps(&self) -> Vec<JsExpression> {
        self.inner
            .steps()
            .iter()
            .cloned()
            .map(JsExpression::from_core)
            .collect()
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

    /// This expression with every clause simplified.
    #[napi]
    pub fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse an expression past the depth or node limit, before a walk.
    #[napi]
    pub fn check_budget(&self) -> Result<()> {
        self.inner.check_budget().map_err(napi_error)
    }

    /// The struct root this expression publishes from `root`.
    #[napi]
    pub fn apply_field(&self, root: &JsField) -> Result<JsField> {
        Ok(JsField::from_core(
            self.inner.apply_field(&root.inner).map_err(napi_error)?,
        ))
    }

    /// Apply this expression to every batch a native reader yields, lazily.
    #[napi(js_name = "_applyArrowReaderNative", skip_typescript)]
    pub fn apply_arrow_reader_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let applied = self
            .inner
            .apply_arrow_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(applied, &root))
    }

    /// Apply this expression to the one batch carried by this private Arrow
    /// bridge.
    #[napi(js_name = "_applyArrowBatchNative", skip_typescript)]
    pub fn apply_arrow_batch_native(&self, reader: &mut JsBatchReader) -> Result<JsBatchReader> {
        let root = reader.root_name().to_owned();
        let applied = self
            .inner
            .apply_arrow_batch(&one_batch(reader)?)
            .map_err(napi_error)?;
        Ok(batch_reader(applied, &root))
    }

    /// The native rows this expression publishes.
    #[napi(js_name = "_applyRecordsNative", skip_typescript)]
    pub fn apply_records_native(
        &self,
        rows: &JsScalar,
        schema: Option<&JsField>,
    ) -> Result<JsRecords> {
        let rows = rows_from_scalar(rows)?;
        let schema = schema.map(|field| field.inner.clone());
        Ok(JsRecords::from_core(
            self.inner
                .apply_records(schema.as_ref(), rows)
                .map_err(napi_error)?,
        ))
    }

    /// The tree of this expression, its clause, plan or sequence at the root.
    #[napi]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this expression as a structural JSON document.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.clone().into_json().map_err(napi_error)
    }

    /// The canonical text, which re-parses to this expression.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether two expressions are the same.
    #[napi]
    pub fn equals(
        &self,
        other: Either5<
            ClassInstance<'_, JsExpression>,
            ClassInstance<'_, JsPlan>,
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsFilter>,
            String,
        >,
    ) -> Result<bool> {
        Ok(self.inner == expression_from_input(other)?)
    }

    /// Compare two expressions by the core's total structural order.
    #[napi]
    pub fn compare(
        &self,
        other: Either5<
            ClassInstance<'_, JsExpression>,
            ClassInstance<'_, JsPlan>,
            ClassInstance<'_, JsSelector>,
            ClassInstance<'_, JsFilter>,
            String,
        >,
    ) -> Result<i32> {
        Ok(crate::ordering_value(
            self.inner.cmp(&expression_from_input(other)?),
        ))
    }

    /// Return deterministic hash bits for the canonical expression text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone of this immutable expression.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Native rows streaming out of an expression, each a `Scalar` sequence in
/// the order of `field`.
#[napi(js_name = "Records")]
pub struct JsRecords {
    field: CoreField,
    rows: Option<CoreRecords>,
}

impl JsRecords {
    fn from_core(records: CoreRecords) -> Self {
        Self {
            field: records.field().clone(),
            rows: Some(records),
        }
    }
}

#[napi]
impl JsRecords {
    /// The struct root every row is shaped under.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.field.clone())
    }

    /// The next row, or `null` once every row was yielded.
    #[napi(js_name = "_nextNative", skip_typescript)]
    pub fn next_native(&mut self) -> Result<Option<JsScalar>> {
        match self.rows.as_mut().and_then(Iterator::next) {
            Some(row) => Ok(Some(JsScalar::from_core(row.map_err(napi_error)?))),
            None => Ok(None),
        }
    }

    /// The remaining rows as a native batch reader, batched lazily.
    #[napi]
    pub fn into_arrow_reader(&mut self) -> Result<JsBatchReader> {
        let rows = self
            .rows
            .take()
            .ok_or_else(|| Error::from_reason("these records were already consumed"))?;
        let reader = rows.into_arrow_reader().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, self.field.name()))
    }

    /// The records a native batch reader holds, one row at a time.
    #[napi(factory)]
    pub fn from_arrow_reader(reader: &mut JsBatchReader) -> Result<Self> {
        Ok(Self::from_core(
            CoreRecords::from_arrow_reader(reader.take()?).map_err(napi_error)?,
        ))
    }

    /// The struct root the rows are shaped under, as text.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.field.to_string()
    }
}

/// The grammar's own vocabularies, as the canonical spellings they cross as.
#[napi(object, object_from_js = false)]
pub struct ExpressionVocabularies {
    /// Every comparison the grammar knows, e.g. `=`, `is distinct from`.
    pub comparisons: Vec<String>,
    /// Every function the closed scalar set knows, e.g. `year`, `truncate`.
    pub functions: Vec<String>,
    /// Every holder attribute `&holder.<name>` can name, e.g. `size`.
    pub holder_attributes: Vec<String>,
    /// Every write verb a plan spells canonically, e.g. `upsert into`.
    pub verbs: Vec<String>,
}

/// The grammar's own vocabularies, as the canonical spellings they cross as.
#[napi]
pub fn expression_vocabularies() -> ExpressionVocabularies {
    ExpressionVocabularies {
        comparisons: CoreComparison::ALL
            .iter()
            .map(|comparison| comparison.as_str().to_owned())
            .collect(),
        functions: CoreFunction::ALL
            .iter()
            .map(|function| function.as_str().to_owned())
            .collect(),
        holder_attributes: Attribute::ALL
            .iter()
            .map(|attribute| attribute.as_str().to_owned())
            .collect(),
        verbs: [
            CoreVerb::Insert,
            CoreVerb::Overwrite,
            CoreVerb::Upsert,
            CoreVerb::Delete,
        ]
        .iter()
        .map(|verb| verb.as_str().to_owned())
        .collect(),
    }
}

/// Return whether an identifier has to be quoted to survive the grammar.
#[napi]
pub fn expression_needs_quoting(name: String) -> bool {
    yggdryl::expression::needs_quoting(&name)
}
