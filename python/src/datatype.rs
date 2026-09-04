//! Native Python view of Yggdryl datatypes.

use std::collections::BTreeMap;
use std::num::IntErrorKind;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch as ArrowRecordBatch, ffi::to_ffi, make_array};
use arrow_data::ArrayData;
use arrow_pyarrow::{FromPyArrow, PyArrowType, ToPyArrow};
use arrow_schema::{DataType as ArrowDataType, ffi::FFI_ArrowSchema};
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyKeyError, PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyByteArray, PyBytes, PyDict, PyIterator, PyList, PyString};
use yggdryl::ArrowCast;
use yggdryl::{
    AsciiDictionary as CoreAsciiDictionary, AsciiEnum as CoreAsciiEnum, DataType as CoreDataType,
    EdgeAlgorithm as CoreEdgeAlgorithm, Scheme as CoreScheme, TimeUnit as CoreTimeUnit,
    UnionMode as CoreUnionMode,
};

use crate::field::PyField;
use crate::scalar::{arrow_scalar_into_array, from_py};
use crate::{
    FieldKey, PyDifferenceIterator, compare, field_at_of, field_by_path_of, field_of,
    normalize_index, one_field_key, value_error,
};

fn import_ffi_schema<'py>(
    py: Python<'py>,
    class_name: &str,
    schema: &FFI_ArrowSchema,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("pyarrow")?
        .getattr(class_name)?
        .call_method1("_import_from_c", (std::ptr::from_ref(schema) as usize,))
}

pub(crate) fn core_dtype_to_pyarrow<'py>(
    py: Python<'py>,
    dtype: &CoreDataType,
) -> PyResult<Bound<'py, PyAny>> {
    let schema = dtype.clone().into_arrow_ffi().map_err(value_error)?;
    import_ffi_schema(py, "DataType", &schema)
}

pub(crate) fn core_field_to_pyarrow<'py>(
    py: Python<'py>,
    field: &yggdryl::Field,
) -> PyResult<Bound<'py, PyAny>> {
    let schema = field.clone().into_arrow_ffi().map_err(value_error)?;
    import_ffi_schema(py, "Field", &schema)
}

/// Imports a shared core Arrow one-row array through the exact owning Field.
///
/// The Field C Schema is intentional: importing only the array datatype would
/// discard extension metadata and prevent `PyArrow` from rehydrating a
/// registered `ExtensionType`.
pub(crate) fn default_arrow_scalar_to_pyarrow<'py>(
    py: Python<'py>,
    field: &yggdryl::Field,
    array: &ArrayRef,
) -> PyResult<Bound<'py, PyAny>> {
    let data = array.to_data();
    let (ffi_array, _dtype_schema) = to_ffi(&data).map_err(value_error)?;
    let ffi_field = field.clone().into_arrow_ffi().map_err(value_error)?;
    py.import("pyarrow")?
        .getattr("Array")?
        .call_method1(
            "_import_from_c",
            (
                std::ptr::from_ref(&ffi_array) as usize,
                std::ptr::from_ref(&ffi_field) as usize,
            ),
        )?
        .get_item(0)
}

/// Imports one `PyArrow` Array through the Arrow C Data Interface.
pub(crate) fn arrow_array_from_pyarrow(value: &Bound<'_, PyAny>) -> PyResult<ArrayRef> {
    ArrayData::from_pyarrow_bound(value).map(make_array)
}

/// Exports one Arrow array, optionally rehydrating an exact owning Field.
pub(crate) fn arrow_array_to_pyarrow<'py>(
    py: Python<'py>,
    array: &ArrayRef,
    field: Option<&yggdryl::Field>,
) -> PyResult<Bound<'py, PyAny>> {
    let data = array.to_data();
    let (ffi_array, ffi_dtype) = to_ffi(&data).map_err(value_error)?;
    let array_class = py.import("pyarrow")?.getattr("Array")?;
    if let Some(field) = field {
        let ffi_field = field.clone().into_arrow_ffi().map_err(value_error)?;
        return array_class.call_method1(
            "_import_from_c",
            (
                std::ptr::from_ref(&ffi_array) as usize,
                std::ptr::from_ref(&ffi_field) as usize,
            ),
        );
    }
    array_class.call_method1(
        "_import_from_c",
        (
            std::ptr::from_ref(&ffi_array) as usize,
            std::ptr::from_ref(&ffi_dtype) as usize,
        ),
    )
}

/// Constructs or safely casts one `PyArrow` scalar to an exact Arrow datatype.
///
/// Keeping this helper beside the C Schema projection gives `DataType`,
/// `Field`, and Python value conversion one contract without adding an
/// Arrow array/value dependency to the core crate.
pub(crate) fn arrow_scalar_from_core_type<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    dtype: &CoreDataType,
    safe: bool,
) -> PyResult<Bound<'py, PyAny>> {
    // An ASCII width, a registered code and a GUID all store as fixed-width
    // bytes `PyArrow` would refuse to build from a value of any other length,
    // so the core that owns the width rule answers for all three.
    if dtype.ascii_width().is_some() || matches!(dtype, CoreDataType::Guid) {
        return ascii_arrow_scalar(py, value, dtype, safe);
    }
    let target = core_dtype_to_pyarrow(py, dtype)?;
    arrow_scalar_to_pyarrow_type(py, value, target, safe)
}

/// Stores one value into a fixed-width datatype through the core boundary.
///
/// `PyArrow` builds a fixed-width binary only from exactly `width` bytes, so
/// the core that owns the padding and the width refusal answers instead: a
/// Python value crosses as a `Scalar` and takes the value contract, a
/// `PyArrow` scalar takes the cast plan. Both run under a nullable field so
/// `None` stays a null scalar; the caller's own field decides nullability.
pub(crate) fn ascii_arrow_scalar<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    dtype: &CoreDataType,
    safe: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let field = yggdryl::Field::new("value", dtype.clone(), true);
    let array = if value.is_instance(&py.import("pyarrow")?.getattr("Scalar")?)? {
        field
            .cast_arrow_array(arrow_scalar_into_array(value)?, safe)
            .map_err(value_error)?
    } else {
        yggdryl::arrow::scalar_array(&field, &from_py(value)?).map_err(value_error)?
    };
    arrow_array_to_pyarrow(py, &array, None)?.get_item(0)
}

pub(crate) fn arrow_scalar_to_pyarrow_type<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    target: Bound<'py, PyAny>,
    safe: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let pyarrow = py.import("pyarrow")?;
    let scalar_class = pyarrow.getattr("Scalar")?;

    if value.is_instance(&scalar_class)? {
        if value.getattr("type")?.eq(&target)? {
            return Ok(value.clone());
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("safe", safe)?;
        return value.call_method("cast", (target,), Some(&kwargs));
    }

    if safe {
        return typed_arrow_scalar(py, &pyarrow, value, target, true);
    }

    // PyArrow's typed scalar constructor has no `safe` argument. Infer once,
    // then use its explicit unsafe cast so callers opting out of overflow and
    // narrowing checks receive the same semantics as Scalar.cast.
    let Ok(inferred) = pyarrow.getattr("scalar")?.call1((value,)) else {
        return typed_arrow_scalar(py, &pyarrow, value, target, false);
    };
    if inferred.getattr("type")?.eq(&target)? {
        return Ok(inferred);
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("safe", false)?;
    match inferred.call_method("cast", (target.clone(),), Some(&kwargs)) {
        Ok(scalar) => Ok(scalar),
        // Inference cannot represent some inputs that typed construction can,
        // including Arrow map association lists. Unsafe mode must never accept
        // fewer values than safe typed construction.
        Err(_) => typed_arrow_scalar(py, &pyarrow, value, target, false),
    }
}

fn typed_arrow_scalar<'py>(
    py: Python<'py>,
    pyarrow: &Bound<'py, PyAny>,
    value: &Bound<'py, PyAny>,
    target: Bound<'py, PyAny>,
    safe: bool,
) -> PyResult<Bound<'py, PyAny>> {
    match pyarrow.getattr("scalar")?.call1((value, target.clone())) {
        Ok(scalar) => Ok(scalar),
        Err(value_error) => {
            // PyArrow exposes a few valid scalar layouts only through an
            // array slot (currently run-end encoding is the important case).
            let values = PyList::new(py, [value])?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("type", target)?;
            kwargs.set_item("safe", safe)?;
            match pyarrow.getattr("array")?.call((values,), Some(&kwargs)) {
                Ok(array) => array.get_item(0),
                Err(_) => Err(value_error),
            }
        }
    }
}

pub(crate) fn core_dtype_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreDataType> {
    if let Ok(value) = value.extract::<PyRef<'_, PyDataType>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreDataType::from_str(value).map_err(value_error);
    }

    match ArrowDataType::from_pyarrow_bound(value) {
        Ok(value) => return CoreDataType::try_from(value).map_err(value_error),
        Err(error) if !error.is_instance_of::<PyTypeError>(value.py()) => return Err(error),
        Err(_) => {}
    }

    core_dtype_from_pyhint(value)
}

fn core_dtype_from_pyhint(hint: &Bound<'_, PyAny>) -> PyResult<CoreDataType> {
    let inferred = hint
        .py()
        .import("yggdryl.fields._hints")?
        .getattr("datatype_from_pyhint")?
        .call1((hint,))?;
    let inferred = inferred.extract::<PyRef<'_, PyDataType>>()?;
    Ok(inferred.inner.clone())
}

fn core_fields_from_iterable(fields: &Bound<'_, PyAny>) -> PyResult<Vec<yggdryl::Field>> {
    fields
        .try_iter()?
        .enumerate()
        .map(|(index, item)| {
            let item = item?;
            item.extract::<PyRef<'_, PyField>>()
                .map(|field| field.inner.clone())
                .map_err(|_| {
                    PyTypeError::new_err(format!("field at index {index} must be a yggdryl.Field"))
                })
        })
        .collect()
}

fn decimal_integer(value: Borrowed<'_, '_, PyAny>, name: &str) -> PyResult<i128> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} must be an integer or numeric string, not bool"
        )));
    }
    if let Ok(text) = value.extract::<&str>() {
        return text.trim().parse::<i128>().map_err(|error| {
            if matches!(
                error.kind(),
                IntErrorKind::PosOverflow | IntErrorKind::NegOverflow
            ) {
                PyOverflowError::new_err(format!(
                    "{name} is outside the supported integer range: {text:?}"
                ))
            } else {
                PyValueError::new_err(format!(
                    "{name} must be a base-10 integer string, got {text:?}"
                ))
            }
        });
    }

    value.extract::<i128>().map_err(|error| {
        if error.is_instance_of::<PyTypeError>(value.py()) {
            let type_name = value
                .get_type()
                .name()
                .and_then(|name| name.to_str().map(str::to_owned))
                .unwrap_or_else(|_| "unknown".to_owned());
            PyTypeError::new_err(format!(
                "{name} must be an integer or numeric string, got {type_name}"
            ))
        } else {
            error
        }
    })
}

#[derive(Clone, Copy)]
struct DecimalPrecision(u8);

impl FromPyObject<'_, '_> for DecimalPrecision {
    type Error = PyErr;

    fn extract(value: Borrowed<'_, '_, PyAny>) -> PyResult<Self> {
        let value = decimal_integer(value, "precision")?;
        u8::try_from(value).map(Self).map_err(|_| {
            PyOverflowError::new_err(format!("precision must fit in an unsigned byte: {value}"))
        })
    }
}

#[derive(Clone, Copy)]
struct DecimalScale(i8);

impl FromPyObject<'_, '_> for DecimalScale {
    type Error = PyErr;

    fn extract(value: Borrowed<'_, '_, PyAny>) -> PyResult<Self> {
        let value = decimal_integer(value, "scale")?;
        i8::try_from(value).map(Self).map_err(|_| {
            PyOverflowError::new_err(format!("scale must fit in a signed byte: {value}"))
        })
    }
}

/// A cheaply cloned Yggdryl logical datatype.
///
/// Child mutation is copy-on-write, and hashing locks it: once a datatype has
/// been hashed it refuses to change, so one that is already a dict key or a set
/// member can never move. That is why this is no longer a `frozen` pyclass.
#[pyclass(name = "DataType", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyDataType {
    pub(crate) inner: CoreDataType,
    /// Set once this datatype has been hashed, so a later mutation cannot move
    /// it inside a dict or a set. The same lock a mutable `Field` carries.
    pub(crate) hash_locked: bool,
    /// Set when this datatype is the snapshot `Field.dtype` answers with.
    ///
    /// Writing to that snapshot could never reach the field it came from, so
    /// it refuses and names the field as the place to write instead of
    /// silently mutating a copy the caller then discards.
    pub(crate) borrowed_from_field: bool,
    /// Child Field views inherit the frozen state of the Field that exposed
    /// this datatype. The core datatype remains immutable in either case.
    pub(crate) children_read_only: bool,
}

impl PyDataType {
    /// Refuse a mutation that would move an already-hashed datatype, or one
    /// reached through a frozen `Field`.
    fn require_mutable(&self) -> PyResult<()> {
        if self.borrowed_from_field {
            Err(PyTypeError::new_err(
                "a DataType read off a Field is a snapshot; mutate the Field instead",
            ))
        } else if self.children_read_only {
            Err(PyTypeError::new_err(
                "this datatype belongs to a frozen field and is read-only",
            ))
        } else if self.hash_locked {
            Err(PyTypeError::new_err(
                "a hashed DataType is frozen; copy it before mutation",
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) const fn from_inner(inner: CoreDataType) -> Self {
        Self {
            inner,
            hash_locked: false,
            borrowed_from_field: false,
            children_read_only: false,
        }
    }

    fn from_validated(inner: CoreDataType) -> PyResult<Self> {
        inner.validate().map_err(value_error)?;
        Ok(Self::from_inner(inner))
    }
}

#[pymethods]
impl PyDataType {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_dtype_from_value(value).map(Self::from_inner)
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::new(value)
    }

    /// Internal direct constructor used by the typed ``yggdryl.fields`` facade.
    #[staticmethod]
    fn _simple(kind: &str) -> PyResult<Self> {
        let inner = match kind {
            "null" => CoreDataType::Null,
            "boolean" => CoreDataType::Boolean,
            "int8" => CoreDataType::Int8,
            "int16" => CoreDataType::Int16,
            "int32" => CoreDataType::Int32,
            "int64" => CoreDataType::Int64,
            "uint8" => CoreDataType::UInt8,
            "uint16" => CoreDataType::UInt16,
            "uint32" => CoreDataType::UInt32,
            "uint64" => CoreDataType::UInt64,
            "float16" => CoreDataType::Float16,
            "float32" => CoreDataType::Float32,
            "float64" => CoreDataType::Float64,
            "date32" => CoreDataType::Date32,
            "date64" => CoreDataType::Date64,
            "binary" => CoreDataType::Binary,
            "large_binary" => CoreDataType::LargeBinary,
            "binary_view" => CoreDataType::BinaryView,
            "utf8" => CoreDataType::Utf8,
            "large_utf8" => CoreDataType::LargeUtf8,
            "utf8_view" => CoreDataType::Utf8View,
            "ascii16" => CoreDataType::Ascii16,
            "ascii24" => CoreDataType::Ascii24,
            "ascii32" => CoreDataType::Ascii32,
            "ascii64" => CoreDataType::Ascii64,
            "ascii96" => CoreDataType::Ascii96,
            "ascii128" => CoreDataType::Ascii128,
            "country" => CoreDataType::Country,
            "currency" => CoreDataType::Currency,
            "mic" => CoreDataType::Mic,
            "cfi" => CoreDataType::Cfi,
            "guid" => CoreDataType::Guid,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "{kind:?} is not a parameter-free datatype kind"
                )));
            }
        };
        Self::from_validated(inner)
    }

    /// Internal temporal constructor used by the typed fields facade.
    #[staticmethod]
    #[pyo3(signature = (kind, unit, timezone=None))]
    fn _temporal(kind: &str, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        // A zone may arrive as a name, a `Timezone`, or a `zoneinfo.ZoneInfo`;
        // the core canonicalizes whichever it is, so two spellings of one zone
        // produce one datatype.
        let timezone = timezone
            .map(crate::timezone::core_timezone_from_value)
            .transpose()?;
        let unit = CoreTimeUnit::from_str(unit).map_err(value_error)?;
        let inner = match kind {
            "timestamp" => {
                if !unit.is_temporal() {
                    return Err(PyValueError::new_err(
                        "timestamp requires a temporal resolution unit",
                    ));
                }
                CoreDataType::Timestamp(unit, timezone)
            }
            "time32" => CoreDataType::time32(unit).map_err(value_error)?,
            "time64" => CoreDataType::time64(unit).map_err(value_error)?,
            "duration32" => {
                if !unit.is_temporal() {
                    return Err(PyValueError::new_err(
                        "duration32 requires a temporal resolution unit",
                    ));
                }
                CoreDataType::duration32(unit).map_err(value_error)?
            }
            "duration64" => {
                if !unit.is_temporal() {
                    return Err(PyValueError::new_err(
                        "duration64 requires a temporal resolution unit",
                    ));
                }
                CoreDataType::duration64(unit).map_err(value_error)?
            }
            "interval" => {
                if !unit.is_interval() {
                    return Err(PyValueError::new_err(
                        "interval requires an interval layout unit",
                    ));
                }
                CoreDataType::Interval(unit)
            }
            _ => {
                return Err(PyValueError::new_err(format!(
                    "{kind:?} is not a temporal datatype kind"
                )));
            }
        };
        Self::from_validated(inner)
    }

    /// Internal fixed-width binary constructor used by the typed fields facade.
    #[staticmethod]
    fn _fixed_size_binary(byte_width: i32) -> PyResult<Self> {
        let inner = CoreDataType::fixed_size_binary(byte_width).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Internal exact-width decimal constructor used by the typed fields facade.
    #[staticmethod]
    #[pyo3(signature = (kind, precision, scale=DecimalScale(0)))]
    fn _decimal(kind: &str, precision: DecimalPrecision, scale: DecimalScale) -> PyResult<Self> {
        let result = match kind {
            "decimal32" => CoreDataType::decimal32(precision.0, scale.0),
            "decimal64" => CoreDataType::decimal64(precision.0, scale.0),
            "decimal128" => CoreDataType::decimal128(precision.0, scale.0),
            "decimal256" => CoreDataType::decimal256(precision.0, scale.0),
            _ => {
                return Err(PyValueError::new_err(format!(
                    "{kind:?} is not an exact decimal datatype kind"
                )));
            }
        };
        Self::from_validated(result.map_err(value_error)?)
    }

    /// Internal list-layout constructor preserving the exact child Field.
    #[staticmethod]
    #[pyo3(signature = (kind, item, length=None))]
    #[allow(clippy::needless_pass_by_value)]
    fn _list(kind: &str, item: PyRef<'_, PyField>, length: Option<i32>) -> PyResult<Self> {
        let item = item.inner.clone();
        let inner = match kind {
            "list" if length.is_none() => CoreDataType::list(item),
            "list_view" if length.is_none() => CoreDataType::list_view(item),
            "fixed_size_list" => CoreDataType::fixed_size_list(
                item,
                length.ok_or_else(|| PyTypeError::new_err("fixed_size_list requires a length"))?,
            )
            .map_err(value_error)?,
            "large_list" if length.is_none() => CoreDataType::large_list(item),
            "large_list_view" if length.is_none() => CoreDataType::large_list_view(item),
            _ => {
                return Err(PyValueError::new_err(format!(
                    "invalid list kind/length combination: {kind:?}, {length:?}"
                )));
            }
        };
        Self::from_validated(inner)
    }

    /// Internal Union constructor preserving exact child Fields.
    #[staticmethod]
    fn _union(fields: &Bound<'_, PyAny>, mode: &str) -> PyResult<Self> {
        let mode = match mode {
            "sparse" => CoreUnionMode::Sparse,
            "dense" => CoreUnionMode::Dense,
            _ => {
                return Err(PyValueError::new_err(
                    "union mode must be 'sparse' or 'dense'",
                ));
            }
        };
        let mut native = Vec::new();
        for (index, item) in fields.try_iter()?.enumerate() {
            let item = item?;
            let (type_id, field) = item.extract::<(i8, PyRef<'_, PyField>)>().map_err(|_| {
                PyTypeError::new_err(format!(
                    "union member at index {index} must be an (int8, Field) pair"
                ))
            })?;
            native.push((type_id, field.inner.clone()));
        }
        let inner = CoreDataType::union(native, mode).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Builds a variant datatype, or the dense-union sugar over members.
    ///
    /// The parenthesis disambiguates, exactly as it does in the grammar:
    /// ``DataType.variant()`` with no argument is the self-describing
    /// semi-structured Variant datatype, while ``DataType.variant(fields)``
    /// keeps building the canonical dense Union with sequential native type
    /// IDs from the given members.
    #[staticmethod]
    #[pyo3(signature = (fields=None), text_signature = "(fields=None)")]
    fn variant(fields: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let inner = match fields {
            None => CoreDataType::variant(),
            Some(fields) => CoreDataType::dense_union(core_fields_from_iterable(fields)?)
                .map_err(value_error)?,
        };
        Self::from_validated(inner)
    }

    /// Creates a geometry datatype: planar features as Well-Known Binary.
    ///
    /// ``None`` fills the ``OGC:CRS84`` default shared with Parquet and
    /// Iceberg; a geometry connects vertices with straight planar lines, so
    /// it takes no edge algorithm.
    #[staticmethod]
    #[pyo3(signature = (crs=None), text_signature = "(crs=None)")]
    fn geometry(crs: Option<&str>) -> PyResult<Self> {
        let inner = CoreDataType::geometry(crs).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Creates a geography datatype: features on a sphere or spheroid.
    ///
    /// ``None`` fills the ``OGC:CRS84`` default and the ``spherical`` edge
    /// algorithm; ``algorithm`` accepts the canonical lowercase names the
    /// core's ``EdgeAlgorithm`` parses.
    #[staticmethod]
    #[pyo3(signature = (crs=None, algorithm=None), text_signature = "(crs=None, algorithm=None)")]
    fn geography(crs: Option<&str>, algorithm: Option<&str>) -> PyResult<Self> {
        let algorithm = algorithm
            .map(CoreEdgeAlgorithm::from_str)
            .transpose()
            .map_err(value_error)?;
        let inner = CoreDataType::geography(crs, algorithm).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Creates the ASCII width holding ``width`` bytes: 1-2 is ``ascii16``,
    /// 3 ``ascii24``, 4 ``ascii32``, 5-8 ``ascii64``, 9-12 ``ascii96``, and
    /// 13-16 ``ascii128``.
    #[staticmethod]
    fn ascii(width: i32) -> PyResult<Self> {
        let inner = CoreDataType::ascii(width).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Resolves a registered logical name such as ``currency`` or ``Price``
    /// to the datatype it spells, folding case, ``_``, ``-``, and spaces.
    #[staticmethod]
    fn from_logical_name(name: &str) -> PyResult<Self> {
        let inner = CoreDataType::from_logical_name(name).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// The registered logical names mapped to the datatype each spells, in
    /// registration order.
    #[staticmethod]
    fn logical_names(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let names = PyDict::new(py);
        for (name, dtype) in CoreDataType::LOGICAL_NAMES {
            names.set_item(name, Self::from_validated(dtype.clone())?)?;
        }
        Ok(names)
    }

    /// Internal Dictionary constructor preserving exact native datatypes.
    #[staticmethod]
    fn _dictionary(key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner =
            CoreDataType::dictionary(core_dtype_from_value(key)?, core_dtype_from_value(value)?)
                .map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Internal allocation-free dictionary value view for annotation inference.
    fn _dictionary_value_type(&self) -> PyResult<Self> {
        match &self.inner {
            CoreDataType::Dictionary(dictionary) => Ok(Self {
                inner: dictionary.value().clone(),
                hash_locked: false,
                borrowed_from_field: false,
                children_read_only: self.children_read_only,
            }),
            _ => Err(PyTypeError::new_err(
                "dictionary value type is available only on dictionary datatypes",
            )),
        }
    }

    /// Internal Map constructor preserving the exact entries Field.
    #[staticmethod]
    #[allow(clippy::needless_pass_by_value)]
    fn _map(entries: PyRef<'_, PyField>, keys_sorted: bool) -> PyResult<Self> {
        let inner = CoreDataType::map(entries.inner.clone(), keys_sorted).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Internal run-end constructor preserving both exact child Fields.
    #[staticmethod]
    #[allow(clippy::needless_pass_by_value)]
    fn _run_end_encoded(
        run_ends: PyRef<'_, PyField>,
        values: PyRef<'_, PyField>,
    ) -> PyResult<Self> {
        let inner = CoreDataType::run_end_encoded(run_ends.inner.clone(), values.inner.clone())
            .map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Creates the Arrow time-of-day width selected by a native unit.
    #[staticmethod]
    #[pyo3(text_signature = "(unit)")]
    fn time(unit: &str) -> PyResult<Self> {
        let unit = CoreTimeUnit::from_str(unit).map_err(value_error)?;
        CoreDataType::time(unit)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    fn duration32(unit: &str) -> PyResult<Self> {
        CoreDataType::duration32(CoreTimeUnit::from_str(unit).map_err(value_error)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    fn duration64(unit: &str) -> PyResult<Self> {
        CoreDataType::duration64(CoreTimeUnit::from_str(unit).map_err(value_error)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Creates the most compact wide decimal from integer-like arguments.
    #[staticmethod]
    #[pyo3(
        signature = (precision, scale = DecimalScale(0)),
        text_signature = "(precision, scale=0)"
    )]
    fn decimal(precision: DecimalPrecision, scale: DecimalScale) -> PyResult<Self> {
        CoreDataType::decimal(precision.0, scale.0)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Infers a native datatype from a Python type annotation.
    #[staticmethod]
    fn from_pyhint(hint: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_dtype_from_pyhint(hint).map(Self::from_inner)
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreDataType::from_str(value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_arrow(value: PyArrowType<ArrowDataType>) -> PyResult<Self> {
        let PyArrowType(value) = value;
        CoreDataType::try_from(value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Builds a native Struct directly from native child fields.
    #[staticmethod]
    fn from_fields(fields: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreDataType::from_fields(core_fields_from_iterable(fields)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        // One entry point for the three shapes a caller already has: the
        // document as text, the same document as the bytes it was read from,
        // or the structure `json.loads` already turned it into. Dispatching
        // here rather than making the caller pick keeps `dict` and `str`
        // equally first-class, which is what `into_dict` and `into_json`
        // already imply.
        if let Ok(text) = value.extract::<&str>() {
            return CoreDataType::from_json(text)
                .map(Self::from_inner)
                .map_err(value_error);
        }
        // Bytes-like by downcast rather than by extracting `Vec<u8>`, which a
        // list of small integers would also satisfy - and a JSON document that
        // really is a sequence must still reach the native path below.
        if let Ok(bytes) = value.cast::<pyo3::types::PyBytes>() {
            return CoreDataType::from_json_bytes(bytes.as_bytes())
                .map(Self::from_inner)
                .map_err(value_error);
        }
        if let Ok(bytes) = value.cast::<pyo3::types::PyByteArray>() {
            return CoreDataType::from_json_bytes(&bytes.to_vec())
                .map(Self::from_inner)
                .map_err(value_error);
        }
        CoreDataType::from_value(crate::scalar::from_py(value)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Returns the cached Python annotation corresponding to this datatype.
    fn default_pyhint<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dtype = Py::new(py, self.clone())?;
        py.import("yggdryl.fields._defaults")?
            .getattr("_default_pyhint_from_datatype")?
            .call1((dtype,))
    }

    /// Returns the native canonical default as an exact `PyArrow` Scalar.
    fn default_arrow_scalar<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = self.inner.default_arrow_array().map_err(value_error)?;
        let field = yggdryl::Field::new("value", self.inner.clone(), false);
        default_arrow_scalar_to_pyarrow(py, &field, &array)
    }

    /// Returns the native canonical default in its cached Python type plan.
    fn default_pyvalue<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = self.inner.default_arrow_array().map_err(value_error)?;
        let field = yggdryl::Field::new("value", self.inner.clone(), false);
        let scalar = default_arrow_scalar_to_pyarrow(py, &field, &array)?;
        let dtype = Py::new(py, self.clone())?;
        py.import("yggdryl.fields._defaults")?
            .getattr("_default_pyvalue_from_datatype")?
            .call1((dtype, scalar))
    }

    /// Returns a recursively normalized datatype for a named compatibility target.
    #[allow(clippy::wrong_self_convention)]
    fn into_scheme_compat(&self, target: &str) -> PyResult<Self> {
        let target = CoreScheme::from_str(target).map_err(value_error)?;
        self.inner
            .clone()
            .into_scheme_compat(&target)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Constructs a `PyArrow` scalar with this exact datatype.
    #[pyo3(signature = (value, *, safe=true))]
    fn arrow_scalar<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        arrow_scalar_from_core_type(py, value, &self.inner, safe)
    }

    /// Casts one `PyArrow` Array through Yggdryl's native Arrow kernels.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow_array<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input = arrow_array_from_pyarrow(value)?;
        let array = self
            .inner
            .cast_arrow_array(Arc::clone(&input), safe)
            .map_err(value_error)?;
        if Arc::ptr_eq(&input, &array) {
            return Ok(value.clone());
        }
        arrow_array_to_pyarrow(py, &array, None)
    }

    /// Reconciles one `PyArrow` `RecordBatch` to this Struct datatype.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = ArrowRecordBatch::from_pyarrow_bound(value)?;
        let source_schema = batch.schema();
        let source_columns = batch.columns().to_vec();
        let cast = self
            .inner
            .cast_arrow_batch(batch, safe)
            .map_err(value_error)?;
        if Arc::ptr_eq(&source_schema, &cast.schema())
            && source_columns
                .iter()
                .zip(cast.columns())
                .all(|(left, right)| Arc::ptr_eq(left, right))
        {
            return Ok(value.clone());
        }
        cast.to_pyarrow(py)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_arrow<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        core_dtype_to_pyarrow(py, &self.inner)
    }

    /// Serialize as deterministic structural JSON.
    ///
    /// `indent=None` is compact - today's output and the default; an integer
    /// pretty-prints with that many spaces per nesting level, exactly as
    /// `json.dumps(indent=n)` reads.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_json(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .clone()
            .into_json_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Serialize to structural JSON bytes.
    ///
    /// The same document `into_json` renders, encoded rather than decoded, for
    /// a caller writing it straight to a file or a socket. `from_json` reads
    /// these bytes back without being told which of the three shapes it got.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_json_bytes<'py>(
        &self,
        py: Python<'py>,
        indent: Option<u8>,
    ) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        let text = self
            .inner
            .clone()
            .into_json_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)?;
        Ok(pyo3::types::PyBytes::new(py, text.as_bytes()))
    }

    /// Deserialize and validate from structural YAML.
    ///
    /// The same structure `from_json` reads, in YAML's syntax - so a config
    /// document can carry a declared schema inline beside its other settings.
    #[staticmethod]
    fn from_yaml(value: &str) -> PyResult<Self> {
        CoreDataType::from_yaml(value)
            .map_err(value_error)
            .and_then(Self::from_validated)
    }

    /// Serialize as YAML: block style, one key per line.
    ///
    /// `indent=2` is the default. `indent=None` asks for flow style -
    /// `{a: 1, b: 2}` on one line - which is valid YAML and round-trips, and
    /// is never what a caller gets by accident.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = Some(2)))]
    fn into_yaml(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .clone()
            .into_yaml_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Deserialize and validate from structural TOML.
    #[staticmethod]
    fn from_toml(value: &str) -> PyResult<Self> {
        CoreDataType::from_toml(value)
            .map_err(value_error)
            .and_then(Self::from_validated)
    }

    /// Serialize as TOML.
    ///
    /// TOML has no null, and this model never needs one: an unset optional
    /// attribute is omitted rather than faked, so nothing is lost.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_toml(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .clone()
            .into_toml_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Project this value onto a plain structural mapping.
    ///
    /// The core's one structural model - the model JSON, YAML, and TOML are
    /// all expressed over - handed back as a `dict`, so a schema drops into any
    /// document a caller already builds.
    #[allow(clippy::wrong_self_convention)]
    fn into_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::scalar::as_py(py, &self.inner.clone().into_value())
    }

    /// Read this value back from a plain structural mapping.
    ///
    /// The inverse of `into_dict`, through the core's one conversion.
    #[staticmethod]
    fn from_dict(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreDataType::from_value(crate::scalar::from_py(value)?)
            .map_err(value_error)
            .and_then(Self::from_validated)
    }

    /// A readable, indented rendering of this value and everything under it.
    ///
    /// `str` and `repr` are unchanged - the compact constructor form, which
    /// round-trips through `from_str` - because that is what a Python reader
    /// expects of `repr` and what the parsers depend on. This is the form for
    /// a human looking at a schema three levels deep.
    fn pretty(&self) -> String {
        format!("{:#}", self.inner)
    }

    /// The readable rendering, for `IPython` and notebook cells.
    fn _repr_pretty_(&self, printer: &Bound<'_, PyAny>, _cycle: bool) -> PyResult<()> {
        printer.call_method1("text", (self.pretty(),))?;
        Ok(())
    }

    /// Coarse datatype family, such as ``integer`` or ``list``.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// Parameter-free variant identifier, such as ``int32`` or ``decimal128``.
    #[getter]
    fn id(&self) -> &'static str {
        self.inner.id().as_str()
    }

    #[getter]
    fn is_nested(&self) -> bool {
        self.inner.is_nested()
    }

    /// The storage width of an ASCII datatype in bytes, ``None`` for every other.
    #[getter]
    fn ascii_width(&self) -> Option<i32> {
        self.inner.ascii_width()
    }

    /// The integer an ASCII value packs into: its storage bytes, big-endian.
    ///
    /// The packed integer is the same in every process, so it is what an enum
    /// member and a stable hash are, and it is exactly the bytes an ASCII
    /// column stores.
    fn ascii_packed(&self, value: &Bound<'_, PyAny>) -> PyResult<i128> {
        self.inner
            .ascii_packed(&ascii_value_of(value)?)
            .map_err(value_error)
    }

    /// The ASCII value a packed integer carries, without its padding.
    fn ascii_value(&self, packed: i128) -> PyResult<String> {
        self.inner
            .ascii_value(packed)
            .map(|value| value.to_string())
            .map_err(value_error)
    }

    /// Internal field-class conversion view of temporal resolution. Keeping this on
    /// the native value avoids projecting a `PyArrow` datatype for every cell.
    fn _time_unit(&self) -> Option<&'static str> {
        match &self.inner {
            CoreDataType::Timestamp(unit, _)
            | CoreDataType::Time32(unit)
            | CoreDataType::Time64(unit)
            | CoreDataType::Duration32(unit)
            | CoreDataType::Duration64(unit) => Some(unit.as_str()),
            _ => None,
        }
    }

    /// Internal field-class conversion view of a timestamp timezone.
    fn _timezone(&self) -> Option<&str> {
        match &self.inner {
            CoreDataType::Timestamp(_, timezone) => {
                timezone.as_ref().map(yggdryl::Timezone::as_str)
            }
            _ => None,
        }
    }

    /// The canonical time zone of a timestamp, as a `Timezone`.
    ///
    /// A naive timestamp - one with no zone at all - answers `None`, which is
    /// how the absence of a zone is spelled everywhere in the project.
    #[getter]
    fn timezone(&self) -> Option<crate::timezone::PyTimezone> {
        match &self.inner {
            CoreDataType::Timestamp(_, timezone) => {
                timezone.clone().map(crate::timezone::PyTimezone::from_core)
            }
            _ => None,
        }
    }

    /// Internal field-class conversion view of fixed-size-list arity.
    fn _fixed_size_list_length(&self) -> Option<i32> {
        match &self.inner {
            CoreDataType::FixedSizeList(_, length) => Some(*length),
            _ => None,
        }
    }

    /// Compare recursively, optionally ignoring metadata on every child Field.
    #[pyo3(signature = (other, with_metadata=true))]
    fn equals(&self, other: &Self, with_metadata: bool) -> bool {
        self.inner.equals(&other.inner, with_metadata)
    }

    /// Iterate stable, terminal-readable recursive difference lines.
    #[pyo3(signature = (other, with_metadata=true, return_equal=false))]
    fn show_diffs(
        &self,
        other: &Self,
        with_metadata: bool,
        return_equal: bool,
    ) -> PyDifferenceIterator {
        PyDifferenceIterator::from_dtypes(&self.inner, &other.inner, with_metadata, return_equal)
    }

    /// Join every recursive difference line.
    ///
    /// ``return_equal`` reports the equal marker instead of an empty
    /// string when the two values match.
    #[pyo3(signature = (other, with_metadata=true, return_equal=true))]
    fn show_diff(&self, other: &Self, with_metadata: bool, return_equal: bool) -> String {
        self.inner
            .show_diff(&other.inner, with_metadata, return_equal)
    }

    fn __len__(&self) -> usize {
        self.inner.field_len()
    }

    fn __iter__(&self) -> PyDataTypeIterator {
        PyDataTypeIterator {
            inner: self.inner.clone(),
            index: 0,
            read_only: self.children_read_only,
        }
    }

    /// Reach a nested child by name or by position.
    ///
    /// Item access on a schema node means a **child**, never metadata: a `str`
    /// is a child name and raises `KeyError` when absent, an `int` is a
    /// position counting from the end when negative and raising `IndexError`
    /// when out of range, and anything else raises `TypeError`. `Field` carries
    /// exactly the same behavior, so a caller walking one object graph gets a
    /// child from every node in it. Metadata is reached through
    /// `Field.metadata[...]` or `Field.get_metadata(...)`.
    ///
    /// Chained subscripts descend: `row["order"]["price"]`. There is no dotted
    /// path form.
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PyField> {
        field_of(&self.inner, key)
            .map(|field| PyField::from_inner_with_read_only(field, self.children_read_only))
    }

    /// Replace a nested child, or append one under an unresolved name.
    ///
    /// Reading and writing are now the same story on both classes: `dtype[key]`
    /// and `field[key]` reach the same child, and either can rewrite it. A
    /// datatype rebuilds itself in place, so a list stays a list and only a
    /// struct may grow or shrink.
    ///
    /// A datatype that has been hashed refuses, so one already used as a dict
    /// key or a set member can never move.
    fn __setitem__(&mut self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let child = crate::field::core_field_from_value(value)?;
        match FieldKey::from_py(key)? {
            FieldKey::Path(path) => self
                .inner
                .set_field_by_path(&path, child)
                .map_err(value_error),
            FieldKey::Position(index) => {
                let position = normalize_index(index, self.inner.field_len())
                    .ok_or_else(|| PyIndexError::new_err(index))?;
                self.inner
                    .set_field_at(position, child)
                    .map_err(value_error)
            }
        }
    }

    /// Remove a nested child by position or by path, closing the gap.
    fn __delitem__(&mut self, key: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        match FieldKey::from_py(key)? {
            FieldKey::Path(path) => self
                .inner
                .remove_field_by_path(&path)
                .map(|_| ())
                .map_err(|_| PyKeyError::new_err(path)),
            FieldKey::Position(index) => {
                let position = normalize_index(index, self.inner.field_len())
                    .ok_or_else(|| PyIndexError::new_err(index))?;
                self.inner
                    .remove_field_at(position)
                    .map(|_| ())
                    .map_err(value_error)
            }
        }
    }

    /// Returns the datatype that holds both this one and `other`.
    ///
    /// `upscale` picks the direction width resolves in: the default meets at
    /// the type holding both, `False` at the tightest type naming both.
    #[pyo3(signature = (other, upscale=true))]
    fn merge_with(&self, other: &Bound<'_, PyAny>, upscale: bool) -> PyResult<Self> {
        let other = core_dtype_from_value(other)?;
        self.inner
            .merge_with(&other, upscale)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Every leaf under this node, named by its dotted path.
    ///
    /// Struct nesting flattens all the way down, and a leaf under a nullable
    /// ancestor is nullable. Collections are leaves: a list or a map is one
    /// column, and `explode_fields` is what reaches inside one. Every name
    /// this answers is one `field_by_path` resolves.
    fn unnest_fields(&self) -> Vec<PyField> {
        self.inner
            .unnest_fields()
            .into_iter()
            .map(PyField::from_inner)
            .collect()
    }

    /// This node's children with every collection replaced by what it holds.
    ///
    /// A list answers its item, a map its entries, a dictionary or run-end
    /// node the values it encodes, and anything else itself - so the result
    /// names the same columns in the same order. One level only, so the depth
    /// is the caller's decision.
    fn explode_fields(&self) -> Vec<PyField> {
        self.inner
            .explode_fields()
            .into_iter()
            .map(PyField::from_inner)
            .collect()
    }

    /// Returns the nested child at `index`, or `None`.
    ///
    /// Negative positions count from the end, as everywhere else.
    fn get_field_at(&self, index: isize) -> Option<PyField> {
        field_at_of(&self.inner, index)
            .ok()
            .map(|field| PyField::from_inner_with_read_only(field, self.children_read_only))
    }

    /// Returns the nested child `path` resolves to, or `None`.
    ///
    /// A child carrying the whole string wins before the string is decomposed
    /// on `.`, so a name containing a dot stays reachable.
    fn get_field_by_path(&self, path: &str) -> Option<PyField> {
        self.inner
            .get_field_by_path(path)
            .cloned()
            .map(|field| PyField::from_inner_with_read_only(field, self.children_read_only))
    }

    /// Returns the nested child a position or a path names, or `None`.
    #[pyo3(signature = (key=None, *, idx=None, path=None))]
    fn get_field(
        &self,
        key: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<Option<PyField>> {
        let found = match one_field_key(key, idx, path)? {
            FieldKey::Path(path) => self.inner.get_field_by_path(&path).cloned(),
            FieldKey::Position(index) => normalize_index(index, self.inner.field_len())
                .and_then(|at| self.inner.get_field_at(at).cloned()),
        };
        Ok(found.map(|field| PyField::from_inner_with_read_only(field, self.children_read_only)))
    }

    /// Returns the nested child at `index`.
    ///
    /// Raises `IndexError` when there is no child at that position.
    fn field_at(&self, index: isize) -> PyResult<PyField> {
        field_at_of(&self.inner, index)
            .map(|field| PyField::from_inner_with_read_only(field, self.children_read_only))
    }

    /// Returns the nested child `path` resolves to.
    ///
    /// Raises `KeyError` when no child carries that name and no decomposition
    /// of it resolves.
    fn field_by_path(&self, path: &str) -> PyResult<PyField> {
        field_by_path_of(&self.inner, path)
            .map(|field| PyField::from_inner_with_read_only(field, self.children_read_only))
    }

    /// Returns the nested child a position or a path names.
    ///
    /// The key may be positional, or named `idx=` or `path=`; naming more than
    /// one is a `TypeError` rather than a silent precedence rule.
    #[pyo3(signature = (key=None, *, idx=None, path=None))]
    fn field(
        &self,
        key: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<PyField> {
        let resolved = match one_field_key(key, idx, path)? {
            FieldKey::Path(path) => field_by_path_of(&self.inner, &path),
            FieldKey::Position(index) => field_at_of(&self.inner, index),
        }?;
        Ok(PyField::from_inner_with_read_only(
            resolved,
            self.children_read_only,
        ))
    }

    /// Replaces the nested child at `index`.
    fn set_field_at(&mut self, index: isize, child: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let child = crate::field::core_field_from_value(child)?;
        let position = normalize_index(index, self.inner.field_len())
            .ok_or_else(|| PyIndexError::new_err(index))?;
        self.inner
            .set_field_at(position, child)
            .map_err(value_error)
    }

    /// Replaces the nested child `path` resolves to, appending an unresolved
    /// name under it.
    fn set_field_by_path(&mut self, path: &str, child: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let child = crate::field::core_field_from_value(child)?;
        self.inner
            .set_field_by_path(path, child)
            .map_err(value_error)
    }

    /// Replaces the nested child a position or a path names.
    #[pyo3(signature = (key=None, child=None, *, idx=None, path=None))]
    fn set_field(
        &mut self,
        key: Option<&Bound<'_, PyAny>>,
        child: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<()> {
        let child = child.ok_or_else(|| PyTypeError::new_err("set_field() needs a child"))?;
        match one_field_key(key, idx, path)? {
            FieldKey::Path(path) => self.set_field_by_path(&path, child),
            FieldKey::Position(index) => self.set_field_at(index, child),
        }
    }

    /// Removes and returns the nested child at `index`.
    fn remove_field_at(&mut self, index: isize) -> PyResult<PyField> {
        self.require_mutable()?;
        let position = normalize_index(index, self.inner.field_len())
            .ok_or_else(|| PyIndexError::new_err(index))?;
        self.inner
            .remove_field_at(position)
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Removes and returns the nested child `path` resolves to.
    fn remove_field_by_path(&mut self, path: &str) -> PyResult<PyField> {
        self.require_mutable()?;
        self.inner
            .remove_field_by_path(path)
            .map(PyField::from_inner)
            .map_err(|_| PyKeyError::new_err(path.to_owned()))
    }

    /// Removes and returns the nested child a position or a path names.
    #[pyo3(signature = (key=None, *, idx=None, path=None))]
    fn remove_field(
        &mut self,
        key: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<PyField> {
        match one_field_key(key, idx, path)? {
            FieldKey::Path(path) => self.remove_field_by_path(&path),
            FieldKey::Position(index) => self.remove_field_at(index),
        }
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> bool {
        if let Ok(name) = value.extract::<&str>() {
            return self.inner.get_field_by_path(name).is_some();
        }
        if let Ok(index) = value.extract::<isize>() {
            return normalize_index(index, self.inner.field_len()).is_some();
        }
        if let Ok(field) = value.extract::<PyRef<'_, PyField>>() {
            return (0..self.inner.field_len())
                .filter_map(|index| self.inner.get_field(index))
                .any(|candidate| candidate == &field.inner);
        }
        false
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("DataType.from_str({:?})", self.inner.to_string())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// The enum an ASCII field's values name: one value per member name.
///
/// A dictionary is a vocabulary and derives its member names; this is the
/// vocabulary a declaration named itself, and it is what a ``Field`` stores
/// under ``field:enum`` so the enum crosses Arrow, a file, and another runtime
/// intact. The width lives in the field's datatype, so a member's code is its
/// packed ASCII value under that width and never a position.
#[pyclass(name = "AsciiEnum", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyAsciiEnum {
    inner: CoreAsciiEnum,
}

impl PyAsciiEnum {
    pub(crate) const fn from_inner(inner: CoreAsciiEnum) -> Self {
        Self { inner }
    }

    pub(crate) const fn as_inner(&self) -> &CoreAsciiEnum {
        &self.inner
    }
}

/// The members argument: a mapping of member name to value, or its pairs.
fn ascii_members_of(value: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    if let Ok(mapping) = value.cast::<PyDict>() {
        return mapping
            .iter()
            .map(|(member, value)| Ok((member.extract()?, value.extract()?)))
            .collect();
    }
    if value.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err(
            "enum members must be a mapping or its name and value pairs, not one string",
        ));
    }
    value
        .try_iter()?
        .map(|entry| entry?.extract::<(String, String)>())
        .collect()
}

#[pymethods]
impl PyAsciiEnum {
    // The members move, so equality cannot be frozen into a hash: a mutable
    // value follows Python's hash contract by having none.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Creates an enum from its members, one ASCII value per member name.
    #[new]
    #[pyo3(signature = (name, members=None))]
    fn new(name: &str, members: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let members = members
            .map(ascii_members_of)
            .transpose()?
            .unwrap_or_default();
        CoreAsciiEnum::from_members(name, members)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Parses the ``field:enum`` document.
    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        CoreAsciiEnum::from_json(document)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Renders the ``field:enum`` document, which is one text per enum.
    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> String {
        self.inner.into_json()
    }

    /// The enum's own name, which is not the field's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Every member by name, with the ASCII value it names.
    #[getter]
    fn members(&self) -> BTreeMap<&str, &str> {
        self.inner.iter().collect()
    }

    /// The ASCII value one member names, or ``None`` for a member it has not.
    fn get(&self, member: &str) -> Option<&str> {
        self.inner.get(member)
    }

    /// The first member naming one ASCII value, or ``None`` when none does.
    fn get_member(&self, value: &str) -> Option<&str> {
        self.inner.get_member(value)
    }

    /// Names one ASCII value and returns the value the member had.
    fn insert(&mut self, member: &str, value: &str) -> PyResult<Option<String>> {
        self.inner
            .insert(member, value)
            .map(|held| held.map(|held| held.to_string()))
            .map_err(value_error)
    }

    /// Removes one member and returns the ASCII value it named.
    fn remove(&mut self, member: &str) -> Option<String> {
        self.inner.remove(member).map(|value| value.to_string())
    }

    /// The members paired with their packed codes under one ASCII width.
    #[allow(clippy::wrong_self_convention)]
    fn into_members(&self, width: &Bound<'_, PyAny>) -> PyResult<Vec<(String, i128)>> {
        self.inner
            .into_members(&core_dtype_from_value(width)?)
            .map(|members| {
                members
                    .into_iter()
                    .map(|(member, code)| (member.to_string(), code))
                    .collect()
            })
            .map_err(value_error)
    }

    /// The vocabulary this enum names, as a dictionary over one width.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (width, key=None))]
    fn into_dictionary(
        &self,
        width: &Bound<'_, PyAny>,
        key: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyAsciiDictionary> {
        let inner = self
            .inner
            .into_dictionary(core_dtype_from_value(width)?)
            .map_err(value_error)?;
        PyAsciiDictionary::keyed(inner, key)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let members = PyList::new(py, self.inner.iter().map(|(member, _)| member))?;
        Ok(members.try_iter()?.into_any())
    }

    fn __contains__(&self, member: &str) -> bool {
        self.inner.get(member).is_some()
    }

    fn __repr__(&self) -> String {
        format!("AsciiEnum.from_json({:?})", self.inner.into_json())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        let answer = match operation {
            CompareOp::Eq => self.inner == other.inner,
            CompareOp::Ne => self.inner != other.inner,
            _ => return Ok(other.py().NotImplemented()),
        };
        Ok(answer
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Iterator over a datatype's direct child fields.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyDataTypeIterator {
    inner: CoreDataType,
    index: usize,
    read_only: bool,
}

impl PyDataTypeIterator {
    /// Iterate one node's children, from `Field` or from `DataType`.
    pub(crate) const fn over(inner: CoreDataType, read_only: bool) -> Self {
        Self {
            inner,
            index: 0,
            read_only,
        }
    }
}

#[pymethods]
impl PyDataTypeIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<PyField> {
        let field = slf.inner.get_field(slf.index)?.clone();
        slf.index += 1;
        Some(PyField::from_inner_with_read_only(field, slf.read_only))
    }

    fn __length_hint__(&self) -> usize {
        self.inner.field_len().saturating_sub(self.index)
    }
}

/// One ASCII dictionary value at the boundary: text, or the bytes storage holds.
///
/// Bytes-like by cast rather than by extracting `Vec<u8>`, which a list of
/// small integers would also satisfy. Nothing is decoded here: the bytes reach
/// the width's own rule, which trims the storage padding and names the width
/// when it refuses.
fn ascii_value_of(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(text.to_cow()?.as_bytes().to_vec());
    }
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(bytes) = value.cast::<PyByteArray>() {
        return Ok(bytes.to_vec());
    }
    Err(PyTypeError::new_err(
        "an ASCII dictionary value must be str or bytes",
    ))
}

/// The column argument of the two bulk entry points.
///
/// A bare `str` is one value, not the column of its characters, so it is
/// refused the way every other iterable-of-strings argument is.
fn ascii_column_of<'py>(values: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyIterator>> {
    if values.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err(
            "an ASCII column must be an iterable of values, not one string",
        ));
    }
    values.try_iter()
}

/// A per-column ASCII vocabulary and the codes that name its values.
///
/// The vocabulary is a value the caller carries, never a process-global
/// registry: a code is stable exactly as far as this object travels, and two
/// independent encodes agree only when the same dictionary crossed both.
/// Nothing in the write path registers on its own.
///
/// That code is a position. The other integer an ASCII value has is
/// ``DataType.ascii_packed``, its own storage bytes, which is the same
/// everywhere and is what ``into_intenum`` names its members by.
///
/// The first argument is the ASCII *width* the values are stored as; the
/// `values` property is the vocabulary itself, in the code order the generated
/// enum names.
#[pyclass(
    name = "AsciiDictionary",
    module = "yggdryl._native",
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyAsciiDictionary {
    inner: CoreAsciiDictionary,
}

impl PyAsciiDictionary {
    fn keyed(inner: CoreAsciiDictionary, key: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let key = match key {
            None => CoreDataType::Int32,
            Some(key) => core_dtype_from_value(key)?,
        };
        inner
            .with_key(key)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }
}

#[pymethods]
impl PyAsciiDictionary {
    // `push` moves the vocabulary, so equality cannot be frozen into a hash:
    // a mutable value follows Python's hash contract by having none.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Creates an empty vocabulary over an ASCII width, with `int32` keys.
    #[new]
    #[pyo3(signature = (values, key=None))]
    fn new(values: &Bound<'_, PyAny>, key: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let inner =
            CoreAsciiDictionary::new(core_dtype_from_value(values)?).map_err(value_error)?;
        Self::keyed(inner, key)
    }

    /// Creates a vocabulary pre-seeded in first-appearance order.
    #[staticmethod]
    #[pyo3(signature = (values, seen, key=None))]
    fn from_values(
        values: &Bound<'_, PyAny>,
        seen: &Bound<'_, PyAny>,
        key: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut dictionary = Self::new(values, key)?;
        for value in ascii_column_of(seen)? {
            dictionary.push(&value?)?;
        }
        Ok(dictionary)
    }

    /// Creates the vocabulary a registered logical name prebuilds, such as
    /// ``currency``, ``country``, or ``mic``.
    ///
    /// A registered name over an ASCII width with no prebuilt list answers an
    /// empty vocabulary of that width.
    #[staticmethod]
    #[pyo3(signature = (name, key=None))]
    fn from_logical_name(name: &str, key: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let inner = CoreAsciiDictionary::from_logical_name(name).map_err(value_error)?;
        Self::keyed(inner, key)
    }

    /// The prebuilt vocabularies, keyed by the logical name that spells them.
    #[staticmethod]
    fn prebuilt(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let lists = PyDict::new(py);
        for (name, values) in CoreAsciiDictionary::PREBUILT {
            lists.set_item(name, PyList::new(py, *values)?)?;
        }
        Ok(lists)
    }

    /// Recovers the vocabulary of a `PyArrow` dictionary array over an ASCII
    /// width, in the array's own value order.
    #[staticmethod]
    fn from_arrow_array(array: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = arrow_array_from_pyarrow(array)?;
        CoreAsciiDictionary::from_arrow_array(array.as_ref())
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// Registers a value and returns its code, existing or newly appended.
    fn push(&mut self, value: &Bound<'_, PyAny>) -> PyResult<i64> {
        self.inner
            .push_bytes(&ascii_value_of(value)?)
            .map_err(value_error)
    }

    /// The value a code names, or `None` when the vocabulary has no such code.
    fn get(&self, code: i64) -> Option<String> {
        self.inner.get(code).map(str::to_owned)
    }

    /// The code a value has, or `None` when it was never registered.
    fn get_code(&self, value: &Bound<'_, PyAny>) -> PyResult<Option<i64>> {
        // Bytes that are not text were never registered, so a lookup misses.
        let value = ascii_value_of(value)?;
        Ok(std::str::from_utf8(&value)
            .ok()
            .and_then(|value| self.inner.get_code(value)))
    }

    /// The vocabulary in code order.
    #[getter]
    fn values(&self) -> Vec<&str> {
        self.inner.as_values().iter().map(AsRef::as_ref).collect()
    }

    /// The datatype an encoded column carries: `dictionary(key, ascii-N)`.
    #[getter]
    fn dtype(&self) -> PyResult<PyDataType> {
        self.inner
            .dtype()
            .map(PyDataType::from_inner)
            .map_err(value_error)
    }

    /// The integer type the codes are read as.
    #[getter]
    fn key(&self) -> PyDataType {
        PyDataType::from_inner(self.inner.key().clone())
    }

    /// The ASCII width the values are stored as.
    #[getter]
    fn values_dtype(&self) -> PyDataType {
        PyDataType::from_inner(self.inner.values_dtype().clone())
    }

    /// Encodes a column into a `PyArrow` dictionary array over this vocabulary.
    ///
    /// Unseen values register in first-appearance order and `None` is a null
    /// key, so two calls on one dictionary answer two arrays whose codes agree.
    #[allow(clippy::wrong_self_convention)]
    fn into_arrow_array<'py>(
        &mut self,
        py: Python<'py>,
        values: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        // An Arrow holder answers its own values; anything else is iterated.
        let values = if values.hasattr("to_pylist")? {
            values.call_method0("to_pylist")?
        } else {
            values.clone()
        };
        let mut column = Vec::new();
        for value in ascii_column_of(&values)? {
            let value = value?;
            column.push(if value.is_none() {
                None
            } else {
                Some(ascii_value_of(&value)?)
            });
        }
        let array = self.inner.into_arrow_array(column).map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, None)
    }

    /// Builds an `enum.IntEnum` whose members are this vocabulary.
    ///
    /// The member names and their codes come from the core listing, so a
    /// member is its value packed big-endian and never its position: the same
    /// value names the same integer in every process and every vocabulary.
    #[allow(clippy::wrong_self_convention)]
    fn into_intenum<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        if name.trim().is_empty() {
            return Err(PyValueError::new_err(
                "AsciiDictionary.into_intenum needs a non-empty enum name",
            ));
        }
        let members = self.inner.into_members().map_err(value_error)?;
        let members = PyList::new(
            py,
            members
                .iter()
                .map(|(member, code)| (member.as_str(), *code)),
        )?;
        py.import("enum")?
            .getattr("IntEnum")?
            .call1((name, members))
    }

    /// The enum member name of one value, under the rule the generated enum
    /// applies to a whole vocabulary at once.
    #[staticmethod]
    fn member_name(value: &str) -> String {
        CoreAsciiDictionary::member_name(value).to_string()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let values = PyList::new(py, self.values())?;
        Ok(values.try_iter()?.into_any())
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> bool {
        matches!(self.get_code(value), Ok(Some(_)))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let values = PyList::new(py, self.values())?;
        Ok(format!(
            "AsciiDictionary.from_values({:?}, {}, key={:?})",
            self.inner.values_dtype().to_string(),
            values.repr()?.to_str()?,
            self.inner.key().to_string(),
        ))
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        let answer = match operation {
            CompareOp::Eq => self.inner == other.inner,
            CompareOp::Ne => self.inner != other.inner,
            _ => return Ok(other.py().NotImplemented()),
        };
        Ok(answer
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
