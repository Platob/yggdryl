//! A compiled Arrow cast from one field to another, reusable across every
//! column or batch of the source layout.
//!
//! Everything a cast decides from two fields alone - which source child
//! answers which declared field, the recursive conversion each pair needs,
//! and the kernel options - is decided once here. Only the masks, offsets,
//! and dictionary reachability that vary per column are left to `apply`, and
//! an identity plan hands the column's own buffers back.

use arrow_pyarrow::FromPyArrow;
use arrow_schema::Schema as ArrowSchema;
use pyo3::prelude::*;
use pyo3::types::PyAny;
use yggdryl::{ArrowCastPlan, Field as CoreField, Serie};

use crate::chunked_serie::{PyChunkedSerie, chunked_of};
use crate::datatype::{ArrayIntake, core_field_to_pyarrow, pyarrow};
use crate::field::{PyField, core_field_from_value};
use crate::iomedia::record_batch_intake;
use crate::serie::{PySerie, described};
use crate::{cast_options, value_error};

/// Resolve a plan's source once: a `pyarrow.Schema` is the record root
/// `row` its columns are children of, and anything else is a field.
fn source_of(value: &Bound<'_, PyAny>) -> PyResult<CoreField> {
    if value.is_instance(pyarrow::schema(value.py())?)? {
        let schema = ArrowSchema::from_pyarrow_bound(value)?;
        return CoreField::from_arrow_schema("row", &schema).map_err(value_error);
    }
    core_field_from_value(value)
}

/// One Arrow cast compiled from a source field and a target field.
///
/// Compiling is the schema-dependent half of a cast, so a reader that yields
/// a thousand batches of one layout pays for it once. The plan is immutable
/// and holds no column, so the same one answers every column of that layout.
#[pyclass(
    name = "ArrowCastPlan",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyArrowCastPlan {
    inner: ArrowCastPlan,
}

#[pymethods]
impl PyArrowCastPlan {
    /// Compile the cast from `source` to `target`.
    ///
    /// Every failure the two fields alone can produce - an unsupported
    /// conversion, an ambiguous case-insensitive name, a required field the
    /// source cannot fill - is raised here rather than on the first column.
    #[new]
    #[pyo3(signature = (source, target, *, safe = true, representation = "value"))]
    fn new(
        source: &Bound<'_, PyAny>,
        target: &Bound<'_, PyAny>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        let source = source_of(source)?;
        let target = core_field_from_value(target)?;
        Ok(Self {
            inner: ArrowCastPlan::compile(&source, &target, options).map_err(value_error)?,
        })
    }

    /// The Arrow field an input must lay out as, as a `pyarrow.Field`.
    #[getter]
    fn source<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let source = CoreField::try_from(self.inner.as_source().as_ref()).map_err(value_error)?;
        core_field_to_pyarrow(py, &source)
    }

    /// The field every cast lands under.
    #[getter]
    fn target(&self) -> PyField {
        PyField::from_inner(self.inner.as_target().clone())
    }

    /// Whether a present value that does not convert becomes null.
    #[getter]
    fn safe(&self) -> bool {
        self.inner.as_options().is_safe()
    }

    /// What a same-width pair carries.
    #[getter]
    fn representation(&self) -> &'static str {
        self.inner.as_options().representation().as_str()
    }

    /// Whether the plan hands every input of its source layout straight back.
    #[getter]
    fn is_identity(&self) -> bool {
        self.inner.is_identity()
    }

    /// Run the plan over no rows, so a failure surfaces before any exist.
    ///
    /// Everything the values themselves could still refuse - an out-of-range
    /// number under `safe=False`, a null in a required column - is left to
    /// `apply`, because no value has been seen yet.
    fn preflight(&self) -> PyResult<()> {
        self.inner.preflight().map_err(value_error)
    }

    /// Cast one column of the source layout through this plan, or every
    /// chunk of chunked columns.
    ///
    /// A `Serie` is cast as it is; a `pyarrow.RecordBatch` is first the
    /// record column of its own schema and any other Arrow array the column
    /// of its own layout, exactly as `Serie.from_arrow_batch` and
    /// `Serie.from_arrow_array` read them with no field. A `ChunkedSerie`,
    /// a `pyarrow.ChunkedArray` and a `pyarrow.Table` are their chunks, as
    /// `ChunkedSerie.from_` reads them with no field: the plan is applied to
    /// every chunk, and the answer is a `ChunkedSerie` of as many chunks.
    ///
    /// The landing and the cast run off the GIL; a `Serie` is cloned out of
    /// its borrow before, so another thread writing it waits for the GIL
    /// rather than finding it borrowed.
    fn apply(&self, py: Python<'_>, serie: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let plan = &self.inner;
        if let Some(chunked) = chunked_of(serie)? {
            let cast = py
                .detach(|| plan.apply_chunked(&chunked))
                .map_err(value_error)?;
            return Ok(Py::new(py, PyChunkedSerie::from_inner(cast))?.into_any());
        }
        let options = *plan.as_options();
        let held = serie
            .extract::<PyRef<'_, PySerie>>()
            .map(|serie| serie.inner.clone());
        let cast = if let Ok(held) = held {
            py.detach(|| plan.apply(&held))
        } else if serie.is_instance(pyarrow::record_batch(py)?)? {
            let batch = record_batch_intake(serie)?;
            py.detach(|| {
                let batch = batch.validated()?;
                plan.apply(&Serie::from_arrow_batch(None, &batch, options)?)
            })
        } else {
            let array = ArrayIntake::from_value(serie)?;
            py.detach(|| plan.apply(&Serie::from_arrow_array(None, array.validated()?, options)?))
        };
        described(py, cast.map_err(value_error)?)
    }

    fn __repr__(&self) -> String {
        let options = self.inner.as_options();
        format!(
            "ArrowCastPlan(target={:?}, safe={}, representation={:?})",
            self.inner.as_target().name(),
            if options.is_safe() { "True" } else { "False" },
            options.representation().as_str(),
        )
    }
}
