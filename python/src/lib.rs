//! Thin native Python views over Yggdryl core values.

use std::cmp::Ordering;
use std::sync::OnceLock;

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use yggdryl::OwnedDifferences;

use crate::enums::{PyMediaType, PyMediaTypeIterator, PyMimeType};
use crate::text::codec::{
    PyCodecScalarIterator, codec_decode, codec_decode_all, codec_decode_all_reader,
    codec_decode_all_text, codec_decode_inferred, codec_decode_inferred_text, codec_decode_iter,
    codec_decode_reader, codec_decode_text, codec_encode, codec_encode_all,
    codec_encode_all_writer, codec_encode_path, codec_encode_writer, codec_infer, codec_infer_path,
    codec_infer_text, codec_normalize_format,
};
use crate::types::datatype::{PyAsciiEnum, PyDataType, PyDataTypeIterator};
use crate::types::field::{
    PyField, PyFieldMetadata, PyFieldMetadataIterator, PyFieldPropertyIterator, PyProtocolField,
};
use crate::types::scalar::PyScalar;
use crate::uri::{PyParameterIterator, PyParameters, PyUri, PyUriPathIterator, PyUrl, PyUrn};

mod arrow;
mod coding;
mod enums;
mod expression;
mod fix;
mod holder;
mod iobase;
mod iomedia;
mod media;
mod text;
mod txhash;
mod types;
mod uri;
mod version;
mod xxhash;

pub(crate) fn value_error(error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Coerce the two cast answers Python spells separately into one native value.
///
/// `safe` decides whether a present value may be converted; `nullability`
/// names the policy for a declared value that is absent; `representation` names
/// what a same-width pair carries. All three cross explicitly on every cast
/// entry point, so none is inferred from another.
pub(crate) fn cast_options(
    safe: bool,
    nullability: &str,
    representation: &str,
) -> PyResult<yggdryl::ArrowCastOptions> {
    Ok(yggdryl::ArrowCastOptions::new()
        .with_safe(safe)
        .with_nullability(yggdryl::Nullability::from_str(nullability).map_err(value_error)?)
        .with_representation(
            yggdryl::Representation::from_str(representation).map_err(value_error)?,
        ))
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
    use yggdryl::{
        Codec, DataTypeId, DataTypeKind, DigestAlgorithm, IOKind, IOMode, PythonKind, Scheme,
        TimeUnit, UnionMode,
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
    listing.set_item("io_modes", IOMode::ALL.map(IOMode::as_str).to_vec())?;
    listing.set_item("io_write_modes", IOMode::WRITE.map(IOMode::as_str).to_vec())?;
    listing.set_item(
        "leading_fragments",
        yggdryl::media::text::LeadingFragment::ALL
            .map(yggdryl::media::text::LeadingFragment::as_str)
            .to_vec(),
    )?;
    listing.set_item("codecs", Codec::ALL.map(Codec::as_str).to_vec())?;
    listing.set_item(
        "digest_algorithms",
        DigestAlgorithm::ALL.map(DigestAlgorithm::as_str).to_vec(),
    )?;
    listing.set_item("io_kinds", IOKind::ALL.map(IOKind::as_str).to_vec())?;
    listing.set_item(
        "edge_algorithms",
        yggdryl::EdgeAlgorithm::ALL
            .map(yggdryl::EdgeAlgorithm::as_str)
            .to_vec(),
    )?;
    listing.set_item(
        "formats",
        yggdryl::text::Format::ALL
            .map(yggdryl::text::Format::as_str)
            .to_vec(),
    )?;
    listing.set_item(
        "nullabilities",
        yggdryl::Nullability::ALL
            .map(yggdryl::Nullability::as_str)
            .to_vec(),
    )?;
    listing.set_item(
        "representations",
        yggdryl::Representation::ALL
            .map(yggdryl::Representation::as_str)
            .to_vec(),
    )?;
    listing.set_item(
        "python_kinds",
        PythonKind::ALL.map(PythonKind::as_str).to_vec(),
    )?;
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

/// The bridge that carries the core's `log` records into Python `logging`.
///
/// Held because `pyo3-log` caches each Python logger's effective level, and a
/// caller that changes a level after import needs that cache dropped.
static LOGGING: OnceLock<pyo3_log::ResetHandle> = OnceLock::new();

/// Drop the cached Python log levels, so a level changed after import applies.
///
/// `logging.getLogger("yggdryl").setLevel(...)` before the first record needs
/// nothing; changing a level once records have flowed needs this.
#[pyfunction]
fn refresh_logging() {
    if let Some(handle) = LOGGING.get() {
        handle.reset();
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    // Records travel under the Rust module path, so a core record from
    // `yggdryl::media::iceberg::table` reaches `logging` as
    // `yggdryl.media.iceberg.table` and the package's own logger is its root.
    //
    // The bridge is global, so every crate in the build would otherwise reach
    // Python: the Avro reader alone narrates a schema parse per manifest, and
    // that is the flood a caller enabling debug does not want. Only this
    // project's own targets pass below `warn`, so a dependency still surfaces
    // what went wrong and never what it did. `install` rather than `init`
    // because an embedder may have installed a logger already, and an
    // extension has no business replacing it.
    let bridge = pyo3_log::Logger::default()
        .filter(log::LevelFilter::Warn)
        .filter_target("yggdryl".to_owned(), log::LevelFilter::Trace)
        .install();
    if let Ok(handle) = bridge {
        let _ = LOGGING.set(handle);
    }
    register_classes(module)?;
    register_functions(module)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    // The two FIX facts a caller spells rather than derives: what an absent
    // `fix:branch` means, and where the FIX specification's own tag range ends.
    module.add("STANDARD_BRANCH", yggdryl::FixBranch::STANDARD.name())?;
    module.add("ULBRIDGE_BRANCH", yggdryl::ULBRIDGE_BRANCH)?;
    module.add("USER_TAG_MIN", yggdryl::FixId::USER_TAG_MIN)?;
    module.add("USER_TAG_MAX", yggdryl::FixId::USER_TAG_MAX)?;
    // The reserved Arrow schema metadata key that carries per-field dictionary
    // IDs across the C Data Interface, which has no slot for them.
    module.add(
        "IPC_DICTIONARY_IDS_KEY",
        yggdryl::arrow::IPC_DICTIONARY_IDS_KEY,
    )?;
    // The two byte-stream sizes every streamed read is shaped by: what one
    // chunk hands out, and what one transport fetch asks the store for.
    module.add(
        "DEFAULT_STREAM_BATCH_SIZE",
        yggdryl::DEFAULT_STREAM_BATCH_SIZE,
    )?;
    module.add("DEFAULT_FETCH_BYTE_SIZE", yggdryl::DEFAULT_FETCH_BYTE_SIZE)?;
    Ok(())
}

/// Register the native value and iterator classes.
fn register_classes(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDataType>()?;
    module.add_class::<PyAsciiEnum>()?;
    module.add_class::<PyField>()?;
    module.add_class::<PyScalar>()?;
    module.add_class::<crate::arrow::PyArrowValue>()?;
    module.add_class::<types::scalar::PyScalarIterator>()?;
    module.add_class::<types::scalar::PyScalarEntryIterator>()?;
    module.add_class::<media::avro::PyAvroSchema>()?;
    module.add_class::<media::avro::PyAvroContainer>()?;
    module.add_class::<media::avro::PyAvroBlock>()?;
    module.add_class::<media::avro::PyAvroBlockIterator>()?;
    module.add_class::<expression::PyExpression>()?;
    module.add_class::<expression::PyBound>()?;
    module.add_class::<expression::PyStatement>()?;
    module.add_class::<expression::PyBoundStatement>()?;
    module.add_class::<expression::PyBounds>()?;
    module.add_function(pyo3::wrap_pyfunction!(
        expression::expression_needs_quoting,
        module
    )?)?;
    module.add_function(pyo3::wrap_pyfunction!(
        expression::expression_vocabularies,
        module
    )?)?;
    module.add_class::<PyDataTypeIterator>()?;
    module.add_class::<PyFieldMetadataIterator>()?;
    module.add_class::<PyFieldPropertyIterator>()?;
    module.add_class::<PyFieldMetadata>()?;
    module.add_class::<PyProtocolField>()?;
    module.add_class::<types::python::PyPythonMetadata>()?;
    module.add_class::<types::cast::PyArrowCastPlan>()?;
    module.add_class::<fix::PyFixBranch>()?;
    module.add_class::<fix::PyFixRegistry>()?;
    module.add_class::<version::PyVersion>()?;
    module.add_class::<fix::PyFixFieldIterator>()?;
    module.add_class::<fix::PyFixMsg>()?;
    module.add_class::<fix::PyFixMsgIterator>()?;
    module.add_class::<fix::PyFixCodec>()?;
    module.add_class::<fix::PyFixLifecycle>()?;
    module.add_class::<fix::PyUlPlugin>()?;
    module.add_class::<fix::PyUlPlugins>()?;
    module.add_class::<fix::PyFixMessages>()?;
    module.add_class::<fix::PyMsgType>()?;
    module.add_class::<fix::PyMsgTypeIterator>()?;
    module.add_class::<fix::PyFixDefinitionIterator>()?;
    module.add_class::<PyDifferenceIterator>()?;
    module.add_class::<PyCodecScalarIterator>()?;
    module.add_class::<PyMimeType>()?;
    module.add_class::<PyMediaType>()?;
    module.add_class::<PyMediaTypeIterator>()?;
    module.add_class::<PyUri>()?;
    module.add_class::<PyUrl>()?;
    module.add_class::<PyUrn>()?;
    module.add_class::<PyUriPathIterator>()?;
    module.add_class::<PyParameters>()?;
    module.add_class::<PyParameterIterator>()?;
    module.add_class::<types::timezone::PyTimezone>()?;
    module.add_class::<iobase::PyIOBase>()?;
    holder::handles::register(module)?;
    coding::handles::register(module)?;
    media::handles::register(module)?;
    module.add_function(wrap_pyfunction!(enum_values, module)?)?;
    module.add_function(wrap_pyfunction!(crate::arrow::arrow_shapes, module)?)?;
    module.add_function(wrap_pyfunction!(iomedia::combined, module)?)?;
    module.add_class::<crate::iobase::PyIOCursor>()?;
    module.add_class::<crate::iobase::PyByteIterator>()?;
    module.add_class::<iobase::PyRecordIterator>()?;
    module.add_class::<iobase::PyIOBaseIterator>()?;
    module.add_class::<iomedia::PyRecordOptions>()?;
    module.add_class::<iomedia::PyTextOptions>()?;
    module.add_class::<media::iceberg::PyCatalog>()?;
    module.add_class::<media::iceberg::PyNamespace>()?;
    module.add_class::<media::iceberg::PyNamespaces>()?;
    module.add_class::<media::iceberg::PyTables>()?;
    module.add_class::<media::iceberg::PyNames>()?;
    module.add_class::<media::iceberg::PyNamespaceIterator>()?;
    module.add_class::<media::iceberg::PyTableIterator>()?;
    module.add_class::<media::iceberg::PyIcebergOptions>()?;
    module.add_class::<media::iceberg::PyTable>()?;
    module.add_class::<media::iceberg::PySchemaUpdate>()?;
    module.add_class::<media::iceberg::PyScanPlan>()?;
    module.add_class::<media::iceberg::PyCompaction>()?;
    module.add_class::<media::iceberg::PyPartitionSpec>()?;
    module.add_class::<media::iceberg::PyPartitionField>()?;
    module.add_class::<media::iceberg::PySnapshot>()?;
    module.add_class::<media::iceberg::PyManifestFile>()?;
    module.add_class::<media::iceberg::PyDataFile>()?;
    media::partition::register(module)?;
    xxhash::register(module)?;
    txhash::register(module)?;
    Ok(())
}

/// Register the native free functions.
fn register_functions(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(refresh_logging, module)?)?;
    module.add_function(wrap_pyfunction!(coding::gzip_loads, module)?)?;
    module.add_function(wrap_pyfunction!(coding::gzip_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(coding::zlib_loads, module)?)?;
    module.add_function(wrap_pyfunction!(coding::zlib_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(coding::zlib_loads_raw, module)?)?;
    module.add_function(wrap_pyfunction!(coding::zlib_dumps_raw, module)?)?;
    module.add_function(wrap_pyfunction!(coding::zstd_loads, module)?)?;
    module.add_function(wrap_pyfunction!(coding::zstd_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_global_registry, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_install_global_registry, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_schema, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_schema_carrying, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_schema_tags, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_parse_arrow_reader, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_classify_arrow_array, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_crate_fields, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_cfb_fields, module)?)?;
    module.add_function(wrap_pyfunction!(fix::fix_ulbridge_fields, module)?)?;
    module.add_function(wrap_pyfunction!(
        media::iceberg::iceberg_assign_field_ids,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        media::iceberg::iceberg_can_promote,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        media::iceberg::iceberg_schema_from_json,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        media::iceberg::iceberg_schema_into_json,
        module
    )?)?;
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
    module.add_function(wrap_pyfunction!(media::avro::avro_loads, module)?)?;
    module.add_function(wrap_pyfunction!(media::avro::avro_blocks, module)?)?;
    module.add_function(wrap_pyfunction!(media::avro::avro_dumps, module)?)?;
    module.add_function(wrap_pyfunction!(media::avro::avro_loads_single, module)?)?;
    module.add_function(wrap_pyfunction!(media::avro::avro_dumps_single, module)?)?;
    Ok(())
}
