//! The filter, as a Python value.
//!
//! One class, `Expression`, carrying the same tree the Rust crate carries, and
//! `Bound`, the compiled form. Text parses through the same grammar in every
//! language, so a predicate written in a Python notebook is the predicate a
//! Rust reader runs and the predicate a JavaScript caller sends.
//!
//! Everywhere a filter is accepted, `str` is accepted too and *parses* - it is
//! never taken as a string literal, because a filter that silently matches
//! everything is the worst failure this layer could have.

use arrow_pyarrow::IntoPyArrow;
use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};
use yggdryl::expression::{
    Bound as CoreBound, BoundStatement as CoreBoundStatement, Bounds as CoreBounds,
    ColumnBounds as CoreColumnBounds, Comparison as CoreComparison, Direction,
    FieldSegment as CoreSegment, Function as CoreFunction, NullsOrder, Operator,
    Order as CoreOrder, Projection as CoreProjection, Selector as CoreSelector,
    Statement as CoreStatement,
};
use yggdryl::{Expression as CoreExpression, Scalar};

use crate::iomedia::{
    batch_reader_from_arrow_reader, batch_reader_from_arrow_table, batch_reader_to_pyarrow,
    record_batch_from_value,
};
use crate::types::datatype::{PyDataType, core_dtype_from_value};
use crate::types::field::core_field_from_value;
use crate::types::scalar::PyScalar;
use crate::value_error;

/// Read late-bound values once, before either expression or statement binding.
fn supplied_parameters(parameters: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(String, Scalar)>> {
    match parameters {
        Some(parameters) => parameters
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.extract::<String>()?,
                    crate::types::scalar::from_py(&value)?,
                ))
            })
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

/// Read a filter from an `Expression` or from text that parses as one.
pub(crate) fn expression_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreExpression> {
    if let Ok(expression) = value.extract::<PyRef<'_, PyExpression>>() {
        return Ok(expression.inner.clone());
    }
    let text: String = value.extract().map_err(|_| {
        value_error("expected an Expression or the text of one, got another object")
    })?;
    text.parse().map_err(value_error)
}

/// Read one arithmetic operand without confusing Python strings with values.
///
/// A string keeps the expression grammar's meaning (`"price"` is a column and
/// `"'EUR'"` is a literal). Every other native Python value crosses through
/// the shared `Scalar` inference before becoming a literal expression.
fn arithmetic_expression_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreExpression> {
    if let Ok(expression) = value.extract::<PyRef<'_, PyExpression>>() {
        return Ok(expression.inner.clone());
    }
    if value.is_instance_of::<PyString>() {
        return value
            .extract::<String>()?
            .parse::<CoreExpression>()
            .map_err(value_error);
    }
    Ok(CoreExpression::literal(crate::types::scalar::from_py(
        value,
    )?))
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

/// Read a list of operands, each an expression or a value.
fn operands_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<CoreExpression>> {
    let mut operands = Vec::new();
    for operand in value.try_iter()? {
        operands.push(arithmetic_expression_from_value(&operand?)?);
    }
    Ok(operands)
}

/// Read one path step: a struct child, a list position, or a map key.
fn segment_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreSegment> {
    if value.is_instance_of::<PyString>() {
        return Ok(CoreSegment::field(value.extract::<String>()?));
    }
    if let Ok(index) = value.extract::<i64>() {
        return Ok(CoreSegment::index(index));
    }
    CoreSegment::key(crate::types::scalar::from_py(value)?).map_err(value_error)
}

/// A recursive, typed filter and projection tree.
#[pyclass(
    name = "Expression",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyExpression {
    inner: CoreExpression,
}

impl PyExpression {
    pub(crate) const fn from_core(inner: CoreExpression) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyExpression {
    /// Parse one expression from its canonical text.
    #[new]
    fn new(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: text.parse().map_err(value_error)?,
        })
    }

    /// Parse one expression from its canonical text.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Self::new(text)
    }

    /// Read one expression from its structural JSON document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self {
            inner: CoreExpression::from_json(document).map_err(value_error)?,
        })
    }

    /// Name one top-level column.
    #[staticmethod]
    fn column(name: &str) -> Self {
        Self {
            inner: CoreExpression::column(name),
        }
    }

    /// Hold one constant.
    #[staticmethod]
    fn literal(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: CoreExpression::literal(crate::types::scalar::from_py(value)?),
        })
    }

    /// Name one holder attribute, such as `size` or `partition` with a column.
    #[staticmethod]
    #[pyo3(signature = (name, key = None))]
    fn attribute(name: &str, key: Option<&str>) -> PyResult<Self> {
        let selector = match key {
            Some(key) if name.eq_ignore_ascii_case("partition") => {
                yggdryl::expression::Selector::Partition(key.into())
            }
            Some(_) => {
                return Err(value_error(
                    "expected a key only for the partition attribute",
                ));
            }
            None => yggdryl::expression::Selector::from_name(name).ok_or_else(|| {
                value_error(format!(
                    "expected one of the holder attributes {}, got {name:?}",
                    yggdryl::expression::Selector::vocabulary()
                ))
            })?,
        };
        Ok(Self {
            inner: CoreExpression::attribute(selector),
        })
    }

    /// Name one late-bound value.
    #[staticmethod]
    fn parameter(name: &str) -> Self {
        Self {
            inner: CoreExpression::parameter(name),
        }
    }

    /// The expression that is true for every row.
    #[staticmethod]
    fn always_true() -> Self {
        Self {
            inner: CoreExpression::always_true(),
        }
    }

    /// The expression that is true for no row.
    #[staticmethod]
    fn always_false() -> Self {
        Self {
            inner: CoreExpression::always_false(),
        }
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

    /// The top-level `and` operands, flattened.
    fn conjuncts(&self) -> Vec<Self> {
        self.inner
            .conjuncts()
            .into_iter()
            .map(Self::from_core)
            .collect()
    }

    /// How deep this expression nests, counting itself as one level.
    fn depth(&self) -> usize {
        self.inner.depth()
    }

    /// Write this expression as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    /// Resolve this expression against a struct root schema.
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

    /// The output field this expression produces against a schema.
    fn field(&self, schema: &Bound<'_, PyAny>) -> PyResult<crate::types::field::PyField> {
        let schema = core_field_from_value(schema)?;
        Ok(crate::types::field::PyField::from_inner(
            self.inner.field(&schema).map_err(value_error)?,
        ))
    }

    /// Build `self + other` without evaluating either side.
    fn add(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(self.inner.clone().arithmetic(
            Operator::Add,
            arithmetic_expression_from_value(other)?,
        )))
    }

    /// Build `self - other` without evaluating either side.
    fn subtract(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(self.inner.clone().arithmetic(
            Operator::Sub,
            arithmetic_expression_from_value(other)?,
        )))
    }

    /// Build `self * other` without evaluating either side.
    fn multiply(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(self.inner.clone().arithmetic(
            Operator::Mul,
            arithmetic_expression_from_value(other)?,
        )))
    }

    /// Build `self / other` without evaluating either side.
    fn divide(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(self.inner.clone().arithmetic(
            Operator::Div,
            arithmetic_expression_from_value(other)?,
        )))
    }

    /// Build `self % other` without evaluating either side.
    fn remainder(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(self.inner.clone().arithmetic(
            Operator::Rem,
            arithmetic_expression_from_value(other)?,
        )))
    }

    /// Build `-self`, folding a numeric literal in the native core.
    /// Compare this expression with another under a named comparison.
    ///
    /// The vocabulary is the grammar's own - `=`, `<>`, `<`, `<=`, `>`, `>=`,
    /// `is distinct from`, `is not distinct from` - and `eq` through `ge` are
    /// the six spellings that name one each.
    fn compare(&self, comparison: &str, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        let comparison = comparison_from_str(comparison)?;
        Ok(Self::from_core(self.inner.clone().compare(
            comparison,
            arithmetic_expression_from_value(other)?,
        )))
    }

    /// `self = other`.
    fn eq(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .eq(arithmetic_expression_from_value(other)?),
        ))
    }

    /// `self <> other`.
    fn ne(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .ne(arithmetic_expression_from_value(other)?),
        ))
    }

    /// `self < other`.
    fn lt(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .lt(arithmetic_expression_from_value(other)?),
        ))
    }

    /// `self <= other`.
    fn le(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .le(arithmetic_expression_from_value(other)?),
        ))
    }

    /// `self > other`.
    fn gt(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .gt(arithmetic_expression_from_value(other)?),
        ))
    }

    /// `self >= other`.
    fn ge(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .ge(arithmetic_expression_from_value(other)?),
        ))
    }

    /// `self in (...)`, over the values or expressions given.
    fn is_in(&self, values: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().is_in(operands_from_value(values)?),
        ))
    }

    /// `self between low and high`, inclusive at both ends.
    fn between(&self, low: &Bound<'_, PyAny>, high: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(self.inner.clone().between(
            arithmetic_expression_from_value(low)?,
            arithmetic_expression_from_value(high)?,
        )))
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
            self.inner
                .clone()
                .like(arithmetic_expression_from_value(pattern)?),
        ))
    }

    /// `self ilike pattern`, folding ASCII case.
    fn ilike(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .ilike(arithmetic_expression_from_value(pattern)?),
        ))
    }

    /// `self glob pattern`, under the `.gitignore` path rule.
    fn glob(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .glob(arithmetic_expression_from_value(pattern)?),
        ))
    }

    /// Cast this expression to a datatype, refusing what it cannot hold.
    fn cast(&self, dtype: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner.clone().cast(core_dtype_from_value(dtype)?),
        ))
    }

    /// Cast this expression to a datatype, nulling what it cannot hold.
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

    /// Append a whole path of steps at once.
    ///
    /// A `str` step is a struct child, an `int` is a list position, and any
    /// other value is a map key, typed through the shared value rules.
    fn path(&self, segments: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut steps = Vec::new();
        for step in segments.try_iter()? {
            steps.push(segment_from_value(&step?)?);
        }
        Ok(Self::from_core(self.inner.clone().path(steps)))
    }

    /// Conjoin many operands into one flattened node; empty is true.
    #[staticmethod]
    fn all(operands: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreExpression::all(operands_from_value(
            operands,
        )?)))
    }

    /// Disjoin many operands into one flattened node; empty is false.
    #[staticmethod]
    fn any(operands: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreExpression::any(operands_from_value(
            operands,
        )?)))
    }

    /// Call one function of the closed scalar set.
    #[staticmethod]
    fn call(function: &str, arguments: &Bound<'_, PyAny>) -> PyResult<Self> {
        let function = CoreFunction::from_name(function).ok_or_else(|| {
            value_error(format!(
                "unknown function {function:?}; expected one of {}",
                CoreFunction::vocabulary()
            ))
        })?;
        Ok(Self::from_core(CoreExpression::call(
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
            pairs.push((
                expression_from_value(&when)?,
                arithmetic_expression_from_value(&then)?,
            ));
        }
        let otherwise = otherwise
            .map(arithmetic_expression_from_value)
            .transpose()?;
        Ok(Self::from_core(CoreExpression::case(pairs, otherwise)))
    }

    /// Hold a constant in an explicitly named datatype.
    ///
    /// `literal` infers the datatype from the value; this declares it, and
    /// the value is checked against it.
    #[staticmethod]
    fn typed_literal(dtype: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreExpression::typed_literal(
            core_dtype_from_value(dtype)?,
            crate::types::scalar::from_py(value)?,
        )
        .map(Self::from_core)
        .map_err(value_error)
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

    /// The number of nodes this expression holds.
    fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Refuse an expression past the depth or node limit, before a walk.
    fn check_budget(&self) -> PyResult<()> {
        self.inner.check_budget().map_err(value_error)
    }

    fn negate(&self) -> Self {
        Self::from_core(self.inner.clone().neg())
    }

    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.add(other)
    }

    fn __radd__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            arithmetic_expression_from_value(other)?.arithmetic(Operator::Add, self.inner.clone()),
        ))
    }

    fn __sub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.subtract(other)
    }

    fn __rsub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            arithmetic_expression_from_value(other)?.arithmetic(Operator::Sub, self.inner.clone()),
        ))
    }

    fn __mul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.multiply(other)
    }

    fn __rmul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            arithmetic_expression_from_value(other)?.arithmetic(Operator::Mul, self.inner.clone()),
        ))
    }

    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.divide(other)
    }

    fn __rtruediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            arithmetic_expression_from_value(other)?.arithmetic(Operator::Div, self.inner.clone()),
        ))
    }

    fn __mod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.remainder(other)
    }

    fn __rmod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            arithmetic_expression_from_value(other)?.arithmetic(Operator::Rem, self.inner.clone()),
        ))
    }

    fn __neg__(&self) -> Self {
        self.negate()
    }

    /// Build `self and other`.
    fn __and__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().and(expression_from_value(other)?),
        })
    }

    /// Build `self or other`.
    fn __or__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().or(expression_from_value(other)?),
        })
    }

    /// Build `not self`.
    fn __invert__(&self) -> Self {
        Self {
            inner: self.inner.clone().not(),
        }
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
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
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

/// One expression resolved against one schema, ready to answer.
#[pyclass(name = "Bound", module = "yggdryl._native", frozen)]
pub(crate) struct PyBound {
    inner: CoreBound,
}

#[pymethods]
impl PyBound {
    // A bound expression is an executable plan over a schema, not a canonical
    // public value. The core supplies no complete identity for it.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The expression as it stands after substitution, folding, and ordering.
    #[getter]
    fn expression(&self) -> PyExpression {
        PyExpression::from_core(self.inner.expression().clone())
    }

    /// The output field this expression produces.
    #[getter]
    fn field(&self) -> crate::types::field::PyField {
        crate::types::field::PyField::from_inner(self.inner.field().clone())
    }

    /// Return whether this expression answers a boolean.
    #[getter]
    fn is_predicate(&self) -> bool {
        self.inner.is_predicate()
    }

    /// The schema column names this expression reads, in index order.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner.column_names()
    }

    /// Return whether answering this expression requires reading rows.
    #[getter]
    fn reads_rows(&self) -> bool {
        self.inner.reads_rows()
    }

    /// Evaluate this expression for one row.
    ///
    /// The row is a sequence of column values in schema order, or a mapping
    /// from column name to value.
    fn eval(&self, py: Python<'_>, row: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let value = self
            .inner
            .eval(&self.row_value(row)?)
            .map_err(value_error)?;
        crate::types::scalar::as_py(py, &value)
    }

    /// Answer this predicate for one row, reading unknown as "no".
    fn matches(&self, row: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.inner
            .matches(&self.row_value(row)?)
            .map_err(value_error)
    }

    /// Split this predicate into the part a partition layout answers and the rest.
    fn partition_split(&self) -> (PyExpression, PyExpression) {
        let residual = self.inner.partition_split();
        (
            PyExpression::from_core(residual.answerable().clone()),
            PyExpression::from_core(residual.remaining().clone()),
        )
    }

    /// The struct root this expression was bound against.
    #[getter]
    fn schema(&self) -> crate::types::field::PyField {
        crate::types::field::PyField::from_inner(self.inner.schema().clone())
    }

    /// The schema column indices this expression reads, ascending.
    ///
    /// This is projection pushdown: a reader decodes these and no others.
    #[getter]
    fn column_indices(&self) -> Vec<usize> {
        self.inner.column_indices()
    }

    /// Run the vectorized tier over one batch, answering one value per row.
    fn evaluate_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        let values = self.inner.evaluate(&batch).map_err(value_error)?;
        crate::types::datatype::arrow_array_to_pyarrow(py, &values, None)
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
        crate::types::datatype::arrow_array_to_pyarrow(py, &array, None)
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
        filtered.into_pyarrow(py)
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

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Bound({:?})", self.inner.to_string())
    }
}

impl PyBound {
    /// Read one row, whichever way Python spelled it.
    fn row_value(&self, row: &Bound<'_, PyAny>) -> PyResult<Scalar> {
        if let Ok(mapping) = row.cast::<PyDict>() {
            let mut values = Vec::with_capacity(self.inner.schema().field_len());
            for field in self.inner.schema().fields() {
                let held = mapping.get_item(field.name())?;
                values.push(match held {
                    Some(held) => crate::types::scalar::from_py(&held)?,
                    None => Scalar::Null,
                });
            }
            return Ok(Scalar::from_sequence(values));
        }
        if row.is_instance_of::<PyList>() || row.is_instance_of::<PyTuple>() {
            return crate::types::scalar::from_py(row);
        }
        Err(value_error(
            "expected a sequence of column values in schema order, or a mapping of column to value",
        ))
    }
}

/// A projection list, a predicate, an ordering, and a limit.
#[pyclass(
    name = "Statement",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyStatement {
    inner: CoreStatement,
}

impl PyStatement {
    const fn from_core(inner: CoreStatement) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyStatement {
    /// Parse one statement from its canonical text.
    #[new]
    fn new(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: text.parse().map_err(value_error)?,
        })
    }

    /// Read one statement from its structural JSON document.
    /// The statement that selects every column, filters nothing, orders
    /// nothing, and limits nothing.
    #[staticmethod]
    fn all() -> Self {
        Self::from_core(CoreStatement::all())
    }

    /// Select a projection list.
    ///
    /// Each projection is an expression, or a `(expression, alias)` pair when
    /// the output column publishes a name of its own.
    #[staticmethod]
    fn select(projections: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut selected = Vec::new();
        for projection in projections.try_iter()? {
            selected.push(projection_from_value(&projection?)?);
        }
        Ok(Self::from_core(CoreStatement::select(selected)))
    }

    /// This statement with a predicate, replacing any it already carries.
    fn with_predicate(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .with_predicate(expression_from_value(predicate)?),
        ))
    }

    /// This statement with an ordering, replacing any it already carries.
    ///
    /// Each key is an expression, or a `(expression, direction)` or
    /// `(expression, direction, nulls)` tuple; `direction` is `"asc"` or
    /// `"desc"` and `nulls` is `"first"` or `"last"`.
    fn with_ordering(&self, ordering: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut keys = Vec::new();
        for key in ordering.try_iter()? {
            keys.push(order_from_value(&key?)?);
        }
        Ok(Self::from_core(self.inner.clone().with_ordering(keys)))
    }

    /// This statement with a row limit.
    fn with_limit(&self, limit: u64) -> Self {
        Self::from_core(self.inner.clone().with_limit(limit))
    }

    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        Ok(Self {
            inner: CoreStatement::from_json(document).map_err(value_error)?,
        })
    }

    /// The names this statement publishes, in output order. Empty means `*`.
    #[getter]
    fn projections(&self) -> Vec<String> {
        self.inner
            .projections()
            .iter()
            .map(|projection| projection.name().to_string())
            .collect()
    }

    /// The predicate, when the statement had a `where`.
    #[getter]
    fn predicate(&self) -> Option<PyExpression> {
        self.inner.predicate().cloned().map(PyExpression::from_core)
    }

    /// The ordering keys, in priority order.
    #[getter]
    fn ordering(&self) -> Vec<(PyExpression, &'static str, Option<&'static str>)> {
        self.inner
            .ordering()
            .iter()
            .map(|order| {
                (
                    PyExpression::from_core(order.expression().clone()),
                    direction_name(order.direction()),
                    order.nulls().map(nulls_name),
                )
            })
            .collect()
    }

    /// The row limit, when the statement had one.
    #[getter]
    fn limit(&self) -> Option<u64> {
        self.inner.limit()
    }

    /// Return whether this statement selects every input column unchanged.
    #[getter]
    fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Resolve every statement expression against one struct root schema.
    #[pyo3(signature = (schema, parameters = None))]
    fn bind(
        &self,
        schema: &Bound<'_, PyAny>,
        parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyBoundStatement> {
        let schema = core_field_from_value(schema)?;
        let supplied = supplied_parameters(parameters)?;
        let borrowed = parameter_refs(&supplied);
        Ok(PyBoundStatement {
            inner: self
                .inner
                .bind_with(&schema, &borrowed)
                .map_err(value_error)?,
        })
    }

    /// Write this statement as a structural JSON document.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Statement({:?})", self.inner.to_string())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
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

/// A statement resolved against one schema, ready for row-batch execution.
#[pyclass(name = "BoundStatement", module = "yggdryl._native", frozen)]
pub(crate) struct PyBoundStatement {
    inner: CoreBoundStatement,
}

#[pymethods]
impl PyBoundStatement {
    // A bound statement retains executable planning state and has no native
    // complete identity, so an object-identity hash would be misleading.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The struct root the statement reads.
    #[getter]
    fn schema(&self) -> crate::types::field::PyField {
        crate::types::field::PyField::from_inner(self.inner.schema().clone())
    }

    /// The struct root the statement publishes.
    #[getter]
    fn output(&self) -> crate::types::field::PyField {
        crate::types::field::PyField::from_inner(self.inner.output().clone())
    }

    /// The bound projections, in output order. Empty means every column.
    #[getter]
    fn projections(&self) -> Vec<PyBound> {
        self.inner
            .projections()
            .iter()
            .cloned()
            .map(|inner| PyBound { inner })
            .collect()
    }

    /// The bound predicate, when the statement had one.
    #[getter]
    fn predicate(&self) -> Option<PyBound> {
        self.inner
            .predicate()
            .cloned()
            .map(|inner| PyBound { inner })
    }

    /// The bound ordering keys, in priority order.
    #[getter]
    fn ordering(&self) -> Vec<(PyBound, &'static str, Option<&'static str>)> {
        self.inner
            .ordering()
            .iter()
            .map(|(bound, direction, nulls)| {
                (
                    PyBound {
                        inner: bound.clone(),
                    },
                    direction_name(*direction),
                    nulls.map(nulls_name),
                )
            })
            .collect()
    }

    /// The row limit, when the statement had one.
    #[getter]
    fn limit(&self) -> Option<u64> {
        self.inner.limit()
    }

    /// Return whether this statement selects every input column unchanged.
    #[getter]
    fn is_all(&self) -> bool {
        self.inner.is_all()
    }

    /// Filter and project one `pyarrow.RecordBatch` through the native plan.
    fn project_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        self.inner
            .project(&batch)
            .map_err(value_error)?
            .into_pyarrow(py)
    }

    /// Lazily filter, project, and limit one `pyarrow.RecordBatchReader`.
    fn project_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        reader: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        let projected = self
            .inner
            .clone()
            .project_reader(reader)
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, projected)
    }

    /// Filter, project, and limit one already-materialized `pyarrow.Table`.
    fn project_arrow_table<'py>(
        &self,
        py: Python<'py>,
        table: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = batch_reader_from_arrow_table(table)?;
        let projected = self
            .inner
            .clone()
            .project_reader(reader)
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, projected)?.call_method0("read_all")
    }

    /// Infer the Arrow holder, run its optimized core path, and preserve it.
    fn project_arrow<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let pyarrow = py.import("pyarrow")?;
        if value.is_instance(&pyarrow.getattr("RecordBatch")?)? {
            return self.project_arrow_batch(py, value);
        }
        if value.is_instance(&pyarrow.getattr("Table")?)? {
            return self.project_arrow_table(py, value);
        }
        self.project_arrow_reader(py, value)
    }

    /// Sort one `pyarrow.RecordBatch` through Arrow's native kernels.
    fn sort_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = record_batch_from_value(batch)?;
        self.inner
            .sort(&batch)
            .map_err(value_error)?
            .into_pyarrow(py)
    }
}

/// Read one projection: an expression, or an `(expression, alias)` pair.
fn projection_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreProjection> {
    if let Ok((expression, alias)) = value.extract::<(Bound<'_, PyAny>, String)>() {
        return Ok(CoreProjection::aliased(
            expression_from_value(&expression)?,
            alias,
        ));
    }
    Ok(CoreProjection::new(expression_from_value(value)?))
}

/// Read one ordering key and its optional direction and null placement.
fn order_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreOrder> {
    let (expression, direction, nulls) =
        if let Ok(parts) = value.extract::<(Bound<'_, PyAny>, String, String)>() {
            (parts.0, Some(parts.1), Some(parts.2))
        } else if let Ok(parts) = value.extract::<(Bound<'_, PyAny>, String)>() {
            (parts.0, Some(parts.1), None)
        } else {
            (value.clone(), None, None)
        };
    let mut order = CoreOrder::new(expression_from_value(&expression)?);
    if let Some(direction) = direction {
        order = order.with_direction(match direction.to_ascii_lowercase().as_str() {
            "asc" | "ascending" => Direction::Ascending,
            "desc" | "descending" => Direction::Descending,
            other => {
                return Err(value_error(format!(
                    "unknown sort direction {other:?}; expected \"asc\" or \"desc\""
                )));
            }
        });
    }
    if let Some(nulls) = nulls {
        order = order.with_nulls(match nulls.to_ascii_lowercase().as_str() {
            "first" => NullsOrder::First,
            "last" => NullsOrder::Last,
            other => {
                return Err(value_error(format!(
                    "unknown null placement {other:?}; expected \"first\" or \"last\""
                )));
            }
        });
    }
    Ok(order)
}

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
        CoreFunction::ALL.map(CoreFunction::as_str).to_vec(),
    )?;
    let attributes: Vec<&str> = CoreSelector::ALL.iter().map(CoreSelector::as_str).collect();
    listing.set_item("holder_attributes", attributes)?;
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
                minimum.map(crate::types::scalar::from_py).transpose()?,
                maximum.map(crate::types::scalar::from_py).transpose()?,
                nulls,
            ),
        })
    }

    /// Record one holder attribute's minimum, maximum, and null count.
    #[pyo3(signature = (name, minimum = None, maximum = None, nulls = None))]
    fn with_attribute(
        &self,
        name: &str,
        minimum: Option<&Bound<'_, PyAny>>,
        maximum: Option<&Bound<'_, PyAny>>,
        nulls: Option<u64>,
    ) -> PyResult<Self> {
        let selector = CoreSelector::from_name(name).ok_or_else(|| {
            value_error(format!(
                "unknown holder attribute {name:?}; expected one of {}",
                CoreSelector::vocabulary()
            ))
        })?;
        Ok(Self {
            inner: self.inner.clone().with_attribute(
                selector,
                minimum.map(crate::types::scalar::from_py).transpose()?,
                maximum.map(crate::types::scalar::from_py).transpose()?,
                nulls,
            ),
        })
    }

    /// One column's recorded statistics, as `(minimum, maximum, nulls)`.
    fn column(&self, name: &str) -> Option<ColumnStatistics> {
        self.inner.column(name).map(column_bounds_parts)
    }

    /// One attribute's recorded statistics, as `(minimum, maximum, nulls)`.
    fn attribute(&self, name: &str) -> PyResult<Option<ColumnStatistics>> {
        let selector = CoreSelector::from_name(name).ok_or_else(|| {
            value_error(format!(
                "unknown holder attribute {name:?}; expected one of {}",
                CoreSelector::vocabulary()
            ))
        })?;
        Ok(self.inner.attribute(&selector).map(column_bounds_parts))
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
