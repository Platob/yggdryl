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
    Bound as CoreBound, BoundStatement as CoreBoundStatement, Direction, NullsOrder, Operator,
    Statement as CoreStatement,
};
use yggdryl::{Expression as CoreExpression, Scalar};

use crate::field::core_field_from_value;
use crate::record::{
    batch_reader_from_arrow_reader, batch_reader_from_arrow_table, batch_reader_to_pyarrow,
    record_batch_from_value,
};
use crate::value_error;

/// Read late-bound values once, before either expression or statement binding.
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
    Ok(CoreExpression::literal(crate::scalar::from_py(value)?))
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
            inner: CoreExpression::literal(crate::scalar::from_py(value)?),
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
    fn field(&self, schema: &Bound<'_, PyAny>) -> PyResult<crate::field::PyField> {
        let schema = core_field_from_value(schema)?;
        Ok(crate::field::PyField::from_inner(
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
    fn field(&self) -> crate::field::PyField {
        crate::field::PyField::from_inner(self.inner.field().clone())
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
        crate::scalar::as_py(py, &value)
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
    fn schema(&self) -> crate::field::PyField {
        crate::field::PyField::from_inner(self.inner.schema().clone())
    }

    /// The struct root the statement publishes.
    #[getter]
    fn output(&self) -> crate::field::PyField {
        crate::field::PyField::from_inner(self.inner.output().clone())
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
