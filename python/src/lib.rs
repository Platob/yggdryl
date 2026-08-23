//! Thin native Python views over Yggdryl core values.

use std::cmp::Ordering;

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use yggdryl::OwnedDifferences;

use crate::codec::{
    PyCodecValueIterator, codec_decode, codec_decode_all, codec_decode_all_reader,
    codec_decode_all_text, codec_decode_inferred, codec_decode_inferred_text, codec_decode_iter,
    codec_decode_reader, codec_decode_text, codec_encode, codec_encode_all,
    codec_encode_all_writer, codec_encode_path, codec_encode_writer, codec_infer, codec_infer_path,
    codec_infer_text, codec_normalize_format,
};
use crate::datatype::{PyDataType, PyDataTypeIterator};
use crate::field::{
    PyField, PyFieldMetadata, PyFieldMetadataIterator, PyFieldPropertyIterator, PyProtocolMetadata,
};
use crate::media::{PyMediaType, PyMediaTypeIterator, PyMimeType};
use crate::uri::{PyUri, PyUriPathIterator, PyUrl, PyUrn};
use crate::value::PyValue;

mod arrowfs;
mod avro;
mod codec;
mod codings;
mod datatype;
mod expression;
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

/// Fold the core's unsigned stable hash into Python's signed `Py_hash_t`.
///
/// CPython reserves `-1` as an error sentinel, so that one result follows the
/// interpreter's own convention and becomes `-2`. The stable public accessor
/// remains the full `u64`; only the `hash()` protocol needs this narrowing.
pub(crate) const fn python_hash(stable: u64) -> isize {
    #[cfg(target_pointer_width = "64")]
    let value = stable as i64 as isize;
    #[cfg(target_pointer_width = "32")]
    let value = ((stable ^ (stable >> 32)) as u32 as i32) as isize;
    if value == -1 { -2 } else { value }
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

/// Map Python's `indent` keyword onto the core formatting value.
///
/// `None` is what a Python caller means by "no layout" - `json.dumps`'s own
/// default - so it maps to the explicit no-indent request rather than to the
/// format's default. Each bound method chooses its own natural default in its
/// signature (`indent=None` for JSON and TOML, `indent=2` for YAML), which is
/// what makes the zero-argument call read right in every format.
pub(crate) fn formatting_of(indent: Option<u8>) -> yggdryl::text::Formatting {
    match indent {
        None => yggdryl::text::Formatting::compact(),
        Some(width) => yggdryl::text::Formatting::indented(width),
    }
}

/// Resolve a subscript key to a child position on a schema node.
///
/// The one implementation behind `Field.__getitem__` and
/// `DataType.__getitem__`, so the two classes cannot drift: a `str` is a child
/// name, an `int` is a position (negative counting from the end), and anything
/// else is a `TypeError` with the same message shape.
pub(crate) enum ChildKey {
    Name(String),
    Position(isize),
}

impl ChildKey {
    /// Read a subscript key, or report what a schema node accepts.
    pub(crate) fn from_py(key: &pyo3::Bound<'_, pyo3::types::PyAny>) -> pyo3::PyResult<Self> {
        if let Ok(name) = key.extract::<String>() {
            return Ok(Self::Name(name));
        }
        if let Ok(index) = key.extract::<isize>() {
            return Ok(Self::Position(index));
        }
        Err(pyo3::exceptions::PyTypeError::new_err(
            "schema child index must be int or str",
        ))
    }
}

/// Read one nested child off a schema node, per [`ChildKey`]'s rules.
pub(crate) fn child_of(
    node: &yggdryl::DataType,
    key: &pyo3::Bound<'_, pyo3::types::PyAny>,
) -> pyo3::PyResult<yggdryl::Field> {
    match ChildKey::from_py(key)? {
        ChildKey::Name(name) => node
            .get_field_by_name(&name)
            .cloned()
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(name)),
        ChildKey::Position(index) => normalize_index(index, node.field_len())
            .and_then(|position| node.get_field(position).cloned())
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index)),
    }
}

/// Replace or append one nested child, through the core's cache-aware mutation.
///
/// A name replaces in place or appends; a position replaces only. One
/// implementation so `Field` and `DataType` cannot answer differently.
pub(crate) fn set_child(
    node: &mut yggdryl::Field,
    key: &pyo3::Bound<'_, pyo3::types::PyAny>,
    child: yggdryl::Field,
) -> pyo3::PyResult<()> {
    match ChildKey::from_py(key)? {
        ChildKey::Name(name) => node.set_field_by_name(&name, child).map_err(value_error),
        ChildKey::Position(index) => {
            let position = normalize_index(index, node.field_len())
                .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))?;
            node.set_field(position, child).map_err(value_error)
        }
    }
}

/// Remove one nested child, through the core's cache-aware mutation.
pub(crate) fn remove_child(
    node: &mut yggdryl::Field,
    key: &pyo3::Bound<'_, pyo3::types::PyAny>,
) -> pyo3::PyResult<()> {
    match ChildKey::from_py(key)? {
        ChildKey::Name(name) => node
            .remove_field_by_name(&name)
            .map(|_| ())
            .map_err(|_| pyo3::exceptions::PyKeyError::new_err(name)),
        ChildKey::Position(index) => {
            let position = normalize_index(index, node.field_len())
                .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))?;
            node.remove_field(position).map(|_| ()).map_err(value_error)
        }
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
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<String> {
        self.inner.next()
    }
}

/// Initializes the private native extension module.
/// Every static enum vocabulary of the core, as canonical spellings.
///
/// The single source the `yggdryl.enums` module unpacks, so the listings can
/// never drift from the Rust constants they mirror. Pure enums cross the
/// boundary as strings by convention; this is the enumeration of what those
/// strings can be.
#[pyfunction]
#[pyo3(name = "_enum_values")]
fn enum_values(py: Python<'_>) -> PyResult<Py<pyo3::types::PyDict>> {
    use pyo3::types::PyDict;
    use yggdryl::{
        Codec, DataTypeId, DataTypeKind, IOKind, Scheme, TimeUnit, UnionMode, WriteMode,
    };

    let listing = PyDict::new(py);
    listing.set_item(
        "data_type_ids",
        DataTypeId::ALL.map(DataTypeId::as_str).to_vec(),
    )?;
    listing.set_item(
        "data_type_kinds",
        DataTypeKind::ALL.map(DataTypeKind::as_str).to_vec(),
    )?;
    listing.set_item("time_units", TimeUnit::ALL.map(TimeUnit::as_str).to_vec())?;
    listing.set_item(
        "union_modes",
        UnionMode::ALL.map(UnionMode::as_str).to_vec(),
    )?;
    listing.set_item(
        "write_modes",
        WriteMode::ALL.map(WriteMode::as_str).to_vec(),
    )?;
    listing.set_item("codecs", Codec::ALL.map(Codec::as_str).to_vec())?;
    listing.set_item("io_kinds", IOKind::ALL.map(IOKind::as_str).to_vec())?;
    listing.set_item(
        "compatibility_schemes",
        Scheme::COMPATIBILITY_TARGETS
            .map(|scheme| scheme.as_str().to_owned())
            .to_vec(),
    )?;
    listing.set_item(
        "levels",
        [
            ("none", yggdryl::Level::NONE.get()),
            ("fast", yggdryl::Level::FAST.get()),
            ("default", yggdryl::Level::DEFAULT.get()),
            ("best", yggdryl::Level::BEST.get()),
        ]
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>(),
    )?;
    Ok(listing.into())
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDataType>()?;
    module.add_class::<PyField>()?;
    module.add_class::<PyValue>()?;
    module.add_class::<value::PyValueIterator>()?;
    module.add_class::<value::PyValueEntryIterator>()?;
    module.add_class::<avro::PyAvroSchema>()?;
    module.add_class::<avro::PyAvroContainer>()?;
    module.add_class::<avro::PyAvroBlock>()?;
    module.add_class::<avro::PyAvroBlockIterator>()?;
    module.add_class::<expression::PyExpression>()?;
    module.add_class::<expression::PyBound>()?;
    module.add_class::<expression::PyStatement>()?;
    module.add_class::<expression::PyBoundStatement>()?;
    module.add_class::<PyDataTypeIterator>()?;
    module.add_class::<PyFieldMetadataIterator>()?;
    module.add_class::<PyFieldPropertyIterator>()?;
    module.add_class::<PyFieldMetadata>()?;
    module.add_class::<PyProtocolMetadata>()?;
    module.add_class::<PyDifferenceIterator>()?;
    module.add_class::<PyCodecValueIterator>()?;
    module.add_class::<PyMimeType>()?;
    module.add_class::<PyMediaType>()?;
    module.add_class::<PyMediaTypeIterator>()?;
    module.add_class::<PyUri>()?;
    module.add_class::<PyUrl>()?;
    module.add_class::<PyUrn>()?;
    module.add_class::<PyUriPathIterator>()?;
    module.add_class::<timezone::PyTimezone>()?;
    module.add_class::<io::PyIOBase>()?;
    module.add_function(wrap_pyfunction!(io::field_from_pattern, module)?)?;
    module.add_function(wrap_pyfunction!(enum_values, module)?)?;
    module.add_function(wrap_pyfunction!(record::combined, module)?)?;
    module.add_class::<crate::io::PyIOCursor>()?;
    module.add_class::<crate::io::PyByteIterator>()?;
    module.add_class::<io::PyLineIterator>()?;
    module.add_class::<io::PyIOBaseIterator>()?;
    module.add_class::<record::PyRecordOptions>()?;
    module.add_class::<iceberg::PyCatalog>()?;
    module.add_class::<iceberg::PyNamespace>()?;
    module.add_class::<iceberg::PyNamespaces>()?;
    module.add_class::<iceberg::PyTables>()?;
    module.add_class::<iceberg::PyNames>()?;
    module.add_class::<iceberg::PyNamespaceIterator>()?;
    module.add_class::<iceberg::PyTableIterator>()?;
    module.add_class::<iceberg::PyIcebergOptions>()?;
    module.add_class::<iceberg::PyTable>()?;
    module.add_class::<iceberg::PySchemaUpdate>()?;
    module.add_class::<iceberg::PyScanPlan>()?;
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
    module.add_function(wrap_pyfunction!(codings::zlib_loads_raw, module)?)?;
    module.add_function(wrap_pyfunction!(codings::zlib_dumps_raw, module)?)?;
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
    module.add_function(wrap_pyfunction!(codec_decode_iter, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_all_text, module)?)?;
    module.add_function(wrap_pyfunction!(codec_decode_all_reader, module)?)?;
    module.add_function(wrap_pyfunction!(codec_infer_text, module)?)?;
    module.add_function(wrap_pyfunction!(avro::avro_loads, module)?)?;
    module.add_function(wrap_pyfunction!(avro::avro_blocks, module)?)?;
    module.add_function(wrap_pyfunction!(avro::avro_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(avro::avro_loads_single, module)?)?;
    module.add_function(wrap_pyfunction!(avro::avro_dumps_single, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
