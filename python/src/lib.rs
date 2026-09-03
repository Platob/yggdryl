//! Thin native Python views over Yggdryl core values.

use std::cmp::Ordering;

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use yggdryl::OwnedDifferences;

use crate::codec::{
    PyCodecScalarIterator, codec_decode, codec_decode_all, codec_decode_all_reader,
    codec_decode_all_text, codec_decode_inferred, codec_decode_inferred_text, codec_decode_iter,
    codec_decode_reader, codec_decode_text, codec_encode, codec_encode_all,
    codec_encode_all_writer, codec_encode_path, codec_encode_writer, codec_infer, codec_infer_path,
    codec_infer_text, codec_normalize_format,
};
use crate::datatype::{PyAsciiDictionary, PyDataType, PyDataTypeIterator};
use crate::field::{
    PyField, PyFieldMetadata, PyFieldMetadataIterator, PyFieldPropertyIterator, PyProtocolMetadata,
};
use crate::media::{PyMediaType, PyMediaTypeIterator, PyMimeType};
use crate::scalar::PyScalar;
use crate::uri::{PyUri, PyUriPathIterator, PyUrl, PyUrn};

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
mod scalar;
mod timezone;
mod uri;

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
/// `CPython` reserves `-1` as an error sentinel, so that one result follows the
/// interpreter's own convention and becomes `-2`. The stable public accessor
/// remains the full `u64`; only the `hash()` protocol needs this narrowing.
pub(crate) const fn python_hash(stable: u64) -> isize {
    #[cfg(target_pointer_width = "64")]
    let value = isize::from_ne_bytes(stable.to_ne_bytes());
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

/// Resolve a subscript key to a child of a schema node.
///
/// The one implementation behind `Field` and `DataType`'s item access and
/// their named accessors, so the two classes cannot drift: a `str` is a path
/// resolved name-first, an `int` is a position counting from the end when
/// negative, and anything else is a `TypeError`.
pub(crate) enum FieldKey {
    Path(String),
    Position(isize),
}

impl FieldKey {
    /// Read a key, or report what a schema node accepts.
    pub(crate) fn from_py(key: &pyo3::Bound<'_, pyo3::types::PyAny>) -> pyo3::PyResult<Self> {
        if let Ok(path) = key.extract::<String>() {
            return Ok(Self::Path(path));
        }
        if let Ok(index) = key.extract::<isize>() {
            return Ok(Self::Position(index));
        }
        Err(pyo3::exceptions::PyTypeError::new_err(
            "schema child index must be int or str",
        ))
    }
}

/// Read one nested child by position, counting from the end when negative.
pub(crate) fn field_at_of(
    node: &yggdryl::DataType,
    index: isize,
) -> pyo3::PyResult<yggdryl::Field> {
    normalize_index(index, node.field_len())
        .and_then(|position| node.get_field_at(position).cloned())
        .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))
}

/// Read one nested child by path, name-first.
pub(crate) fn field_by_path_of(
    node: &yggdryl::DataType,
    path: &str,
) -> pyo3::PyResult<yggdryl::Field> {
    node.get_field_by_path(path)
        .cloned()
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(path.to_owned()))
}

/// Read one nested child off a schema node, per [`FieldKey`]'s rules.
pub(crate) fn field_of(
    node: &yggdryl::DataType,
    key: &pyo3::Bound<'_, pyo3::types::PyAny>,
) -> pyo3::PyResult<yggdryl::Field> {
    match FieldKey::from_py(key)? {
        FieldKey::Path(path) => field_by_path_of(node, &path),
        FieldKey::Position(index) => field_at_of(node, index),
    }
}

/// Resolve the one key a `field(...)` call names, refusing an ambiguous call.
///
/// The positional form and the two keyword forms are three spellings of one
/// argument, so naming more than one of them is a `TypeError` rather than a
/// silent precedence rule.
pub(crate) fn one_field_key(
    key: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
    idx: Option<isize>,
    path: Option<&str>,
) -> pyo3::PyResult<FieldKey> {
    let given =
        usize::from(key.is_some()) + usize::from(idx.is_some()) + usize::from(path.is_some());
    if given != 1 {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "field() takes exactly one of a positional key, idx=, or path=",
        ));
    }
    if let Some(key) = key {
        return FieldKey::from_py(key);
    }
    if let Some(index) = idx {
        return Ok(FieldKey::Position(index));
    }
    Ok(FieldKey::Path(path.unwrap_or_default().to_owned()))
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

    pub(crate) fn from_dtypes(
        left: &yggdryl::DataType,
        right: &yggdryl::DataType,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            inner: OwnedDifferences::from_dtypes(left, right, with_metadata, return_equal),
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
    use yggdryl::{Codec, DataTypeId, DataTypeKind, IOKind, IOMode, Scheme, TimeUnit, UnionMode};

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
    listing.set_item("io_modes", IOMode::ALL.map(IOMode::as_str).to_vec())?;
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
    module.add_class::<PyAsciiDictionary>()?;
    module.add_class::<PyField>()?;
    module.add_class::<PyScalar>()?;
    module.add_class::<scalar::PyScalarIterator>()?;
    module.add_class::<scalar::PyScalarEntryIterator>()?;
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
    module.add_class::<PyCodecScalarIterator>()?;
    module.add_class::<PyMimeType>()?;
    module.add_class::<PyMediaType>()?;
    module.add_class::<PyMediaTypeIterator>()?;
    module.add_class::<PyUri>()?;
    module.add_class::<PyUrl>()?;
    module.add_class::<PyUrn>()?;
    module.add_class::<PyUriPathIterator>()?;
    module.add_class::<timezone::PyTimezone>()?;
    module.add_class::<io::PyIOBase>()?;
    module.add_function(wrap_pyfunction!(enum_values, module)?)?;
    module.add_function(wrap_pyfunction!(record::combined, module)?)?;
    module.add_class::<crate::io::PyIOCursor>()?;
    module.add_class::<crate::io::PyByteIterator>()?;
    module.add_class::<io::PyRecordIterator>()?;
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
    module.add_function(wrap_pyfunction!(iceberg::iceberg_schema_into_json, module)?)?;
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
