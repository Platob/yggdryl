//! The one partition renderer, and the two batch transforms around it.
//!
//! A partition column lives in a path rather than in a row, so moving it
//! between the two is one implementation the core owns: `partition_text`
//! renders one value as the exact text a directory name carries, and
//! `with_partitions` and `without_partitions` add and drop those columns on
//! either side of a read or a write.

use arrow_array::RecordBatch;
use arrow_pyarrow::{FromPyArrow, IntoPyArrow};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyModule};

use crate::iomedia::{batch_reader_from_arrow_reader, batch_reader_to_pyarrow};
use crate::types::field::core_field_from_value;
use crate::types::scalar::from_py;
use crate::value_error;

/// Read the `column=value` pairs a caller spells as an iterable of pairs.
fn pairs_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for entry in value.try_iter()? {
        let entry = entry?;
        let (column, text): (String, String) = entry.extract()?;
        pairs.push((column, text));
    }
    Ok(pairs)
}

/// Render one value as the text a partition directory name carries.
///
/// This is the only partition renderer: a date is its ISO day, a decimal is
/// restated at its scale, and a null is the reserved `NULL_PARTITION`
/// literal. A path written any other way is not the path a read finds.
#[pyfunction]
#[pyo3(name = "partition_text")]
pub(crate) fn partition_text(value: &Bound<'_, PyAny>) -> PyResult<String> {
    yggdryl::media::partition::partition_text(&from_py(value)?)
        .map(|text| text.to_string())
        .map_err(value_error)
}

/// Widen rows with the partition columns their location's path spells.
///
/// Each text value is cast to the type `field` gives that column, and stays
/// text when no schema declares it. A `pyarrow.RecordBatchReader` is widened
/// lazily and reports its widened schema before the first batch is pulled.
#[pyfunction]
#[pyo3(name = "with_partitions", signature = (rows, partitions, field = None))]
pub(crate) fn with_partitions<'py>(
    py: Python<'py>,
    rows: &Bound<'py, PyAny>,
    partitions: &Bound<'py, PyAny>,
    field: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let pairs = pairs_from_value(partitions)?;
    let field = field.map(core_field_from_value).transpose()?;
    if let Ok(batch) = RecordBatch::from_pyarrow_bound(rows) {
        let widened = yggdryl::media::partition::with_partitions(&batch, &pairs, field.as_ref())
            .map_err(value_error)?;
        return widened.into_pyarrow(py);
    }
    let reader = batch_reader_from_arrow_reader(rows)?;
    let widened = yggdryl::media::partition::partitioned_reader(reader, pairs, field)
        .map_err(value_error)?;
    batch_reader_to_pyarrow(py, widened)
}

/// Drop from rows every column a path already spells out.
///
/// This is the write-side mirror, so a partitioned value is not stored again
/// in every row. Rows carrying none of the named columns pass through
/// unchanged.
#[pyfunction]
#[pyo3(name = "without_partitions", signature = (rows, partitions))]
pub(crate) fn without_partitions<'py>(
    py: Python<'py>,
    rows: &Bound<'py, PyAny>,
    partitions: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let pairs = pairs_from_value(partitions)?;
    if let Ok(batch) = RecordBatch::from_pyarrow_bound(rows) {
        let narrowed = yggdryl::media::partition::without_partitions(&batch, &pairs)
            .map_err(value_error)?;
        return narrowed.into_pyarrow(py);
    }
    let reader = batch_reader_from_arrow_reader(rows)?;
    // The narrowed schema is what the first batch would answer, so it is
    // derived once here rather than per batch.
    let probe = RecordBatch::new_empty(reader.schema());
    let schema = yggdryl::media::partition::without_partitions(&probe, &pairs)
        .map_err(value_error)?
        .schema();
    let narrowed = yggdryl::media::partition::narrowed_reader(reader, pairs, schema);
    batch_reader_to_pyarrow(py, narrowed)
}

/// Register the partition surface on the native module.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(pyo3::wrap_pyfunction!(partition_text, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(with_partitions, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(without_partitions, module)?)?;
    module.add("NULL_PARTITION", yggdryl::media::NULL_PARTITION)?;
    module.add(
        "DEFAULT_RECORD_BATCH_ROW_SIZE",
        yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE,
    )?;
    module.add(
        "AVRO_MAX_SCHEMA_DEPTH",
        yggdryl::media::avro::MAX_SCHEMA_DEPTH,
    )?;
    Ok(())
}
