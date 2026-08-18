//! Thin native Python views over Yggdryl core values.

use std::cmp::Ordering;

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use yggdryl::OwnedDifferences;

use crate::codec::{
    codec_decode, codec_decode_all, codec_decode_all_reader, codec_decode_all_text,
    codec_decode_inferred, codec_decode_inferred_text, codec_decode_reader, codec_decode_text,
    codec_encode, codec_encode_all, codec_encode_all_writer, codec_encode_path,
    codec_encode_writer, codec_infer, codec_infer_path, codec_infer_text, codec_normalize_format,
};
use crate::datatype::{PyDataType, PyDataTypeIterator};
use crate::field::{PyField, PyFieldMetadataIterator, PyFieldPropertyIterator, PyProtocolMetadata};
use crate::media::{PyMediaType, PyMediaTypeIterator, PyMimeType};
use crate::uri::{PyUri, PyUriPathIterator, PyUrl, PyUrn};

mod codec;
mod codings;
mod datatype;
mod field;
mod iceberg;
mod io;
mod media;
mod record;
mod timezone;
mod uri;
mod value;

pub(crate) fn value_error(error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}

fn compare(ordering: Ordering, operation: CompareOp) -> bool {
    match operation {
        CompareOp::Lt => ordering.is_lt(),
        CompareOp::Le => ordering.is_le(),
        CompareOp::Eq => ordering.is_eq(),
        CompareOp::Ne => ordering.is_ne(),
        CompareOp::Gt => ordering.is_gt(),
        CompareOp::Ge => ordering.is_ge(),
    }
}

fn normalize_index(index: isize, length: usize) -> Option<usize> {
    if index >= 0 {
        usize::try_from(index).ok().filter(|index| *index < length)
    } else {
        let signed_length = isize::try_from(length).ok()?;
        usize::try_from(signed_length.checked_add(index)?)
            .ok()
            .filter(|index| *index < length)
    }
}

/// Owning lazy iterator over stable native schema-difference lines.
#[pyclass(name = "DifferenceIterator", module = "yggdryl._native")]
pub(crate) struct PyDifferenceIterator {
    inner: OwnedDifferences,
}

impl PyDifferenceIterator {
    pub(crate) fn from_fields(
        left: &yggdryl::Field,
        right: &yggdryl::Field,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            inner: OwnedDifferences::from_fields(left, right, with_metadata, return_equal),
        }
    }

    pub(crate) fn from_data_types(
        left: &yggdryl::DataType,
        right: &yggdryl::DataType,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            inner: OwnedDifferences::from_data_types(left, right, with_metadata, return_equal),
        }
    }
}

#[pymethods]
impl PyDifferenceIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<String> {
        self.inner.next()
    }
}

/// Initializes the private native extension module.
#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDataType>()?;
    module.add_class::<PyField>()?;
    module.add_class::<PyDataTypeIterator>()?;
    module.add_class::<PyFieldMetadataIterator>()?;
    module.add_class::<PyFieldPropertyIterator>()?;
    module.add_class::<PyProtocolMetadata>()?;
    module.add_class::<PyDifferenceIterator>()?;
    module.add_class::<PyMimeType>()?;
    module.add_class::<PyMediaType>()?;
    module.add_class::<PyMediaTypeIterator>()?;
    module.add_class::<PyUri>()?;
    module.add_class::<PyUrl>()?;
    module.add_class::<PyUrn>()?;
    module.add_class::<PyUriPathIterator>()?;
    module.add_class::<timezone::PyTimezone>()?;
    module.add_class::<io::PyIOBase>()?;
    module.add_class::<crate::io::PyIOCursor>()?;
    module.add_class::<io::PyLineIterator>()?;
    module.add_class::<io::PyIOBaseIterator>()?;
    module.add_class::<record::PyRecordOptions>()?;
    module.add_class::<iceberg::PyCatalog>()?;
    module.add_class::<iceberg::PyNamespace>()?;
    module.add_class::<iceberg::PyNamespaces>()?;
    module.add_class::<iceberg::PyTables>()?;
    module.add_class::<iceberg::PyIcebergOptions>()?;
    module.add_class::<iceberg::PyTable>()?;
    module.add_class::<iceberg::PySchemaUpdate>()?;
    module.add_class::<iceberg::PyCompaction>()?;
    module.add_class::<iceberg::PyPartitionSpec>()?;
    module.add_class::<iceberg::PyPartitionField>()?;
    module.add_class::<iceberg::PySnapshot>()?;
    module.add_class::<iceberg::PyManifestFile>()?;
    module.add_class::<iceberg::PyDataFile>()?;
    module.add_function(wrap_pyfunction!(codings::gzip_loads, module)?)?;
    module.add_function(wrap_pyfunction!(codings::gzip_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(codings::zlib_loads, module)?)?;
    module.add_function(wrap_pyfunction!(codings::zlib_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(codings::zstd_loads, module)?)?;
    module.add_function(wrap_pyfunction!(codings::zstd_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(iceberg::iceberg_assign_field_ids, module)?)?;
    module.add_function(wrap_pyfunction!(iceberg::iceberg_can_promote, module)?)?;
    module.add_function(wrap_pyfunction!(iceberg::iceberg_schema_from_json, module)?)?;
    module.add_function(wrap_pyfunction!(iceberg::iceberg_schema_to_json, module)?)?;
    module.add_function(wrap_pyfunction!(codec_encode, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_inferred, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_inferred_text, module)?)?;
    module.add_function(wrap_pyfunction!(codec_encode_all, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_all, module)?)?;
    module.add_function(wrap_pyfunction!(codec_infer, module)?)?;
    module.add_function(wrap_pyfunction!(codec_infer_path, module)?)?;
    module.add_function(wrap_pyfunction!(codec_normalize_format, module)?)?;
    module.add_function(wrap_pyfunction!(codec_encode_writer, module)?)?;
    module.add_function(wrap_pyfunction!(codec_encode_path, module)?)?;
    module.add_function(wrap_pyfunction!(codec_encode_all_writer, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_text, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_reader, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_all_text, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_all_reader, module)?)?;
    module.add_function(wrap_pyfunction!(codec_infer_text, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
