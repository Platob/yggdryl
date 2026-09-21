//! Expressions, as Python values.
//!
//! The same layers the Rust crate has, one class each: `Term` is the tree a
//! `where` or a projection is built from, `Bound` is a term compiled against
//! one schema, `Filter` and `Selector` are the two clauses, `Plan` is the
//! sections of one read or write, and `Expression` is whichever of those a
//! piece of text turns out to be - a clause, a plan, or a `;`-separated
//! sequence of plans. `Records` streams native rows through any of them.
//!
//! Text parses through the same grammar in every language, so a predicate
//! written in a Python notebook is the predicate a Rust reader runs and the
//! predicate a JavaScript caller sends. Everywhere a clause is accepted,
//! `str` is accepted too and *parses* - it is never taken as a string
//! literal, because a filter that silently matches everything is the worst
//! failure this layer could have.

use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};
use std::sync::Arc;

use yggdryl::FieldValue as _;
use yggdryl::expression::{
    Attribute, Bound as CoreBound, BoundSelector as CoreBoundSelector, Bounds as CoreBounds,
    ColumnBounds as CoreColumnBounds, Comparison as CoreComparison, FieldSegment as CoreSegment,
    Function as CoreFunction, IntoFilter, IntoPlan, IntoSelector, Operator,
    Ordering as CoreOrdering, Plan as CorePlan, Projection as CoreProjection,
    Records as CoreRecords, Source as CoreSource, Target as CoreTarget, Term as CoreTerm,
    Verb as CoreVerb, Write as CoreWrite,
};
use yggdryl::expression::{
    FunctionSignature as CoreFunctionSignature, UserFunction as CoreUserFunction,
    UserRef as CoreUserRef, lookup_function, register_function, registered_functions,
    unregister_function,
};
use yggdryl::{
    Expression as CoreExpression, Field as CoreField, Filter as CoreFilter, Scalar,
    Selector as CoreSelector,
};

use crate::datatype::{
    PyDataType, arrow_array_from_pyarrow, arrow_array_to_pyarrow, core_dtype_from_value,
};
use crate::field::{PyField, core_field_from_value};
use crate::iomedia::{
    batch_reader_from_arrow_reader, batch_reader_from_arrow_table, batch_reader_to_pyarrow,
    batch_to_pyarrow, record_batch_from_value,
};
use crate::scalar::PyScalar;
use crate::value_error;

// ---------------------------------------------------------------------------
// Reading Python values as the core's
// ---------------------------------------------------------------------------

/// Read late-bound values once, before binding.
fn supplied_parameters(parameters: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(String, Scalar)>> {
    match parameters {
        Some(parameters) => parameters
            .iter()
            .map(|(name, value)| Ok((name.extract::<String>()?, crate::scalar::from_py(&value)?)))
            .collect(),
        None => Ok(Vec::new()),
    }
}

/// Borrow the core parameter shape for exactly one bind call.
fn parameter_refs(parameters: &[(String, Scalar)]) -> Vec<(&str, Scalar)> {
    parameters
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect()
}

/// Read a term from a `Term`, a `Filter`, or any scalar: text parses, and
/// every other value is the literal it is, as [`CoreTerm::from_scalar`]
/// reads one.
pub(crate) fn term_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreTerm> {
    if let Ok(term) = value.extract::<PyRef<'_, PyTerm>>() {
        return Ok(term.inner.clone());
    }
    if let Ok(filter) = value.extract::<PyRef<'_, PyFilter>>() {
        return Ok(filter.inner.term().clone());
    }
    CoreTerm::from_scalar(&crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read one projection: a `Term`, or any scalar [`CoreProjection::from_scalar`]
/// reads - text, or a `(term, alias)` pair.
fn projection_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreProjection> {
    if let Ok(term) = value.extract::<PyRef<'_, PyTerm>>() {
        return Ok(CoreProjection::new(term.inner.clone()));
    }
    CoreProjection::from_scalar(&crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read a list of operands, each a term or a value.
fn operands_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<CoreTerm>> {
    let mut operands = Vec::new();
    for operand in value.try_iter()? {
        operands.push(term_from_value(&operand?)?);
    }
    Ok(operands)
}

/// Read a filter from a `Filter`, a `Term`, an `Expression`, or any scalar
/// [`CoreFilter::from_scalar`] reads: text, a boolean, a list of conditions.
pub(crate) fn filter_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreFilter> {
    if let Ok(filter) = value.extract::<PyRef<'_, PyFilter>>() {
        return Ok(filter.inner.clone());
    }
    if let Ok(term) = value.extract::<PyRef<'_, PyTerm>>() {
        return Ok(CoreFilter::new(term.inner.clone()));
    }
    if let Ok(expression) = value.extract::<PyRef<'_, PyExpression>>() {
        return expression.inner.clone().into_filter().map_err(value_error);
    }
    CoreFilter::from_scalar(&crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read a selector from a `Selector`, a `Term`, an `Expression`, or any
/// scalar [`CoreSelector::from_scalar`] reads: text, a list of projections
/// (each text, a `Term`, or a `(term, alias)` pair), or a dict of aliases.
pub(crate) fn selector_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreSelector> {
    if let Ok(selector) = value.extract::<PyRef<'_, PySelector>>() {
        return Ok(selector.inner.clone());
    }
    if let Ok(term) = value.extract::<PyRef<'_, PyTerm>>() {
        return Ok(CoreSelector::from(CoreProjection::new(term.inner.clone())));
    }
    if let Ok(expression) = value.extract::<PyRef<'_, PyExpression>>() {
        return expression
            .inner
            .clone()
            .into_selector()
            .map_err(value_error);
    }
    CoreSelector::from_scalar(&crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read a plan from a `Plan`, a clause, an `Expression`, a `Field`, or the
/// text of one.
pub(crate) fn plan_from_value(value: &Bound<'_, PyAny>) -> PyResult<CorePlan> {
    if let Ok(plan) = value.extract::<PyRef<'_, PyPlan>>() {
        return Ok(plan.inner.clone());
    }
    if let Ok(selector) = value.extract::<PyRef<'_, PySelector>>() {
        return Ok(CorePlan::from(selector.inner.clone()));
    }
    if let Ok(filter) = value.extract::<PyRef<'_, PyFilter>>() {
        return Ok(CorePlan::from(filter.inner.clone()));
    }
    if let Ok(expression) = value.extract::<PyRef<'_, PyExpression>>() {
        return expression.inner.clone().into_plan().map_err(value_error);
    }
    if let Ok(field) = value.extract::<PyRef<'_, PyField>>() {
        return Ok(CorePlan::from_field(&field.inner));
    }
    CorePlan::from_scalar(&crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read an expression from an `Expression`, a `Plan`, a clause, or any
/// scalar [`CoreExpression::from_scalar`] reads: text or a list of steps.
pub(crate) fn expression_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreExpression> {
    if let Ok(expression) = value.extract::<PyRef<'_, PyExpression>>() {
        return Ok(expression.inner.clone());
    }
    if let Ok(plan) = value.extract::<PyRef<'_, PyPlan>>() {
        return Ok(plan.inner.clone().into_expression());
    }
    if let Ok(selector) = value.extract::<PyRef<'_, PySelector>>() {
        return Ok(CoreExpression::Selector(selector.inner.clone()));
    }
    if let Ok(filter) = value.extract::<PyRef<'_, PyFilter>>() {
        return Ok(CoreExpression::Filter(filter.inner.clone()));
    }
    CoreExpression::from_scalar(&crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read one target: text a location is spelled as, quoted URL or catalog path.
fn target_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreTarget> {
    let text: String = value.extract().map_err(|_| {
        value_error(
            "expected the text of a target - a quoted URL or a catalog path - got another object",
        )
    })?;
    CoreTarget::parse(&text).map_err(value_error)
}

/// Read one source: a target's text, or a plan to read from.
fn source_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreSource> {
    if let Ok(plan) = value.extract::<PyRef<'_, PyPlan>>() {
        return Ok(CoreSource::Plan(Box::new(plan.inner.clone())));
    }
    Ok(CoreSource::Target(target_from_value(value)?))
}

/// Read one ordering key and its optional direction and null placement.
fn ordering_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreOrdering> {
    let (term, direction, nulls) =
        if let Ok(parts) = value.extract::<(Bound<'_, PyAny>, String, String)>() {
            (parts.0, Some(parts.1), Some(parts.2))
        } else if let Ok(parts) = value.extract::<(Bound<'_, PyAny>, String)>() {
            (parts.0, Some(parts.1), None)
        } else {
            (value.clone(), None, None)
        };
    let term = term_from_value(&term)?;
    let direction = direction.map(|word| word.to_ascii_lowercase());
    let mut key = match direction.as_deref().unwrap_or("asc") {
        "asc" | "ascending" => CoreOrdering::asc(term),
        "desc" | "descending" => CoreOrdering::desc(term),
        other => {
            return Err(value_error(format!(
                "unknown sort direction {other:?}; expected \"asc\" or \"desc\""
            )));
        }
    };
    let nulls = nulls.map(|word| word.to_ascii_lowercase());
    match nulls.as_deref().unwrap_or("last") {
        "last" => {}
        "first" => key = key.nulls_first(true),
        other => {
            return Err(value_error(format!(
                "unknown null placement {other:?}; expected \"first\" or \"last\""
            )));
        }
    }
    Ok(key)
}

/// One ordering key as the `(term, direction, nulls)` triple Python reads.
fn ordering_parts(key: &CoreOrdering) -> (PyTerm, &'static str, &'static str) {
    (
        PyTerm::from_core(key.term().clone()),
        if key.is_descending() { "desc" } else { "asc" },
        if key.is_nulls_first() {
            "first"
        } else {
            "last"
        },
    )
}

/// Read a write verb in any spelling the grammar reads.
fn verb_from_str(value: &str) -> PyResult<CoreVerb> {
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
            return Err(value_error(format!(
                "unknown write verb {value:?}; expected \"insert into\", \"insert overwrite\", \
                 \"upsert into\", \"delete from\", or one of their aliases"
            )));
        }
    })
}

/// Read one path step: a struct child, a list position, or a map key.
fn segment_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreSegment> {
    if value.is_instance_of::<PyString>() {
        return Ok(CoreSegment::field(value.extract::<String>()?));
    }
    if let Ok(index) = value.extract::<i64>() {
        return Ok(CoreSegment::index(index));
    }
    CoreSegment::key(crate::scalar::from_py(value)?).map_err(value_error)
}

/// Read one comparison from the grammar's own spelling of it.
fn comparison_from_str(value: &str) -> PyResult<CoreComparison> {
    CoreComparison::ALL
        .into_iter()
        .find(|comparison| comparison.as_str().eq_ignore_ascii_case(value))
        .ok_or_else(|| {
            value_error(format!(
                "unknown comparison {value:?}; expected one of {}",
                CoreComparison::ALL.map(CoreComparison::as_str).join(", ")
            ))
        })
}

/// Read one holder attribute, `partition` taking the column it reads.
fn attribute_from_name(name: &str, key: Option<&str>) -> PyResult<Attribute> {
    match key {
        Some(key) if name.eq_ignore_ascii_case("partition") => Ok(Attribute::Partition(key.into())),
        Some(_) => Err(value_error(
            "expected a key only for the partition attribute",
        )),
        None => Attribute::from_name(name).ok_or_else(|| {
            value_error(format!(
                "expected one of the holder attributes {}, got {name:?}",
                Attribute::vocabulary()
            ))
        }),
    }
}

/// Read native rows: a mapping is a named record, anything else crosses
/// through the shared value inference.
fn rows_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<Scalar>> {
    let mut rows = Vec::new();
    for row in value.try_iter()? {
        let row = row?;
        if let Ok(mapping) = row.cast::<PyDict>() {
            let mut entries = Vec::with_capacity(mapping.len());
            for (name, value) in mapping.iter() {
                entries.push((name.extract::<String>()?, crate::scalar::from_py(&value)?));
            }
            rows.push(Scalar::from_struct(entries).map_err(value_error)?);
        } else {
            rows.push(crate::scalar::from_py(&row)?);
        }
    }
    Ok(rows)
}

/// Read one row the way a bound term reads it: a sequence in schema order or
/// a mapping from column name to value.
fn row_value(schema: &CoreField, row: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    if let Ok(mapping) = row.cast::<PyDict>() {
        let mut values = Vec::with_capacity(schema.field_len());
        for field in schema.fields() {
            let held = mapping.get_item(field.name())?;
            values.push(match held {
                Some(held) => crate::scalar::from_py(&held)?,
                None => Scalar::Null,
            });
        }
        return Ok(Scalar::from_sequence(values));
    }
    if row.is_instance_of::<PyList>() || row.is_instance_of::<PyTuple>() {
        return crate::scalar::from_py(row);
    }
    Err(value_error(
        "expected a sequence of column values in schema order, or a mapping of column to value",
    ))
}

/// Run an Arrow value through one of the three arrow surfaces, keeping its
/// holder: a batch answers a batch, a table a table, and anything else is
/// read as a reader.
fn apply_arrow_scalar<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    batch: impl FnOnce(&arrow_array::RecordBatch) -> yggdryl::Result<arrow_array::RecordBatch>,
    reader: impl FnOnce(yggdryl::arrow::BatchReader) -> yggdryl::Result<yggdryl::arrow::BatchReader>,
) -> PyResult<Bound<'py, PyAny>> {
    let pyarrow = py.import("pyarrow")?;
    if value.is_instance(&pyarrow.getattr("RecordBatch")?)? {
        let source = record_batch_from_value(value)?;
        let applied = batch(&source).map_err(value_error)?;
        return batch_to_pyarrow(py, applied);
    }
    if value.is_instance(&pyarrow.getattr("Table")?)? {
        let source = batch_reader_from_arrow_table(value)?;
        let applied = reader(source).map_err(value_error)?;
        return batch_reader_to_pyarrow(py, applied)?.call_method0("read_all");
    }
    let source = batch_reader_from_arrow_reader(value)?;
    let applied = reader(source).map_err(value_error)?;
    batch_reader_to_pyarrow(py, applied)
}

/// Compare two core values under one Python comparison operator.
fn rich_compare<T: Ord>(
    this: &T,
    other: &Bound<'_, PyAny>,
    extracted: Option<T>,
    operation: CompareOp,
) -> PyResult<Py<PyAny>> {
    let Some(other_value) = extracted else {
        return Ok(other.py().NotImplemented());
    };
    Ok(crate::compare(this.cmp(&other_value), operation)
        .into_pyobject(other.py())?
        .to_owned()
        .into_any()
        .unbind())
}

// ---------------------------------------------------------------------------
// Term
// ---------------------------------------------------------------------------

/// A recursive, typed tree: a column, a constant, a comparison, a function.
#[pyclass(name = "Term", module = "yggdryl._native", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTerm {
    pub(crate) inner: CoreTerm,
}

impl PyTerm {
    pub(crate) const fn from_core(inner: CoreTerm) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTerm {
    /// Parse one term from its canonical text.
    #[new]
    fn new(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: text.parse().map_err(value_error)?,
        })
    }

    /// Parse one term from its canonical text.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Self::new(text)
    }

    /// Read one term from its structural JSON document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self {
            inner: CoreTerm::from_json(document).map_err(value_error)?,
        })
    }

    /// Name one top-level column.
    #[staticmethod]
    fn column(name: &str) -> Self {
        Self::from_core(CoreTerm::column(name))
    }

    /// Hold one constant.
    #[staticmethod]
    fn literal(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreTerm::literal(crate::scalar::from_py(
            value,
        )?)))
    }

    /// Hold a constant in an explicitly named datatype.
    ///
    /// `literal` infers the datatype from the value; this declares it, and
    /// the value is checked against it.
    #[staticmethod]
    fn typed_literal(dtype: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreTerm::typed_literal(
            core_dtype_from_value(dtype)?,
            crate::scalar::from_py(value)?,
        )
        .map(Self::from_core)
        .map_err(value_error)
    }

    /// Name one holder attribute, such as `size` or `partition` with a column.
    #[staticmethod]
    #[pyo3(signature = (name, key = None))]
    fn attribute(name: &str, key: Option<&str>) -> PyResult<Self> {
        Ok(Self::from_core(CoreTerm::attribute(attribute_from_name(
            name, key,
        )?)))
    }

    /// Name one late-bound value.
    #[staticmethod]
    fn parameter(name: &str) -> Self {
        Self::from_core(CoreTerm::parameter(name))
    }

    /// The term that is true for every row.
    #[staticmethod]
    fn always_true() -> Self {
        Self::from_core(CoreTerm::always_true())
    }

    /// The term that is true for no row.
    #[staticmethod]
    fn always_false() -> Self {
        Self::from_core(CoreTerm::always_false())
    }

    /// Conjoin many operands into one flattened node; empty is true.
    #[staticmethod]
    fn all(operands: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreTerm::all(operands_from_value(
            operands,
        )?)))
    }

    /// Disjoin many operands into one flattened node; empty is false.
    #[staticmethod]
    fn any(operands: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreTerm::any(operands_from_value(
            operands,
        )?)))
    }

    /// Call one function: a grammar function by name, or a registered
    /// user-defined function by its qualified `namespace.name`.
    #[staticmethod]
    fn call(function: &str, arguments: &Bound<'_, PyAny>) -> PyResult<Self> {
        let function = CoreFunction::resolve(function).map_err(value_error)?;
        Ok(Self::from_core(CoreTerm::call(
            function,
            operands_from_value(arguments)?,
        )))
    }

    /// Build a searched conditional from `(when, then)` pairs, tried in order.
    ///
    /// An absent `otherwise` means null, which is what SQL's `CASE` means.
    #[staticmethod]
    #[pyo3(signature = (branches, otherwise = None))]
    fn case(branches: &Bound<'_, PyAny>, otherwise: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut pairs = Vec::new();
        for branch in branches.try_iter()? {
            let branch = branch?;
            let (when, then): (Bound<'_, PyAny>, Bound<'_, PyAny>) = branch.extract()?;
            pairs.push((term_from_value(&when)?, term_from_value(&then)?));
        }
        let otherwise = otherwise.map(term_from_value).transpose()?;
        Ok(Self::from_core(CoreTerm::case(pairs, otherwise)))
    }

    /// Every top-level column this term reads, in first-seen order.
    fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this term reads, in first-seen order.
    fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this term names, in first-seen order.
    fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// The top-level `and` operands, flattened.
    fn conjuncts(&self) -> Vec<Self> {
        self.inner
            .conjuncts()
            .into_iter()
            .map(Self::from_core)
            .collect()
    }

    /// How deep this term nests, counting itself as one level.
    fn depth(&self) -> usize {
        self.inner.depth()
    }

    /// The number of nodes this term holds.
    fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Refuse a term past the depth or node limit, before a walk.
    fn check_budget(&self) -> PyResult<()> {
        self.inner.check_budget().map_err(value_error)
    }

    /// This term with the same answer and fewer nodes.
    fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// The tree of this term, drawn one node per line.
    fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this term as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    /// Resolve this term against a struct root schema.
    #[pyo3(signature = (schema, parameters = None))]
    fn bind(
        &self,
        schema: &Bound<'_, PyAny>,
        parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyBound> {
        let schema = core_field_from_value(schema)?;
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(PyBound {
            inner: self
                .inner
                .bind_with(&schema, &borrowed)
                .map_err(value_error)?,
        })
    }

    /// The output field this term produces against a schema.
    fn field(&self, schema: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let schema = core_field_from_value(schema)?;
        Ok(PyField::from_inner(
            self.inner.field(&schema).map_err(value_error)?,
        ))
    }

    /// Build `self + other` without evaluating either side.
    fn add(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Add, term_from_value(other)?),
        ))
    }

    /// Build `self - other` without evaluating either side.
    fn subtract(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Sub, term_from_value(other)?),
        ))
    }

    /// Build `self * other` without evaluating either side.
    fn multiply(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Mul, term_from_value(other)?),
        ))
    }

    /// Build `self / other` without evaluating either side.
    fn divide(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Div, term_from_value(other)?),
        ))
    }

    /// Build `self % other` without evaluating either side.
    fn remainder(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .arithmetic(Operator::Rem, term_from_value(other)?),
        ))
    }

    /// Build `-self`, folding a numeric literal in the native core.
    fn negate(&self) -> Self {
        Self::from_core(self.inner.clone().neg())
    }

    /// Compare this term with another under a named comparison.
    ///
    /// The vocabulary is the grammar's own - `=`, `<>`, `<`, `<=`, `>`, `>=`,
    /// `is distinct from`, `is not distinct from` - and `eq` through `ge` are
    /// the six spellings that name one each.
    fn compare(&self, comparison: &str, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        let comparison = comparison_from_str(comparison)?;
        Ok(Self::from_core(
            self.inner
                .clone()
                .compare(comparison, term_from_value(other)?),
        ))
    }

    /// `self = other`.
    fn eq(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().eq(term_from_value(other)?),
        ))
    }

    /// `self <> other`.
    fn ne(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().ne(term_from_value(other)?),
        ))
    }

    /// `self < other`.
    fn lt(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().lt(term_from_value(other)?),
        ))
    }

    /// `self <= other`.
    fn le(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().le(term_from_value(other)?),
        ))
    }

    /// `self > other`.
    fn gt(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().gt(term_from_value(other)?),
        ))
    }

    /// `self >= other`.
    fn ge(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().ge(term_from_value(other)?),
        ))
    }

    /// `self in (...)`, over the values or terms given.
    fn is_in(&self, values: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().is_in(operands_from_value(values)?),
        ))
    }

    /// `self between low and high`, inclusive at both ends.
    fn between(&self, low: &Bound<'_, PyAny>, high: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .between(term_from_value(low)?, term_from_value(high)?),
        ))
    }

    /// `self is null`, which answers true or false and never unknown.
    fn is_null(&self) -> Self {
        Self::from_core(self.inner.clone().is_null())
    }

    /// `self is not null`.
    fn is_not_null(&self) -> Self {
        Self::from_core(self.inner.clone().is_not_null())
    }

    /// `self like pattern`, with SQL's `%` and `_` wildcards.
    fn like(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().like(term_from_value(pattern)?),
        ))
    }

    /// `self ilike pattern`, folding ASCII case.
    fn ilike(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().ilike(term_from_value(pattern)?),
        ))
    }

    /// `self glob pattern`, under the `.gitignore` path rule.
    fn glob(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().glob(term_from_value(pattern)?),
        ))
    }

    /// Cast this term to a datatype, refusing what it cannot hold.
    fn cast(&self, dtype: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().cast(core_dtype_from_value(dtype)?),
        ))
    }

    /// Cast this term to a datatype, nulling what it cannot hold.
    fn try_cast(&self, dtype: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().try_cast(core_dtype_from_value(dtype)?),
        ))
    }

    /// Read a struct child by name, resolved case-insensitively.
    fn child(&self, name: &str) -> Self {
        Self::from_core(self.inner.clone().child(name))
    }

    /// Read a list element by position, counting back from the end when
    /// negative.
    fn at(&self, index: i64) -> Self {
        Self::from_core(self.inner.clone().at(index))
    }

    /// Read a run of list elements, `start` inclusive and `end` exclusive;
    /// either bound counts back from the end when negative, and an absent
    /// one is the list's own edge.
    #[pyo3(signature = (start = None, end = None))]
    fn slice(&self, start: Option<i64>, end: Option<i64>) -> Self {
        Self::from_core(self.inner.clone().slice(start, end))
    }

    /// Append a whole path of steps at once.
    ///
    /// A `str` step is a struct child, an `int` is a list position, and any
    /// other value is a map key, typed through the shared value rules.
    fn path(&self, segments: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut steps = Vec::new();
        for step in segments.try_iter()? {
            steps.push(segment_from_value(&step?)?);
        }
        Ok(Self::from_core(
            self.inner.clone().path(steps).map_err(value_error)?,
        ))
    }

    /// Whether this node is a constant.
    #[getter]
    fn is_literal(&self) -> bool {
        self.inner.is_literal()
    }

    /// The constant this node holds, as `(value, dtype)`, or `None`.
    fn as_literal(&self) -> Option<(PyScalar, PyDataType)> {
        self.inner.as_literal().map(|typed| {
            (
                PyScalar::from_inner(typed.value().clone()),
                PyDataType::from_inner(typed.dtype().clone()),
            )
        })
    }

    /// The column name this node reads directly, or `None`.
    fn as_column(&self) -> Option<&str> {
        self.inner.as_column()
    }

    /// Whether this node is the constant true; an empty `all()` counts.
    #[getter]
    fn is_always_true(&self) -> bool {
        self.inner.is_always_true()
    }

    /// Whether this node is the constant false; an empty `any()` counts.
    #[getter]
    fn is_always_false(&self) -> bool {
        self.inner.is_always_false()
    }

    /// Whether this term reads any holder attribute.
    #[getter]
    fn has_attributes(&self) -> bool {
        self.inner.has_attributes()
    }

    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.add(other)
    }

    fn __radd__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            term_from_value(other)?.arithmetic(Operator::Add, self.inner.clone()),
        ))
    }

    fn __sub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.subtract(other)
    }

    fn __rsub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            term_from_value(other)?.arithmetic(Operator::Sub, self.inner.clone()),
        ))
    }

    fn __mul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.multiply(other)
    }

    fn __rmul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            term_from_value(other)?.arithmetic(Operator::Mul, self.inner.clone()),
        ))
    }

    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.divide(other)
    }

    fn __rtruediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            term_from_value(other)?.arithmetic(Operator::Div, self.inner.clone()),
        ))
    }

    fn __mod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.remainder(other)
    }

    fn __rmod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            term_from_value(other)?.arithmetic(Operator::Rem, self.inner.clone()),
        ))
    }

    fn __neg__(&self) -> Self {
        self.negate()
    }

    /// Build `self and other`.
    fn __and__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().and(term_from_value(other)?),
        ))
    }

    /// Build `self or other`.
    fn __or__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().or(term_from_value(other)?),
        ))
    }

    /// Build `not self`.
    fn __invert__(&self) -> Self {
        Self::from_core(self.inner.clone().not())
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Term({:?})", self.inner.to_string())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let extracted = other
            .extract::<PyRef<'_, Self>>()
            .ok()
            .map(|other| other.inner.clone());
        rich_compare(&self.inner, other, extracted, operation)
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Bound
// ---------------------------------------------------------------------------

/// One term resolved against one schema, ready to answer.
#[pyclass(name = "Bound", module = "yggdryl._native", frozen)]
pub(crate) struct PyBound {
    pub(crate) inner: CoreBound,
}

#[pymethods]
impl PyBound {
    // A bound term is an executable plan over a schema, not a canonical
    // public value. The core supplies no complete identity for it.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The term as it stands after substitution, folding, and ordering.
    #[getter]
    fn term(&self) -> PyTerm {
        PyTerm::from_core(self.inner.term().clone())
    }

    /// The output field this term produces.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.inner.field().clone())
    }

    /// The struct root this term was bound against.
    #[getter]
    fn schema(&self) -> PyField {
        PyField::from_inner(self.inner.schema().clone())
    }

    /// Return whether this term answers a boolean.
    #[getter]
    fn is_predicate(&self) -> bool {
        self.inner.is_predicate()
    }

    /// The schema column names this term reads, in index order.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner.column_names()
    }

    /// The schema column indices this term reads, ascending.
    ///
    /// This is projection pushdown: a reader decodes these and no others.
    #[getter]
    fn column_indices(&self) -> Vec<usize> {
        self.inner.column_indices()
    }

    /// Return whether answering this term requires reading rows.
    #[getter]
    fn reads_rows(&self) -> bool {
        self.inner.reads_rows()
    }

    /// Evaluate this term for one row.
    ///
    /// The row is a sequence of column values in schema order, or a mapping
    /// from column name to value.
    fn eval(&self, py: Python<'_>, row: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .eval(&row_value(self.inner.schema(), row)?)
            .map_err(value_error)?;
        crate::scalar::as_py(py, &value)
    }

    /// Answer this predicate for one row, reading unknown as "no".
    fn matches(&self, row: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.inner
            .matches(&row_value(self.inner.schema(), row)?)
            .map_err(value_error)
    }

    /// Split this predicate into the part a partition layout answers and the rest.
    fn partition_split(&self) -> (PyFilter, PyFilter) {
        let residual = self.inner.partition_split();
        (
            PyFilter::from_core(residual.answerable().clone()),
            PyFilter::from_core(residual.remaining().clone()),
        )
    }

    /// Run the vectorized tier over one batch, answering one value per row.
    fn evaluate_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let values = self.inner.evaluate(&batch).map_err(value_error)?;
        arrow_array_to_pyarrow(py, &values, None)
    }

    /// The selection this predicate makes over one batch, as a mask.
    ///
    /// The mask is null-free: an unknown answer folds to false, which is what
    /// keeps a filter from keeping a row it cannot judge.
    fn filter_mask_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let mask = self.inner.filter_mask(&batch).map_err(value_error)?;
        let array: arrow_array::ArrayRef = std::sync::Arc::new(mask);
        arrow_array_to_pyarrow(py, &array, None)
    }

    /// Keep the rows of one batch this predicate answers true for.
    ///
    /// A mask that keeps every row hands the caller's own batch back.
    fn filter_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let source = record_batch_from_value(batch)?;
        let filtered = self.inner.filter(&source).map_err(value_error)?;
        if filtered.num_rows() == source.num_rows() {
            return Ok(batch.clone());
        }
        batch_to_pyarrow(py, filtered)
    }

    /// Filter every batch a reader yields, lazily.
    fn filter_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        batch_reader_to_pyarrow(py, self.inner.clone().filter_reader(reader))
    }

    /// Whether a container's statistics leave any row that could match.
    ///
    /// False is a proof: the container can be skipped without reading it.
    /// True only means the statistics do not rule it out.
    fn statistics_prune(&self, bounds: &PyBounds) -> bool {
        self.inner.statistics_prune(&bounds.inner)
    }

    /// What a container's statistics settle, three-valued.
    ///
    /// `True` every row matches, `False` none does, `None` the statistics do
    /// not say and the rows have to be read.
    fn statistics_certainty(&self, bounds: &PyBounds) -> Option<bool> {
        self.inner.statistics_certainty(&bounds.inner)
    }

    /// The plan this term runs, drawn one node per line with its datatype
    /// and cost.
    fn explain(&self) -> String {
        self.inner.explain()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Bound({:?})", self.inner.to_string())
    }
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

/// A `where` clause: one predicate over rows.
#[pyclass(
    name = "Filter",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyFilter {
    pub(crate) inner: CoreFilter,
}

impl PyFilter {
    pub(crate) const fn from_core(inner: CoreFilter) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyFilter {
    /// Read one filter from a predicate's text, a `Term`, or another filter;
    /// the `where` keyword is optional.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(filter_from_value(value)?))
    }

    /// Parse one filter from its canonical text.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Ok(Self::from_core(text.parse().map_err(value_error)?))
    }

    /// Read one filter from its structural JSON document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self::from_core(
            CoreFilter::from_json(document).map_err(value_error)?,
        ))
    }

    /// The filter that keeps every row.
    #[staticmethod]
    fn always_true() -> Self {
        Self::from_core(CoreFilter::always_true())
    }

    /// The filter that keeps no row.
    #[staticmethod]
    fn always_false() -> Self {
        Self::from_core(CoreFilter::always_false())
    }

    /// Conjoin many filters into one; empty keeps every row.
    #[staticmethod]
    fn all(operands: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut filters = Vec::new();
        for operand in operands.try_iter()? {
            filters.push(filter_from_value(&operand?)?);
        }
        Ok(Self::from_core(CoreFilter::all(filters)))
    }

    /// Disjoin many filters into one; empty keeps no row.
    #[staticmethod]
    fn any(operands: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut filters = Vec::new();
        for operand in operands.try_iter()? {
            filters.push(filter_from_value(&operand?)?);
        }
        Ok(Self::from_core(CoreFilter::any(filters)))
    }

    /// The predicate this filter keeps rows by.
    #[getter]
    fn term(&self) -> PyTerm {
        PyTerm::from_core(self.inner.term().clone())
    }

    /// Whether this filter keeps every row.
    #[getter]
    fn is_always_true(&self) -> bool {
        self.inner.is_always_true()
    }

    /// Whether this filter keeps no row.
    #[getter]
    fn is_always_false(&self) -> bool {
        self.inner.is_always_false()
    }

    /// Whether this filter reads any holder attribute.
    #[getter]
    fn has_attributes(&self) -> bool {
        self.inner.has_attributes()
    }

    /// The top-level `and` operands, each its own filter.
    fn conjuncts(&self) -> Vec<Self> {
        self.inner
            .conjuncts()
            .into_iter()
            .map(Self::from_core)
            .collect()
    }

    /// Every top-level column this filter reads, in first-seen order.
    fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this filter reads, in first-seen order.
    fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this filter names, in first-seen order.
    fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// This filter with the same answer and fewer nodes.
    fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse a filter past the depth or node limit, before a walk.
    fn check_budget(&self) -> PyResult<()> {
        self.inner.check_budget().map_err(value_error)
    }

    /// Resolve this filter against a struct root schema, as a predicate.
    #[pyo3(signature = (schema, parameters = None))]
    fn bind(
        &self,
        schema: &Bound<'_, PyAny>,
        parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyBound> {
        let schema = core_field_from_value(schema)?;
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(PyBound {
            inner: self
                .inner
                .bind_with(&schema, &borrowed)
                .map_err(value_error)?,
        })
    }

    /// The struct root the kept rows have: the schema itself, once the
    /// predicate types against it.
    fn apply_field(&self, root: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let root = core_field_from_value(root)?;
        Ok(PyField::from_inner(
            self.inner.apply_field(&root).map_err(value_error)?,
        ))
    }

    /// Keep the rows of one `pyarrow.RecordBatch` this filter answers true
    /// for; a filter that keeps everything hands the batch back.
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let source = record_batch_from_value(batch)?;
        let kept = self.inner.apply_arrow_batch(&source).map_err(value_error)?;
        if kept.num_rows() == source.num_rows() {
            return Ok(batch.clone());
        }
        batch_to_pyarrow(py, kept)
    }

    /// Filter every batch a `pyarrow.RecordBatchReader` yields, lazily.
    fn apply_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        let kept = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, kept)
    }

    /// Keep the rows of one `pyarrow.Table`, batch by batch.
    fn apply_arrow_table<'py>(
        &self,
        py: Python<'py>,
        table: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_table(table)?;
        let kept = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, kept)?.call_method0("read_all")
    }

    /// Keep the rows of one `pyarrow.StructArray` this filter answers true for.
    fn apply_arrow_array<'py>(
        &self,
        py: Python<'py>,
        array: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let array = arrow_array_from_pyarrow(array)?;
        let kept = self.inner.apply_arrow_array(&array).map_err(value_error)?;
        arrow_array_to_pyarrow(py, &kept, None)
    }

    /// Run this filter over a batch, a table, or a reader, keeping its kind.
    fn apply_arrow<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        apply_arrow_scalar(
            py,
            value,
            |batch| self.inner.apply_arrow_batch(batch),
            |reader| self.inner.apply_arrow_reader(reader),
        )
    }

    /// The native rows this filter keeps, bound once against `schema` or the
    /// schema the first record implies.
    #[pyo3(signature = (rows, schema = None))]
    fn apply_records(
        &self,
        rows: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRecords> {
        let schema = schema.map(core_field_from_value).transpose()?;
        let rows = rows_from_value(rows)?;
        Ok(PyRecords::from_core(
            self.inner
                .apply_records(schema.as_ref(), rows)
                .map_err(value_error)?,
        ))
    }

    /// The tree of this filter, drawn one node per line.
    fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this filter as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    /// Build `self and other`.
    fn __and__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().and(filter_from_value(other)?),
        ))
    }

    /// Build `self or other`.
    fn __or__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().or(filter_from_value(other)?),
        ))
    }

    /// Build `not self`.
    fn __invert__(&self) -> Self {
        Self::from_core(self.inner.clone().not())
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Filter({:?})", self.inner.to_string())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let extracted = other
            .extract::<PyRef<'_, Self>>()
            .ok()
            .map(|other| other.inner.clone());
        rich_compare(&self.inner, other, extracted, operation)
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Selector
// ---------------------------------------------------------------------------

/// A `select` clause: the columns published, computed, declared or excluded.
#[pyclass(
    name = "Selector",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySelector {
    pub(crate) inner: CoreSelector,
}

impl PySelector {
    pub(crate) const fn from_core(inner: CoreSelector) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PySelector {
    /// Read one selector from a projection list's text, a `Term`, another
    /// selector, or an iterable of projections; the `select` keyword is
    /// optional.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(selector_from_value(value)?))
    }

    /// Parse one selector from its canonical text.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Ok(Self::from_core(text.parse().map_err(value_error)?))
    }

    /// Read one selector from its structural JSON document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self::from_core(
            CoreSelector::from_json(document).map_err(value_error)?,
        ))
    }

    /// `select *`: every column, unchanged.
    #[staticmethod]
    fn all() -> Self {
        Self::from_core(CoreSelector::all())
    }

    /// `select * exclude (...)`: every column but the named ones.
    #[staticmethod]
    fn all_except(names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let names = crate::enums::strings_from_iterable(names, "names")?;
        Ok(Self::from_core(CoreSelector::all_except(names)))
    }

    /// The named columns, in that order, unchanged.
    #[staticmethod]
    fn from_columns(names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let names = crate::enums::strings_from_iterable(names, "names")?;
        Ok(Self::from_core(CoreSelector::from_columns(names)))
    }

    /// The projection list, each a term, text, or a `(term, alias)` pair.
    #[staticmethod]
    fn select(projections: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut selected = Vec::new();
        for projection in projections.try_iter()? {
            selected.push(projection_from_value(&projection?)?);
        }
        Ok(Self::from_core(CoreSelector::new(selected)))
    }

    /// The selector a struct root declares: one column per child, with its
    /// datatype, nullability, metadata, and any `TRANSFORM:` it carries.
    #[staticmethod]
    fn from_field(field: &Bound<'_, PyAny>) -> PyResult<Self> {
        let field = core_field_from_value(field)?;
        Ok(Self::from_core(CoreSelector::from_field(&field)))
    }

    /// Each projection, as its canonical text.
    #[getter]
    fn projections(&self) -> Vec<String> {
        self.inner
            .projections()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The names this selector publishes, in output order; empty for `*`.
    #[getter]
    fn names(&self) -> Vec<String> {
        self.inner
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    }

    /// The column names `select * exclude (...)` drops.
    #[getter]
    fn excluded(&self) -> Vec<String> {
        self.inner
            .excluded()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Whether this is `select *` with nothing excluded.
    #[getter]
    fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Whether every projection is a bare column.
    #[getter]
    fn is_columns(&self) -> bool {
        self.inner.is_columns()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Every top-level column this selector reads, in first-seen order.
    fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this selector reads, in first-seen order.
    fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this selector names, in first-seen order.
    fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// This selector with one more projection.
    fn with_projection(&self, projection: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .with_projection(projection_from_value(projection)?),
        ))
    }

    /// This selector without the projections that are these bare columns.
    fn without_columns(&self, names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let names = crate::enums::strings_from_iterable(names, "names")?;
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        Ok(Self::from_core(self.inner.without_columns(&borrowed)))
    }

    /// This selector with every term simplified and every self alias dropped.
    fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse a selector past the depth or node limit, before a walk.
    fn check_budget(&self) -> PyResult<()> {
        self.inner.check_budget().map_err(value_error)
    }

    /// Resolve every projection against one struct root schema.
    #[pyo3(signature = (schema, parameters = None))]
    fn bind(
        &self,
        schema: &Bound<'_, PyAny>,
        parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyBoundSelector> {
        let schema = core_field_from_value(schema)?;
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(PyBoundSelector {
            inner: self
                .inner
                .bind_with(&schema, &borrowed)
                .map_err(value_error)?,
        })
    }

    /// The struct root this selector publishes from `root`.
    fn apply_field(&self, root: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let root = core_field_from_value(root)?;
        Ok(PyField::from_inner(
            self.inner.apply_field(&root).map_err(value_error)?,
        ))
    }

    /// The struct root this selector publishes from `root`, carrying the
    /// selector itself as each column's `TRANSFORM:` declaration, so a
    /// `Selector.from_field` of the answer is this selector again.
    #[allow(clippy::wrong_self_convention)]
    fn into_field(&self, root: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let root = core_field_from_value(root)?;
        Ok(PyField::from_inner(
            self.inner.into_field(&root).map_err(value_error)?,
        ))
    }

    /// The columns this selector publishes from one `pyarrow.RecordBatch`.
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let projected = self.inner.apply_arrow_batch(&batch).map_err(value_error)?;
        batch_to_pyarrow(py, projected)
    }

    /// Project every batch a `pyarrow.RecordBatchReader` yields, lazily.
    fn apply_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        let projected = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, projected)
    }

    /// Project one `pyarrow.Table`, batch by batch.
    fn apply_arrow_table<'py>(
        &self,
        py: Python<'py>,
        table: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_table(table)?;
        let projected = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, projected)?.call_method0("read_all")
    }

    /// The struct array this selector publishes from one `pyarrow.StructArray`.
    fn apply_arrow_array<'py>(
        &self,
        py: Python<'py>,
        array: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let array = arrow_array_from_pyarrow(array)?;
        let projected = self.inner.apply_arrow_array(&array).map_err(value_error)?;
        arrow_array_to_pyarrow(py, &projected, None)
    }

    /// Run this selector over a batch, a table, or a reader, keeping its kind.
    fn apply_arrow<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        apply_arrow_scalar(
            py,
            value,
            |batch| self.inner.apply_arrow_batch(batch),
            |reader| self.inner.apply_arrow_reader(reader),
        )
    }

    /// The rows this selector publishes from native records, bound once
    /// against `schema` or the schema the first record implies.
    #[pyo3(signature = (rows, schema = None))]
    fn apply_records(
        &self,
        rows: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRecords> {
        let schema = schema.map(core_field_from_value).transpose()?;
        let rows = rows_from_value(rows)?;
        Ok(PyRecords::from_core(
            self.inner
                .apply_records(schema.as_ref(), rows)
                .map_err(value_error)?,
        ))
    }

    /// The tree of this selector, one branch per projection.
    fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this selector as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Selector({:?})", self.inner.to_string())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let extracted = other
            .extract::<PyRef<'_, Self>>()
            .ok()
            .map(|other| other.inner.clone());
        rich_compare(&self.inner, other, extracted, operation)
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// A selector resolved against one schema, ready for batches.
#[pyclass(name = "BoundSelector", module = "yggdryl._native", frozen)]
pub(crate) struct PyBoundSelector {
    pub(crate) inner: CoreBoundSelector,
}

#[pymethods]
impl PyBoundSelector {
    // A bound selector retains executable planning state and has no native
    // complete identity, so an object-identity hash would be misleading.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The struct root the selector reads.
    #[getter]
    fn schema(&self) -> PyField {
        PyField::from_inner(self.inner.schema().clone())
    }

    /// The struct root the selector publishes.
    #[getter]
    fn output(&self) -> PyField {
        PyField::from_inner(self.inner.output().clone())
    }

    /// The bound projections, in output order; empty for `select *`.
    #[getter]
    fn projections(&self) -> Vec<PyBound> {
        self.inner
            .projections()
            .iter()
            .cloned()
            .map(|inner| PyBound { inner })
            .collect()
    }

    /// Whether this selector publishes every input column unchanged.
    #[getter]
    fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Whether applying this selector changes nothing, so a batch is handed
    /// back as it is.
    #[getter]
    fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// The row this selector publishes from one native row, as a mapping.
    fn apply_row(&self, py: Python<'_>, row: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let row = row_value(self.inner.schema(), row)?;
        let published = self.inner.apply_scalar(&row).map_err(value_error)?;
        crate::scalar::as_py_with_field(py, &published, self.inner.output())
    }

    /// Project one `pyarrow.RecordBatch` through the bound plan.
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let projected = self.inner.apply_arrow_batch(&batch).map_err(value_error)?;
        batch_to_pyarrow(py, projected)
    }

    /// Lazily project every batch a `pyarrow.RecordBatchReader` yields.
    fn apply_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        let projected = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, projected)
    }

    /// The bound plan, drawn one node per line with datatypes and costs.
    fn explain(&self) -> String {
        self.inner.explain()
    }

    fn __repr__(&self) -> String {
        format!("BoundSelector({:?})", self.inner.output().to_string())
    }
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// The sections of one read or write: what it creates, writes, selects,
/// reads from, keeps, orders, and how many rows.
#[pyclass(name = "Plan", module = "yggdryl._native", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyPlan {
    pub(crate) inner: CorePlan,
}

impl PyPlan {
    pub(crate) const fn from_core(inner: CorePlan) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyPlan {
    /// Read one plan from its text, a clause, an `Expression`, or a `Field`
    /// (the `create` section it declares).
    #[new]
    #[pyo3(signature = (value = None))]
    fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self::from_core(match value {
            Some(value) => plan_from_value(value)?,
            None => CorePlan::new(),
        }))
    }

    /// Parse one plan from its canonical text.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Ok(Self::from_core(text.parse().map_err(value_error)?))
    }

    /// Read one plan from its structural JSON document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self::from_core(
            CorePlan::from_json(document).map_err(value_error)?,
        ))
    }

    /// The plan a struct root is: a `create` section declaring it.
    #[staticmethod]
    fn from_field(field: &Bound<'_, PyAny>) -> PyResult<Self> {
        let field = core_field_from_value(field)?;
        Ok(Self::from_core(CorePlan::from_field(&field)))
    }

    /// The `create` section's target, spelled as the grammar spells it.
    #[getter]
    fn create_target(&self) -> Option<String> {
        self.inner.create_target().map(ToString::to_string)
    }

    /// The `create` section's schema, when the plan has one.
    #[getter]
    fn schema(&self) -> Option<PySelector> {
        self.inner.schema().cloned().map(PySelector::from_core)
    }

    /// The name of the struct root this plan declares or reads.
    #[getter]
    fn root_name(&self) -> String {
        self.inner.root_name().to_owned()
    }

    /// The metadata the `create` section declares on the root, `with (...)`.
    #[getter]
    fn root_metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let metadata = PyDict::new(py);
        for (key, value) in self.inner.root_metadata() {
            metadata.set_item(key, value)?;
        }
        Ok(metadata)
    }

    /// The write verb - `insert into`, `insert overwrite`, `upsert into`,
    /// `delete from` - or `None` for a read.
    #[getter]
    fn verb(&self) -> Option<&'static str> {
        self.inner
            .write_section()
            .map(|write| write.verb().as_str())
    }

    /// The write section's target, when the write names one.
    #[getter]
    fn write_target(&self) -> Option<String> {
        self.inner
            .write_section()
            .and_then(CoreWrite::target)
            .map(ToString::to_string)
    }

    /// The keys an upsert matches stored rows on; empty otherwise.
    #[getter]
    fn merge_by(&self) -> PySelector {
        PySelector::from_core(self.inner.merge_by().clone())
    }

    /// The `select` section; `select *` when absent.
    #[getter]
    fn selector(&self) -> PySelector {
        PySelector::from_core(self.inner.selector().clone())
    }

    /// The `from` section, spelled as the grammar spells it, or `None`.
    #[getter]
    fn source(&self) -> Option<String> {
        self.inner.source().map(ToString::to_string)
    }

    /// The plan the `from` section reads, when it is one in parentheses.
    #[getter]
    fn source_plan(&self) -> Option<Self> {
        match self.inner.source() {
            Some(CoreSource::Plan(plan)) => Some(Self::from_core((**plan).clone())),
            _ => None,
        }
    }

    /// The `where` section; always true when absent.
    #[getter]
    fn filter(&self) -> PyFilter {
        PyFilter::from_core(self.inner.filter_section().clone())
    }

    /// The `order by` keys as `(term, "asc" | "desc", "first" | "last")`.
    #[getter]
    fn ordering(&self) -> Vec<(PyTerm, &'static str, &'static str)> {
        self.inner.ordering().iter().map(ordering_parts).collect()
    }

    /// The row limit, when the plan has one.
    #[getter]
    fn limit(&self) -> Option<u64> {
        self.inner.row_limit()
    }

    /// The rows skipped before the first kept, when the plan has an offset.
    #[getter]
    fn offset(&self) -> Option<u64> {
        self.inner.row_offset()
    }

    /// Whether the plan has no section at all.
    #[getter]
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether applying the plan changes nothing: no write, no schema, and
    /// read sections that keep every row and column.
    #[getter]
    fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// This plan with a `create` section: a target, or `None` for the handle
    /// the plan is given to, and the schema it declares.
    #[pyo3(signature = (schema, target = None))]
    fn with_create(
        &self,
        schema: &Bound<'_, PyAny>,
        target: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let target = target.map(target_from_value).transpose()?;
        Ok(Self::from_core(
            self.inner
                .clone()
                .create(target, selector_from_value(schema)?),
        ))
    }

    /// This plan with a write section: a verb in any spelling the grammar
    /// reads, a target or `None` for the handle the plan is given to, and
    /// the keys an upsert matches on.
    #[pyo3(signature = (verb, target = None, merge_by = None))]
    fn with_write(
        &self,
        verb: &str,
        target: Option<&Bound<'_, PyAny>>,
        merge_by: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut write = CoreWrite::new(verb_from_str(verb)?);
        if let Some(target) = target {
            write = write.into(target_from_value(target)?);
        }
        if let Some(merge_by) = merge_by {
            write = write.by(selector_from_value(merge_by)?);
        }
        Ok(Self::from_core(self.inner.clone().write(write)))
    }

    /// This plan with a `select` section, replacing any it carries.
    fn with_select(&self, selector: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut plan = self.inner.clone();
        plan.set_selector(selector_from_value(selector)?);
        Ok(Self::from_core(plan))
    }

    /// This plan reading from a target's text or from another plan.
    fn with_source(&self, source: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().read_from(source_from_value(source)?),
        ))
    }

    /// This plan with a `where` section, replacing any it carries.
    fn with_filter(&self, filter: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut plan = self.inner.clone();
        plan.set_filter(filter_from_value(filter)?);
        Ok(Self::from_core(plan))
    }

    /// This plan with an `order by`, replacing any it carries.
    ///
    /// Each key is a term or its text, or a `(term, direction)` or
    /// `(term, direction, nulls)` tuple; `direction` is `"asc"` or `"desc"`
    /// and `nulls` is `"first"` or `"last"`.
    fn with_ordering(&self, keys: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut ordering = Vec::new();
        for key in keys.try_iter()? {
            ordering.push(ordering_from_value(&key?)?);
        }
        Ok(Self::from_core(self.inner.clone().order_by(ordering)))
    }

    /// This plan with a row limit; `None` lifts it.
    #[pyo3(signature = (limit = None))]
    fn with_limit(&self, limit: Option<u64>) -> Self {
        Self::from_core(self.inner.clone().limit(limit))
    }

    /// This plan skipping `offset` rows before the first kept; `None` lifts it.
    #[pyo3(signature = (offset = None))]
    fn with_offset(&self, offset: Option<u64>) -> Self {
        Self::from_core(self.inner.clone().offset(offset))
    }

    /// This plan matching stored rows on these keys, which makes it an
    /// upsert when it has no write section.
    fn with_merge_by(&self, merge_by: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut plan = self.inner.clone();
        plan.set_merge_by(selector_from_value(merge_by)?);
        Ok(Self::from_core(plan))
    }

    /// The struct root the `create` section declares, or `None`.
    fn field(&self) -> PyResult<Option<PyField>> {
        Ok(self
            .inner
            .field()
            .map_err(value_error)?
            .map(PyField::from_inner))
    }

    /// The struct root this plan publishes from `root`.
    fn field_from(&self, root: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let root = core_field_from_value(root)?;
        Ok(PyField::from_inner(
            self.inner.field_from(&root).map_err(value_error)?,
        ))
    }

    /// Every top-level column this plan reads, in first-seen order.
    fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// The stored columns a read has to decode, or `None` for all of them.
    fn read_columns(&self) -> Option<Vec<String>> {
        self.inner.read_columns()
    }

    /// Every holder attribute this plan reads, in first-seen order.
    fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this plan names, in first-seen order.
    fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// The sections that shape a read - `select`, `where`, `order by`,
    /// `limit`, `offset` - as a plan of their own.
    fn read_sections(&self) -> Self {
        Self::from_core(self.inner.read_sections())
    }

    /// This plan with every section simplified.
    fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse a plan past the depth or node limit, before a walk.
    fn check_budget(&self) -> PyResult<()> {
        self.inner.check_budget().map_err(value_error)
    }

    /// This plan as an expression: a lone `select` is a `Selector`, a lone
    /// `where` a `Filter`, anything else the plan itself.
    #[allow(clippy::wrong_self_convention)]
    fn into_expression(&self) -> PyExpression {
        PyExpression::from_core(self.inner.clone().into_expression())
    }

    /// Run this plan from its own `from` source, as a `pyarrow.RecordBatchReader`.
    ///
    /// A target source is read through its holder with the read sections
    /// pushed down; a nested plan runs first; no source is the empty stream.
    /// A plan with a `create` or write section writes and yields the empty
    /// stream under the schema it wrote.
    fn execute<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.execute().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Apply this plan to one `pyarrow.RecordBatch`, its sections in order.
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let expression = CoreExpression::Plan(Box::new(self.inner.clone()));
        let applied = expression.apply_arrow_batch(&batch).map_err(value_error)?;
        batch_to_pyarrow(py, applied)
    }

    /// Apply this plan to every batch a `pyarrow.RecordBatchReader` yields.
    fn apply_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        let applied = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, applied)
    }

    /// Apply this plan to one `pyarrow.Table`, batch by batch.
    fn apply_arrow_table<'py>(
        &self,
        py: Python<'py>,
        table: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_table(table)?;
        let applied = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, applied)?.call_method0("read_all")
    }

    /// Apply this plan to a batch, a table, or a reader, keeping its kind.
    fn apply_arrow<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let expression = CoreExpression::Plan(Box::new(self.inner.clone()));
        apply_arrow_scalar(
            py,
            value,
            |batch| expression.apply_arrow_batch(batch),
            |reader| self.inner.apply_arrow_reader(reader),
        )
    }

    /// The native rows this plan publishes, run through the vectorized tier.
    #[pyo3(signature = (rows, schema = None))]
    fn apply_records(
        &self,
        rows: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRecords> {
        let schema = schema.map(core_field_from_value).transpose()?;
        let rows = rows_from_value(rows)?;
        let expression = CoreExpression::Plan(Box::new(self.inner.clone()));
        Ok(PyRecords::from_core(
            expression
                .apply_records(schema.as_ref(), rows)
                .map_err(value_error)?,
        ))
    }

    /// The tree of this plan, one branch per section in the order they run.
    fn explain(&self) -> String {
        CoreExpression::Plan(Box::new(self.inner.clone())).explain()
    }

    /// Write this plan as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Plan({:?})", self.inner.to_string())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let extracted = other
            .extract::<PyRef<'_, Self>>()
            .ok()
            .map(|other| other.inner.clone());
        rich_compare(&self.inner, other, extracted, operation)
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Expression
// ---------------------------------------------------------------------------

/// A clause, a plan, or a sequence of plans: whatever one piece of
/// expression text is.
#[pyclass(
    name = "Expression",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyExpression {
    pub(crate) inner: CoreExpression,
}

impl PyExpression {
    pub(crate) const fn from_core(inner: CoreExpression) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyExpression {
    /// Read one expression from its text, a clause, or a plan.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(expression_from_value(value)?))
    }

    /// Parse one expression from its canonical text.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Ok(Self::from_core(text.parse().map_err(value_error)?))
    }

    /// Read one expression from its structural JSON document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self::from_core(
            CoreExpression::from_json(document).map_err(value_error)?,
        ))
    }

    /// The `select` expression of one selector.
    #[staticmethod]
    fn select(selector: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreExpression::Selector(
            selector_from_value(selector)?,
        )))
    }

    /// The `where` expression of one filter.
    #[staticmethod]
    fn filter(filter: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreExpression::Filter(filter_from_value(
            filter,
        )?)))
    }

    /// The expression of one plan: a lone clause collapses into it.
    #[staticmethod]
    fn plan(plan: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(plan_from_value(plan)?.into_expression()))
    }

    /// The sequence of expressions, applied in order; one is itself.
    #[staticmethod]
    fn sequence(steps: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut held = Vec::new();
        for step in steps.try_iter()? {
            held.push(expression_from_value(&step?)?);
        }
        Ok(Self::from_core(CoreExpression::sequence(held)))
    }

    /// Which kind this expression is: `"select"`, `"where"`, `"plan"`, or
    /// `"sequence"`.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            CoreExpression::Selector(_) => "select",
            CoreExpression::Filter(_) => "where",
            CoreExpression::Plan(_) => "plan",
            CoreExpression::Sequence(_) => "sequence",
        }
    }

    /// Whether this is a `select` clause.
    #[getter]
    fn is_selector(&self) -> bool {
        self.inner.is_selector()
    }

    /// Whether this is a `where` clause.
    #[getter]
    fn is_filter(&self) -> bool {
        self.inner.is_filter()
    }

    /// Whether this is a plan with more than one section.
    #[getter]
    fn is_plan(&self) -> bool {
        self.inner.is_plan()
    }

    /// Whether this is a sequence of expressions.
    #[getter]
    fn is_sequence(&self) -> bool {
        self.inner.is_sequence()
    }

    /// Whether applying this expression changes nothing.
    #[getter]
    fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// The selector, when this is a `select` clause.
    fn as_selector(&self) -> Option<PySelector> {
        self.inner.as_selector().cloned().map(PySelector::from_core)
    }

    /// The filter, when this is a `where` clause.
    fn as_filter(&self) -> Option<PyFilter> {
        self.inner.as_filter().cloned().map(PyFilter::from_core)
    }

    /// The plan, when this is one.
    fn as_plan(&self) -> Option<PyPlan> {
        self.inner.as_plan().cloned().map(PyPlan::from_core)
    }

    /// The steps of a sequence; any other expression is its own one step.
    #[getter]
    fn steps(&self) -> Vec<Self> {
        self.inner
            .steps()
            .iter()
            .cloned()
            .map(Self::from_core)
            .collect()
    }

    /// Every top-level column this expression reads, in first-seen order.
    fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }

    /// Every holder attribute this expression reads, in first-seen order.
    fn attributes(&self) -> Vec<String> {
        self.inner
            .attributes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Every parameter this expression names, in first-seen order.
    fn parameters(&self) -> Vec<String> {
        self.inner.parameters()
    }

    /// This expression with every clause simplified.
    fn simplify(&self) -> Self {
        Self::from_core(self.inner.simplify())
    }

    /// Refuse an expression past the depth or node limit, before a walk.
    fn check_budget(&self) -> PyResult<()> {
        self.inner.check_budget().map_err(value_error)
    }

    /// The struct root this expression publishes from `root`.
    fn apply_field(&self, root: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let root = core_field_from_value(root)?;
        Ok(PyField::from_inner(
            self.inner.apply_field(&root).map_err(value_error)?,
        ))
    }

    /// Apply this expression to one `pyarrow.RecordBatch`.
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let applied = self.inner.apply_arrow_batch(&batch).map_err(value_error)?;
        batch_to_pyarrow(py, applied)
    }

    /// Apply this expression to every batch a `pyarrow.RecordBatchReader`
    /// yields, lazily.
    fn apply_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        let applied = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, applied)
    }

    /// Apply this expression to one `pyarrow.Table`, batch by batch.
    fn apply_arrow_table<'py>(
        &self,
        py: Python<'py>,
        table: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_table(table)?;
        let applied = self.inner.apply_arrow_reader(reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, applied)?.call_method0("read_all")
    }

    /// Apply this expression to one `pyarrow.StructArray`.
    fn apply_arrow_array<'py>(
        &self,
        py: Python<'py>,
        array: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let array = arrow_array_from_pyarrow(array)?;
        let applied = self.inner.apply_arrow_array(&array).map_err(value_error)?;
        arrow_array_to_pyarrow(py, &applied, None)
    }

    /// Apply this expression to a batch, a table, or a reader, keeping its kind.
    fn apply_arrow<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        apply_arrow_scalar(
            py,
            value,
            |batch| self.inner.apply_arrow_batch(batch),
            |reader| self.inner.apply_arrow_reader(reader),
        )
    }

    /// The native rows this expression publishes.
    #[pyo3(signature = (rows, schema = None))]
    fn apply_records(
        &self,
        rows: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRecords> {
        let schema = schema.map(core_field_from_value).transpose()?;
        let rows = rows_from_value(rows)?;
        Ok(PyRecords::from_core(
            self.inner
                .apply_records(schema.as_ref(), rows)
                .map_err(value_error)?,
        ))
    }

    /// The tree of this expression, its clause, plan or sequence at the root.
    fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Write this expression as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Expression({:?})", self.inner.to_string())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let extracted = other
            .extract::<PyRef<'_, Self>>()
            .ok()
            .map(|other| other.inner.clone());
        rich_compare(&self.inner, other, extracted, operation)
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Native rows streaming out of an expression, each a mapping under `field`.
#[pyclass(name = "Records", module = "yggdryl._native")]
pub(crate) struct PyRecords {
    field: CoreField,
    // A mutex, not for contention: the row stream is `Send` and not `Sync`,
    // and a class Python may share across threads has to be both.
    rows: std::sync::Mutex<Option<CoreRecords>>,
}

impl PyRecords {
    fn from_core(records: CoreRecords) -> Self {
        Self {
            field: records.field().clone(),
            rows: std::sync::Mutex::new(Some(records)),
        }
    }

    fn next_row(&self) -> PyResult<Option<Scalar>> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| value_error("the record stream was poisoned by an earlier panic"))?;
        match rows.as_mut().and_then(Iterator::next) {
            Some(row) => Ok(Some(row.map_err(value_error)?)),
            None => Ok(None),
        }
    }

    fn take(&self) -> PyResult<CoreRecords> {
        self.rows
            .lock()
            .map_err(|_| value_error("the record stream was poisoned by an earlier panic"))?
            .take()
            .ok_or_else(|| value_error("these records were already consumed"))
    }
}

#[pymethods]
impl PyRecords {
    /// The struct root every row is shaped under.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.field.clone())
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match self.next_row()? {
            Some(row) => Ok(Some(crate::scalar::as_py_with_field(
                py,
                &row,
                &self.field,
            )?)),
            None => Ok(None),
        }
    }

    /// Every remaining row, as a list of mappings.
    fn collect(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let mut rows = Vec::new();
        while let Some(row) = self.__next__(py)? {
            rows.push(row);
        }
        Ok(rows)
    }

    /// The remaining rows as a `pyarrow.RecordBatchReader`, batched lazily.
    #[allow(clippy::wrong_self_convention)]
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.take()?.into_arrow_reader().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// The records a `pyarrow.RecordBatchReader` holds, one row at a time.
    #[staticmethod]
    fn from_arrow_reader(reader: &Bound<'_, PyAny>) -> PyResult<Self> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        Ok(Self::from_core(
            CoreRecords::from_arrow_reader(reader).map_err(value_error)?,
        ))
    }

    fn __repr__(&self) -> String {
        format!("Records({:?})", self.field.to_string())
    }
}

// ---------------------------------------------------------------------------
// Grammar helpers and statistics
// ---------------------------------------------------------------------------

/// Return whether an identifier has to be quoted to survive the grammar.
#[pyfunction]
#[pyo3(name = "expression_needs_quoting")]
pub(crate) fn expression_needs_quoting(name: &str) -> bool {
    yggdryl::expression::needs_quoting(name)
}

/// The grammar's own vocabularies, as the canonical spellings they cross as.
#[pyfunction]
#[pyo3(name = "expression_vocabularies")]
pub(crate) fn expression_vocabularies(py: Python<'_>) -> PyResult<Py<PyDict>> {
    let listing = PyDict::new(py);
    listing.set_item(
        "comparisons",
        CoreComparison::ALL.map(CoreComparison::as_str).to_vec(),
    )?;
    listing.set_item(
        "functions",
        CoreFunction::ALL
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    )?;
    let attributes: Vec<&str> = Attribute::ALL.iter().map(Attribute::as_str).collect();
    listing.set_item("holder_attributes", attributes)?;
    listing.set_item(
        "verbs",
        [
            CoreVerb::Insert,
            CoreVerb::Overwrite,
            CoreVerb::Upsert,
            CoreVerb::Delete,
        ]
        .map(CoreVerb::as_str)
        .to_vec(),
    )?;
    Ok(listing.into())
}

/// One column's recorded statistics as Python reads them.
type ColumnStatistics = (Option<PyScalar>, Option<PyScalar>, Option<u64>);

/// One container's per-column statistics, as a pushdown reads them.
///
/// The same shape whether the numbers came from a Parquet footer, an Iceberg
/// manifest, or a Hive path: what a column holds at least, at most, and how
/// many of its rows are null.
#[pyclass(name = "Bounds", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyBounds {
    inner: CoreBounds,
}

#[pymethods]
impl PyBounds {
    /// Start from a container of a known - or unknown - number of rows.
    #[new]
    #[pyo3(signature = (rows = None))]
    fn new(rows: Option<u64>) -> Self {
        Self {
            inner: CoreBounds::new(rows),
        }
    }

    /// Read the bounds a path's partition values imply under a schema.
    ///
    /// A partition column holds exactly one value in every row it covers, so
    /// its minimum and maximum are that value.
    #[staticmethod]
    fn from_partitions(schema: &Bound<'_, PyAny>, partitions: &Bound<'_, PyAny>) -> PyResult<Self> {
        let schema = core_field_from_value(schema)?;
        let mut pairs = Vec::new();
        for entry in partitions.try_iter()? {
            let (column, value): (String, String) = entry?.extract()?;
            pairs.push((column, value));
        }
        Ok(Self {
            inner: CoreBounds::from_partitions(&schema, &pairs),
        })
    }

    /// Record one column's minimum, maximum, and null count.
    #[pyo3(signature = (name, minimum = None, maximum = None, nulls = None))]
    fn with_column(
        &self,
        name: &str,
        minimum: Option<&Bound<'_, PyAny>>,
        maximum: Option<&Bound<'_, PyAny>>,
        nulls: Option<u64>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().with_column(
                name,
                minimum.map(crate::scalar::from_py).transpose()?,
                maximum.map(crate::scalar::from_py).transpose()?,
                nulls,
            ),
        })
    }

    /// Record one holder attribute's minimum, maximum, and null count;
    /// `partition` takes the column it reads as `key`.
    #[pyo3(signature = (name, minimum = None, maximum = None, nulls = None, key = None))]
    fn with_attribute(
        &self,
        name: &str,
        minimum: Option<&Bound<'_, PyAny>>,
        maximum: Option<&Bound<'_, PyAny>>,
        nulls: Option<u64>,
        key: Option<&str>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().with_attribute(
                attribute_from_name(name, key)?,
                minimum.map(crate::scalar::from_py).transpose()?,
                maximum.map(crate::scalar::from_py).transpose()?,
                nulls,
            ),
        })
    }

    /// One column's recorded statistics, as `(minimum, maximum, nulls)`.
    fn column(&self, name: &str) -> Option<ColumnStatistics> {
        self.inner.column(name).map(column_bounds_parts)
    }

    /// One attribute's recorded statistics, as `(minimum, maximum, nulls)`.
    #[pyo3(signature = (name, key = None))]
    fn attribute(&self, name: &str, key: Option<&str>) -> PyResult<Option<ColumnStatistics>> {
        let attribute = attribute_from_name(name, key)?;
        Ok(self.inner.attribute(&attribute).map(column_bounds_parts))
    }

    fn __repr__(&self) -> String {
        format!("{:?}", self.inner)
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Project one column's statistics as the plain triple Python reads.
fn column_bounds_parts(bounds: &CoreColumnBounds) -> ColumnStatistics {
    (
        bounds.minimum().cloned().map(PyScalar::from_inner),
        bounds.maximum().cloned().map(PyScalar::from_inner),
        bounds.nulls(),
    )
}

/// A Python callable registered as a user-defined function.
///
/// The scalar tier hands each call its arguments as the Python values their
/// parameter fields project to and reads the answer back through the one
/// scalar crossing. The vectorized tier does the same for a whole column
/// under a single interpreter attachment - one crossing per column in, one
/// array out - or, for a callable declared `vectorized`, hands it `pyarrow`
/// arrays and casts the array it answers onto the declared return.
struct PyUserFunction {
    signature: CoreFunctionSignature,
    callable: Py<PyAny>,
    vectorized: bool,
}

/// A Python refusal, named as the function's own.
fn user_error(reference: &CoreUserRef, error: &PyErr) -> yggdryl::Error {
    yggdryl::Error::InvalidRecord {
        path: format!("$.{reference}").into(),
        reason: format!("{error}").into(),
    }
}

impl PyUserFunction {
    /// Call the Python function once, over values already filled to the
    /// signature.
    fn call_py(&self, py: Python<'_>, arguments: &[yggdryl::Scalar]) -> PyResult<yggdryl::Scalar> {
        let mut values = Vec::with_capacity(arguments.len());
        for (argument, parameter) in arguments.iter().zip(self.signature.parameters()) {
            values.push(crate::scalar::as_py_with_field(py, argument, parameter)?);
        }
        let answer = self.callable.call1(py, PyTuple::new(py, values)?)?;
        crate::scalar::from_py(&answer.into_bound(py))
    }
}

impl CoreUserFunction for PyUserFunction {
    fn signature(&self) -> &CoreFunctionSignature {
        &self.signature
    }

    fn call(&self, arguments: &[yggdryl::Scalar]) -> yggdryl::Result<yggdryl::Scalar> {
        Python::attach(|py| self.call_py(py, arguments))
            .map_err(|error| user_error(self.signature.reference(), &error))
    }

    fn call_arrow(
        &self,
        fields: &[yggdryl::Field],
        arguments: &[arrow_array::ArrayRef],
        rows: usize,
        output: &yggdryl::Field,
    ) -> yggdryl::Result<arrow_array::ArrayRef> {
        use yggdryl::arrow::{array_from_value, array_to_value};

        let reference = self.signature.reference();
        Python::attach(|py| -> yggdryl::Result<arrow_array::ArrayRef> {
            if self.vectorized {
                // Whole columns cross: each argument cast onto its parameter
                // datatype, nulls left to the callable, the answer cast onto
                // the declared return.
                let mut columns = Vec::with_capacity(arguments.len());
                for (array, parameter) in arguments.iter().zip(self.signature.parameters()) {
                    let target = parameter.clone().with_nullable(true);
                    let cast = target
                        .cast_arrow_array(Arc::clone(array), yggdryl::ArrowCastOptions::new())?;
                    columns.push(
                        arrow_array_to_pyarrow(py, &cast, Some(&target))
                            .map_err(|error| user_error(reference, &error))?,
                    );
                }
                let answer = (|| -> PyResult<arrow_array::ArrayRef> {
                    let answer = self.callable.call1(py, PyTuple::new(py, columns)?)?;
                    arrow_array_from_pyarrow(&answer.into_bound(py))
                })()
                .map_err(|error| user_error(reference, &error))?;
                return Ok(output
                    .clone()
                    .with_nullable(true)
                    .cast_arrow_array(answer, yggdryl::ArrowCastOptions::new())?);
            }
            // One attachment for the whole batch: every column crosses once,
            // each row is filled to the signature and called, and the answers
            // form one array.
            let mut columns = Vec::with_capacity(arguments.len());
            for (field, array) in fields.iter().zip(arguments) {
                let values = array_to_value(field, array.as_ref())?;
                columns.push(values.as_sequence().map(<[_]>::to_vec).unwrap_or_default());
            }
            let mut answers = Vec::with_capacity(rows);
            let mut values = Vec::with_capacity(arguments.len());
            for row in 0..rows {
                values.clear();
                for column in &columns {
                    values.push(column.get(row).cloned().unwrap_or(yggdryl::Scalar::Null));
                }
                answers.push(match self.signature.fill(&values)? {
                    Some(filled) => self
                        .call_py(py, &filled)
                        .map_err(|error| user_error(reference, &error))?,
                    None => yggdryl::Scalar::Null,
                });
            }
            Ok(array_from_value(
                output,
                &yggdryl::Scalar::from_sequence(answers),
            )?)
        })
    }
}

/// Read the parameters of a signature: a struct `Field`, or an iterable of
/// fields and `name: dtype` texts, in position order.
fn parameters_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<CoreField>> {
    if let Ok(field) = value.extract::<PyRef<'_, PyField>>() {
        return Ok(field.inner.fields().to_vec());
    }
    let mut fields = Vec::new();
    for parameter in value.try_iter()? {
        fields.push(core_field_from_value(&parameter?)?);
    }
    Ok(fields)
}

/// Read the return of a signature: a `Field`, a `DataType`, or the text of a
/// datatype, nullable unless the field says otherwise.
fn returns_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreField> {
    if let Ok(field) = value.extract::<PyRef<'_, PyField>>() {
        return Ok(field.inner.clone().with_name("returns"));
    }
    let dtype = crate::datatype::core_dtype_from_value(value)?;
    Ok(CoreField::new("returns", dtype, true))
}

/// Register a Python callable as the user-defined function `namespace.name`.
///
/// `parameters` type the arguments in position order and `returns` the
/// answer; `defaults` name the parameters a call may leave out, and
/// `vectorized` says the callable takes and answers `pyarrow` arrays. The
/// signature comes back as the struct field it is, so it can be stored,
/// compared, and read back with the same fidelity as any schema.
#[pyfunction]
#[pyo3(signature = (namespace, name, parameters, returns, callable, *, defaults=None, vectorized=false))]
pub(crate) fn register_user_function(
    namespace: &str,
    name: &str,
    parameters: &Bound<'_, PyAny>,
    returns: &Bound<'_, PyAny>,
    callable: &Bound<'_, PyAny>,
    defaults: Option<&Bound<'_, PyDict>>,
    vectorized: bool,
) -> PyResult<PyField> {
    let reference = CoreUserRef::new(namespace, name).map_err(value_error)?;
    let mut fields = parameters_from_value(parameters)?;
    if let Some(defaults) = defaults {
        for (parameter, default) in defaults.iter() {
            let parameter: String = parameter.extract()?;
            let position = fields
                .iter()
                .position(|field| field.name().eq_ignore_ascii_case(&parameter))
                .ok_or_else(|| {
                    value_error(format!(
                        "expected a default for one of the parameters, got {parameter:?}"
                    ))
                })?;
            let value = crate::scalar::from_py(&default)?;
            fields[position] =
                CoreFunctionSignature::with_default(fields[position].clone(), &value)
                    .map_err(value_error)?;
        }
    }
    let signature = CoreFunctionSignature::new(reference, fields, returns_from_value(returns)?)
        .map_err(value_error)?;
    let field = signature.as_field().map_err(value_error)?;
    register_function(Arc::new(PyUserFunction {
        signature,
        callable: callable.clone().unbind(),
        vectorized,
    }))
    .map_err(value_error)?;
    Ok(PyField::from_inner(field))
}

/// Remove the registered function `namespace.name`, answering whether one
/// was registered.
#[pyfunction]
pub(crate) fn unregister_user_function(name: &str) -> PyResult<bool> {
    Ok(unregister_function(
        &CoreUserRef::parse(name).map_err(value_error)?,
    ))
}

/// Every registered function, by qualified name.
#[pyfunction]
pub(crate) fn user_functions() -> Vec<String> {
    registered_functions()
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// The signature field of the registered function `namespace.name`, or
/// `None`.
#[pyfunction]
pub(crate) fn user_function_signature(name: &str) -> PyResult<Option<PyField>> {
    let reference = CoreUserRef::parse(name).map_err(value_error)?;
    match lookup_function(&reference) {
        Ok(function) => Ok(Some(PyField::from_inner(
            function.signature().as_field().map_err(value_error)?,
        ))),
        Err(_) => Ok(None),
    }
}
