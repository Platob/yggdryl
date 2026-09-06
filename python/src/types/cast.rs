//! A compiled Arrow record-batch cast, reusable across every batch of one
//! schema.
//!
//! Everything a cast decides from two schemas alone - which source column
//! answers which declared field, the recursive conversion each pair needs, the
//! target schema, and the kernel options - is decided once here. Only the
//! masks, offsets, and dictionary reachability that vary per batch are left to
//! `apply`, and an exact cast hands the caller's own batch back.

use std::sync::Arc;

use arrow_array::RecordBatch as ArrowRecordBatch;
use arrow_pyarrow::{FromPyArrow, ToPyArrow};
use arrow_schema::Schema as ArrowSchema;
use pyo3::prelude::*;
use pyo3::types::PyAny;
use yggdryl::ArrowCastPlan;

use crate::types::field::{PyField, core_field_from_value};
use crate::{cast_options, value_error};

/// One Arrow cast compiled from a source schema and a declared root.
///
/// Compiling is the schema-dependent half of a cast, so a reader that yields
/// a thousand batches of one schema pays for it once. The plan is immutable
/// and carries no batch, so the same one answers every batch of that schema.
#[pyclass(
    name = "ArrowCastPlan",
    module = "yggdryl._native",
    skip_from_py_object
)]
pub(crate) struct PyArrowCastPlan {
    inner: Arc<ArrowCastPlan>,
}

#[pymethods]
impl PyArrowCastPlan {
    /// Compile the cast from one source schema to one non-null Struct root.
    ///
    /// Every failure the two schemas alone can produce - an unsupported
    /// conversion, an ambiguous case-insensitive name, a required field the
    /// source cannot fill under `nullability="strict"` - is raised here
    /// rather than on the first batch.
    #[new]
    #[pyo3(signature = (source, target, *, safe=true, nullability="default", representation="value"))]
    fn new(
        source: &Bound<'_, PyAny>,
        target: &Bound<'_, PyAny>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Self> {
        let source = ArrowSchema::from_pyarrow_bound(source)?;
        let target = core_field_from_value(target)?;
        Ok(Self {
            inner: Arc::new(
                ArrowCastPlan::compile(
                    &source,
                    &target,
                    cast_options(safe, nullability, representation)?,
                )
                .map_err(value_error)?,
            ),
        })
    }

    /// The declared root this plan casts to.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.inner.as_field().clone())
    }

    /// The source schema this plan was compiled for.
    ///
    /// A batch of any other schema needs its own plan, which is what makes
    /// one plan reusable without a per-batch check.
    #[getter]
    fn source_schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.inner.as_source_schema().to_pyarrow(py)
    }

    /// The schema every batch this plan answers carries.
    #[getter]
    fn schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.inner.as_schema().to_pyarrow(py)
    }

    /// Whether a present value may be converted.
    #[getter]
    fn safe(&self) -> bool {
        self.inner.as_options().is_safe()
    }

    /// The policy for a declared value the source cannot fill.
    #[getter]
    fn nullability(&self) -> &'static str {
        self.inner.as_options().nullability().as_str()
    }

    /// What a same-width pair carries.
    #[getter]
    fn representation(&self) -> &'static str {
        self.inner.as_options().representation().as_str()
    }

    /// Run the plan over no rows, so a failure surfaces before any exist.
    ///
    /// This is what an empty backend asks before a lazy read is attempted:
    /// everything the values themselves could still refuse - an out-of-range
    /// number under `safe=True`, a null in a required column - is left to
    /// `apply`, because no value has been seen yet.
    fn preflight(&self) -> PyResult<()> {
        self.inner.preflight().map_err(value_error)
    }

    /// Cast one `PyArrow` `RecordBatch` through this plan.
    ///
    /// A batch that is already exactly what the plan answers is handed back
    /// as the caller's own object, so an exact cast costs no arrays.
    fn apply<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let value = ArrowRecordBatch::from_pyarrow_bound(batch)?;
        let source_schema = value.schema();
        let source_columns = value.columns().to_vec();
        let cast = self.inner.apply(value).map_err(value_error)?;
        if Arc::ptr_eq(&source_schema, &cast.schema())
            && source_columns
                .iter()
                .zip(cast.columns())
                .all(|(left, right)| Arc::ptr_eq(left, right))
        {
            return Ok(batch.clone());
        }
        cast.to_pyarrow(py)
    }

    fn __repr__(&self) -> String {
        format!(
            "ArrowCastPlan(field={:?}, safe={}, nullability={:?}, representation={:?})",
            self.inner.as_field().name(),
            if self.inner.as_options().is_safe() {
                "True"
            } else {
                "False"
            },
            self.inner.as_options().nullability().as_str(),
            self.inner.as_options().representation().as_str(),
        )
    }
}
