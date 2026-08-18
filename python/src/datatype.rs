//! Native Python view of Yggdryl datatypes.

use std::num::IntErrorKind;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch as ArrowRecordBatch, ffi::to_ffi, make_array};
use arrow_data::ArrayData;
use arrow_pyarrow::{FromPyArrow, PyArrowType, ToPyArrow};
use arrow_schema::{DataType as ArrowDataType, ffi::FFI_ArrowSchema};
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList};
use yggdryl::ArrowCast;
use yggdryl::{
    DataType as CoreDataType, Scheme as CoreScheme, TimeUnit as CoreTimeUnit,
    UnionMode as CoreUnionMode,
};

use crate::field::PyField;
use crate::{PyDifferenceIterator, child_of, compare, normalize_index, value_error};

fn import_ffi_schema<'py>(
    py: Python<'py>,
    class_name: &str,
    schema: &FFI_ArrowSchema,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("pyarrow")?
        .getattr(class_name)?
        .call_method1("_import_from_c", (std::ptr::from_ref(schema) as usize,))
}

pub(crate) fn core_data_type_to_pyarrow<'py>(
    py: Python<'py>,
    data_type: &CoreDataType,
) -> PyResult<Bound<'py, PyAny>> {
    let schema = data_type.to_arrow_ffi().map_err(value_error)?;
    import_ffi_schema(py, "DataType", &schema)
}

pub(crate) fn core_field_to_pyarrow<'py>(
    py: Python<'py>,
    field: &yggdryl::Field,
) -> PyResult<Bound<'py, PyAny>> {
    let schema = field.to_arrow_ffi().map_err(value_error)?;
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
    let (ffi_array, _data_type_schema) = to_ffi(&data).map_err(value_error)?;
    let ffi_field = field.to_arrow_ffi().map_err(value_error)?;
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
    let (ffi_array, ffi_data_type) = to_ffi(&data).map_err(value_error)?;
    let array_class = py.import("pyarrow")?.getattr("Array")?;
    if let Some(field) = field {
        let ffi_field = field.to_arrow_ffi().map_err(value_error)?;
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
            std::ptr::from_ref(&ffi_data_type) as usize,
        ),
    )
}

/// Constructs or safely casts one `PyArrow` scalar to an exact Arrow datatype.
///
/// Keeping this helper beside the C Schema projection gives `DataType`,
/// `Field`, and the records adapter one conversion contract without adding an
/// Arrow array/value dependency to the core crate.
pub(crate) fn arrow_scalar_from_core_type<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    data_type: &CoreDataType,
    safe: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let target = core_data_type_to_pyarrow(py, data_type)?;
    arrow_scalar_to_pyarrow_type(py, value, target, safe)
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
        Err(scalar_error) => {
            // PyArrow exposes a few valid scalar layouts only through an
            // array slot (currently run-end encoding is the important case).
            let values = PyList::new(py, [value])?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("type", target)?;
            kwargs.set_item("safe", safe)?;
            match pyarrow.getattr("array")?.call((values,), Some(&kwargs)) {
                Ok(array) => array.get_item(0),
                Err(_) => Err(scalar_error),
            }
        }
    }
}

pub(crate) fn core_data_type_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreDataType> {
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

    core_data_type_from_pyhint(value)
}

fn core_data_type_from_pyhint(hint: &Bound<'_, PyAny>) -> PyResult<CoreDataType> {
    let inferred = hint
        .py()
        .import("yggdryl.records._hints")?
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

/// A cheaply cloned, immutable Yggdryl logical datatype.
#[pyclass(
    name = "DataType",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyDataType {
    pub(crate) inner: CoreDataType,
}

impl PyDataType {
    fn from_validated(inner: CoreDataType) -> PyResult<Self> {
        inner.validate().map_err(value_error)?;
        Ok(Self { inner })
    }
}

#[pymethods]
impl PyDataType {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_data_type_from_value(value).map(|inner| Self { inner })
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
            "duration" => {
                if !unit.is_temporal() {
                    return Err(PyValueError::new_err(
                        "duration requires a temporal resolution unit",
                    ));
                }
                CoreDataType::Duration(unit)
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

    /// Builds the canonical dense Union with sequential native type IDs.
    #[staticmethod]
    fn variant(fields: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner =
            CoreDataType::variant(core_fields_from_iterable(fields)?).map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Internal Dictionary constructor preserving exact native datatypes.
    #[staticmethod]
    fn _dictionary(key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner = CoreDataType::dictionary(
            core_data_type_from_value(key)?,
            core_data_type_from_value(value)?,
        )
        .map_err(value_error)?;
        Self::from_validated(inner)
    }

    /// Internal allocation-free dictionary value view for record inference.
    fn _dictionary_value_type(&self) -> PyResult<Self> {
        match &self.inner {
            CoreDataType::Dictionary(dictionary) => Ok(Self {
                inner: dictionary.value().clone(),
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
            .map(|inner| Self { inner })
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
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// Infers a native datatype from a Python type annotation.
    #[staticmethod]
    fn from_pyhint(hint: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_data_type_from_pyhint(hint).map(|inner| Self { inner })
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreDataType::from_str(value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_arrow(value: PyArrowType<ArrowDataType>) -> PyResult<Self> {
        let PyArrowType(value) = value;
        CoreDataType::try_from(value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// Builds a native Struct directly from native child fields.
    #[staticmethod]
    fn from_fields(fields: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreDataType::from_fields(core_fields_from_iterable(fields)?)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreDataType::from_json(value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    fn to_arrow<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        core_data_type_to_pyarrow(py, &self.inner)
    }

    /// Returns the cached Python annotation corresponding to this datatype.
    fn default_pyhint<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let data_type = Py::new(py, self.clone())?;
        py.import("yggdryl.records._defaults")?
            .getattr("_default_pyhint_from_datatype")?
            .call1((data_type,))
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
        let data_type = Py::new(py, self.clone())?;
        py.import("yggdryl.records._defaults")?
            .getattr("_default_pyvalue_from_datatype")?
            .call1((data_type, scalar))
    }

    /// Returns a recursively normalized datatype for a named compatibility target.
    fn to_scheme_compat(&self, target: &str) -> PyResult<Self> {
        let target = CoreScheme::from_str(target).map_err(value_error)?;
        self.inner
            .to_scheme_compat(&target)
            .map(|inner| Self { inner })
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
        core_data_type_to_pyarrow(py, &self.inner)
    }

    fn to_json(&self) -> PyResult<String> {
        self.inner.to_json().map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
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

    /// Internal record-casting view of temporal resolution. Keeping this on
    /// the native value avoids projecting a `PyArrow` datatype for every cell.
    fn _time_unit(&self) -> Option<&'static str> {
        match &self.inner {
            CoreDataType::Timestamp(unit, _)
            | CoreDataType::Time32(unit)
            | CoreDataType::Time64(unit)
            | CoreDataType::Duration(unit) => Some(unit.as_str()),
            _ => None,
        }
    }

    /// Internal record-casting view of a timestamp timezone.
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

    /// Internal record-casting view of fixed-size-list arity.
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
        PyDifferenceIterator::from_data_types(
            &self.inner,
            &other.inner,
            with_metadata,
            return_equal,
        )
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
        child_of(&self.inner, key).map(PyField::from_inner)
    }

    /// Refuse child mutation: a `DataType` is a read-only child collection.
    ///
    /// Reading is shared with `Field` - `data_type["price"]` and
    /// `field["price"]` both reach a nested child - but *writing* belongs on
    /// `Field`, which owns the cache-aware mutation the core requires and the
    /// metadata a datatype does not carry. A `DataType` is immutable and
    /// hashable; letting a subscript rewrite one would break both.
    fn __setitem__(&self, _key: &Bound<'_, PyAny>, _value: &Bound<'_, PyAny>) -> PyResult<()> {
        Err(PyTypeError::new_err(
            "a DataType is a read-only child collection; mutate children on the Field that \
             carries it",
        ))
    }

    /// Refuse child removal, for the reason `__setitem__` gives.
    fn __delitem__(&self, _key: &Bound<'_, PyAny>) -> PyResult<()> {
        Err(PyTypeError::new_err(
            "a DataType is a read-only child collection; mutate children on the Field that \
             carries it",
        ))
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> bool {
        if let Ok(name) = value.extract::<&str>() {
            return self.inner.get_field_by_name(name).is_some();
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

    fn __hash__(&self) -> u64 {
        self.inner.stable_hash()
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

/// Iterator over a datatype's direct child fields.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyDataTypeIterator {
    inner: CoreDataType,
    index: usize,
}

impl PyDataTypeIterator {
    /// Iterate one node's children, from `Field` or from `DataType`.
    pub(crate) const fn over(inner: CoreDataType) -> Self {
        Self { inner, index: 0 }
    }
}

#[pymethods]
impl PyDataTypeIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<PyField> {
        let field = slf.inner.get_field(slf.index)?.clone();
        slf.index += 1;
        Some(PyField::from_inner(field))
    }

    fn __length_hint__(&self) -> usize {
        self.inner.field_len().saturating_sub(self.index)
    }
}
