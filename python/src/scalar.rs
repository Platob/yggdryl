//! Python's native view of the shared [`Scalar`] tree.
//!
//! [`PyScalar`] owns only the core value. The conversion helpers here are also
//! the one boundary used by codecs, expressions, and record adapters.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_pyarrow::{FromPyArrow, IntoPyArrow};
use pyo3::PyTypeInfo;
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{
    PyArithmeticError, PyIndexError, PyKeyError, PyOverflowError, PyTypeError, PyValueError,
    PyZeroDivisionError,
};
use pyo3::prelude::*;
use pyo3::types::{
    PyAny, PyBool, PyByteArray, PyBytes, PyComplex, PyDict, PyFloat, PyFrozenSet, PyInt, PyList,
    PyMemoryView, PyModule, PySet, PyString, PyTuple, PyType,
};
use yggdryl::arrow::{
    array_from_value, array_to_value, batch_from_value, batch_to_value, scalar_array, scalar_value,
};
use yggdryl::{
    ArrowCast, DataType as CoreDataType, EnumScalar, Error as CoreError, Field as CoreField,
    Float16, Float32, Float64, I256, Scalar, TimeUnit, Timezone,
};

use crate::datatype::{PyDataType, arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::field::{PyField, core_field_from_value};
use crate::record::core_root_field_from_value;
use crate::timezone::core_timezone_from_value;
use crate::uri::{PyUri, PyUrl, PyUrn};
use crate::{compare, value_error};

/// How deep a Python graph may nest before conversion refuses to recurse.
const MAX_PYTHON_DEPTH: usize = 128;

/// The day `date.toordinal` counts from, expressed as a Unix epoch offset.
///
/// `date(1970, 1, 1).toordinal()` is this number, so subtracting it turns
/// Python's proleptic Gregorian ordinal into the epoch day count a value holds.
const EPOCH_ORDINAL: i64 = 719_163;

/// The microseconds one whole day holds, the unit every temporal crosses in.
const MICROSECONDS_PER_DAY: i64 = 86_400_000_000;

/// Native Python wrapper over the shared Rust value tree.
#[derive(Clone)]
#[pyclass(
    name = "Scalar",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyScalar {
    pub(crate) inner: Scalar,
}

impl PyScalar {
    pub(crate) const fn from_inner(inner: Scalar) -> Self {
        Self { inner }
    }

    fn child_at(&self, index: isize) -> Option<&Scalar> {
        let length = self.inner.as_sequence()?.len();
        let index = if index < 0 {
            length.checked_sub(index.unsigned_abs())?
        } else {
            usize::try_from(index).ok()?
        };
        self.inner.get(index)
    }

    fn child_for_key<'value>(
        &'value self,
        key: &Bound<'_, PyAny>,
    ) -> PyResult<Option<&'value Scalar>> {
        if self.inner.as_record().is_some() {
            let key = key
                .cast::<PyString>()
                .map_err(|_| PyTypeError::new_err("record keys must be str"))?;
            return Ok(self.inner.get_key_str(key.to_str()?));
        }
        if self.inner.as_mapping().is_some() {
            let key = from_py(key)?;
            return Ok(self.inner.get_key(&key));
        }
        Err(PyTypeError::new_err(format!(
            "{} values do not have keys",
            self.inner.kind()
        )))
    }
}

fn time_unit(value: &str) -> PyResult<TimeUnit> {
    TimeUnit::from_str(value).map_err(value_error)
}

fn timezone_or_naive(value: Option<&Bound<'_, PyAny>>) -> PyResult<Timezone> {
    value.map_or(Ok(Timezone::NAIVE), core_timezone_from_value)
}

fn i256_from_py(value: &Bound<'_, PyAny>) -> PyResult<I256> {
    if value.is_instance_of::<PyBool>()
        || !(value.is_instance_of::<PyInt>() || value.is_instance_of::<PyString>())
    {
        return Err(PyTypeError::new_err(
            "a D256 coefficient must be an integer or base-10 integer string",
        ));
    }
    value
        .str()?
        .to_str()?
        .parse::<I256>()
        .map_err(|error| PyOverflowError::new_err(error.to_string()))
}

fn arrow_scalar_into_array(value: &Bound<'_, PyAny>) -> PyResult<ArrayRef> {
    ensure_pyarrow_instance(value, "Scalar")?;
    let py = value.py();
    let values = PyList::new(py, [value])?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("type", value.getattr("type")?)?;
    let array = py
        .import("pyarrow")?
        .getattr("array")?
        .call((values,), Some(&kwargs))?;
    arrow_array_from_pyarrow(&array)
}

fn ensure_pyarrow_instance(value: &Bound<'_, PyAny>, class: &str) -> PyResult<()> {
    if value.is_instance(&value.py().import("pyarrow")?.getattr(class)?)? {
        Ok(())
    } else {
        Err(PyTypeError::new_err(format!(
            "expected a pyarrow.{class}, got {}",
            value.get_type().name()?
        )))
    }
}

fn exact_or_inferred_array_field(
    field: Option<&Bound<'_, PyAny>>,
    array: &ArrayRef,
    name: &str,
) -> PyResult<CoreField> {
    if let Some(field) = field {
        return core_field_from_value(field);
    }
    let data_type = CoreDataType::try_from(array.data_type().clone()).map_err(value_error)?;
    Ok(CoreField::new(name, data_type, array.null_count() != 0))
}

fn extend_rows(rows: &mut Vec<Scalar>, value: &Scalar) -> PyResult<()> {
    let values = value.as_sequence().ok_or_else(|| {
        PyValueError::new_err("Arrow record conversion must produce an outer Sequence")
    })?;
    rows.extend(values.iter().cloned());
    Ok(())
}

fn value_into_arrow_array(field: &CoreField, value: &Scalar) -> PyResult<ArrayRef> {
    array_from_value(field, value).map_err(value_error)
}

fn value_into_arrow_batch(field: &CoreField, value: &Scalar) -> PyResult<RecordBatch> {
    batch_from_value(field, value).map_err(value_error)
}

/// Preserve the arithmetic failure categories Python's numeric protocol uses.
fn arithmetic_error(error: &CoreError) -> PyErr {
    let message = error.to_string();
    match error {
        CoreError::InvalidArithmetic { .. } => PyTypeError::new_err(message),
        CoreError::ArithmeticOverflow { .. } => PyOverflowError::new_err(message),
        CoreError::DivisionByZero { .. } => PyZeroDivisionError::new_err(message),
        CoreError::InexactArithmetic { .. } => PyArithmeticError::new_err(message),
        _ => PyValueError::new_err(message),
    }
}

/// Convert one inferred Python operand and run one core checked operation.
fn binary_arithmetic(
    left: &Scalar,
    right: &Bound<'_, PyAny>,
    operation: fn(&Scalar, &Scalar) -> yggdryl::Result<Scalar>,
) -> PyResult<PyScalar> {
    operation(left, &from_py(right)?)
        .map(PyScalar::from_inner)
        .map_err(|error| arithmetic_error(&error))
}

/// Run one reflected operation with the inferred Python value on the left.
fn reflected_arithmetic(
    left: &Bound<'_, PyAny>,
    right: &Scalar,
    operation: fn(&Scalar, &Scalar) -> yggdryl::Result<Scalar>,
) -> PyResult<PyScalar> {
    operation(&from_py(left)?, right)
        .map(PyScalar::from_inner)
        .map_err(|error| arithmetic_error(&error))
}

/// Build one Python tuple used only by the exact pickle/repr protocol.
fn pickle_tuple(py: Python<'_>, items: Vec<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    Ok(PyTuple::new(py, items)?.into_any().unbind())
}

/// Tag one exact native payload without changing the public text codecs.
fn tagged_pickle_state(
    py: Python<'_>,
    tag: &str,
    payload: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    let mut items = vec![PyString::new(py, tag).into_any().unbind()];
    items.extend(payload);
    pickle_tuple(py, items)
}

fn decimal_pickle_state(
    py: Python<'_>,
    tag: &str,
    coefficient: &str,
    scale: i8,
) -> PyResult<Py<PyAny>> {
    let coefficient = PyString::new(py, coefficient).into_any().unbind();
    let scale = scale.into_pyobject(py)?.clone().into_any().unbind();
    tagged_pickle_state(py, tag, Some(pickle_tuple(py, vec![coefficient, scale])?))
}

fn temporal_i32_pickle_state(
    py: Python<'_>,
    tag: &str,
    count: i32,
    unit: TimeUnit,
    zone: &Timezone,
) -> PyResult<Py<PyAny>> {
    let count = count.into_pyobject(py)?.clone().into_any().unbind();
    let unit = PyString::new(py, unit.as_str()).into_any().unbind();
    let zone = PyString::new(py, zone.as_str()).into_any().unbind();
    tagged_pickle_state(py, tag, Some(pickle_tuple(py, vec![count, unit, zone])?))
}

fn temporal_i64_pickle_state(
    py: Python<'_>,
    tag: &str,
    count: i64,
    unit: TimeUnit,
    zone: &Timezone,
) -> PyResult<Py<PyAny>> {
    let count = count.into_pyobject(py)?.clone().into_any().unbind();
    let unit = PyString::new(py, unit.as_str()).into_any().unbind();
    let zone = PyString::new(py, zone.as_str()).into_any().unbind();
    tagged_pickle_state(py, tag, Some(pickle_tuple(py, vec![count, unit, zone])?))
}

/// Convert every `Scalar` variant into a lossless, Python-pickle-safe tree.
#[allow(clippy::too_many_lines)] // One exhaustive match keeps the private wire auditable.
pub(crate) fn scalar_pickle_state(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    macro_rules! scalar {
        ($tag:literal, $value:expr) => {{
            let payload = ($value).into_pyobject(py)?.to_owned().into_any().unbind();
            tagged_pickle_state(py, $tag, Some(payload))
        }};
    }

    match value {
        Scalar::Null => tagged_pickle_state(py, "null", None),
        Scalar::Bool(value) => scalar!("bool", *value),
        Scalar::I8(value) => scalar!("i8", *value),
        Scalar::I16(value) => scalar!("i16", *value),
        Scalar::I32(value) => scalar!("i32", *value),
        Scalar::I64(value) => scalar!("i64", *value),
        Scalar::U8(value) => scalar!("u8", *value),
        Scalar::U16(value) => scalar!("u16", *value),
        Scalar::U32(value) => scalar!("u32", *value),
        Scalar::U64(value) => scalar!("u64", *value),
        Scalar::I128(value) => scalar!("i128", *value),
        Scalar::U128(value) => scalar!("u128", *value),
        Scalar::F16(value) => scalar!("f16", value.as_f16().to_bits()),
        Scalar::F32(value) => scalar!("f32", value.as_f32().to_bits()),
        Scalar::F64(value) => scalar!("f64", value.as_f64().to_bits()),
        Scalar::D128(coefficient, scale) => {
            decimal_pickle_state(py, "d128", &coefficient.to_string(), *scale)
        }
        Scalar::D256(coefficient, scale) => {
            decimal_pickle_state(py, "d256", &coefficient.to_string(), *scale)
        }
        Scalar::String(value) => tagged_pickle_state(
            py,
            "string",
            Some(PyString::new(py, value).into_any().unbind()),
        ),
        Scalar::Enum(value) => tagged_pickle_state(
            py,
            "enum",
            Some(pickle_tuple(
                py,
                vec![
                    PyString::new(py, value.kind()).into_any().unbind(),
                    PyString::new(py, value.as_str()).into_any().unbind(),
                ],
            )?),
        ),
        Scalar::Bytes(value) => tagged_pickle_state(
            py,
            "bytes",
            Some(PyBytes::new(py, value).into_any().unbind()),
        ),
        Scalar::Geospatial(value) => tagged_pickle_state(
            py,
            "geospatial",
            Some(PyBytes::new(py, value).into_any().unbind()),
        ),
        Scalar::Date32(count, unit, zone) => {
            temporal_i32_pickle_state(py, "date32", *count, *unit, zone)
        }
        Scalar::Date64(count, unit, zone) => {
            temporal_i64_pickle_state(py, "date64", *count, *unit, zone)
        }
        Scalar::Time32(count, unit, zone) => {
            temporal_i32_pickle_state(py, "time32", *count, *unit, zone)
        }
        Scalar::Time64(count, unit, zone) => {
            temporal_i64_pickle_state(py, "time64", *count, *unit, zone)
        }
        Scalar::DateTime64(count, unit, zone) => {
            temporal_i64_pickle_state(py, "datetime64", *count, *unit, zone)
        }
        Scalar::Duration32(count, unit, zone) => {
            temporal_i32_pickle_state(py, "duration32", *count, *unit, zone)
        }
        Scalar::Duration64(count, unit, zone) => {
            temporal_i64_pickle_state(py, "duration64", *count, *unit, zone)
        }
        Scalar::Sequence(values) => {
            let values = values
                .iter()
                .map(|value| scalar_pickle_state(py, value))
                .collect::<PyResult<Vec<_>>>()?;
            tagged_pickle_state(py, "sequence", Some(pickle_tuple(py, values)?))
        }
        Scalar::Mapping(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| {
                    pickle_tuple(
                        py,
                        vec![
                            scalar_pickle_state(py, key)?,
                            scalar_pickle_state(py, value)?,
                        ],
                    )
                })
                .collect::<PyResult<Vec<_>>>()?;
            tagged_pickle_state(py, "mapping", Some(pickle_tuple(py, entries)?))
        }
        Scalar::Record(entries) => {
            let entries = entries
                .iter()
                .map(|(name, value)| {
                    pickle_tuple(
                        py,
                        vec![
                            PyString::new(py, name).into_any().unbind(),
                            scalar_pickle_state(py, value)?,
                        ],
                    )
                })
                .collect::<PyResult<Vec<_>>>()?;
            tagged_pickle_state(py, "record", Some(pickle_tuple(py, entries)?))
        }
    }
}

fn pickle_bytes(payload: &Bound<'_, PyAny>) -> PyResult<Arc<[u8]>> {
    payload
        .cast::<PyBytes>()
        .map(|bytes| Arc::from(bytes.as_bytes()))
        .map_err(|_| PyTypeError::new_err("Scalar byte state must be bytes"))
}

fn pickle_decimal(payload: &Bound<'_, PyAny>) -> PyResult<(String, i8)> {
    payload
        .extract::<(String, i8)>()
        .map_err(|_| PyTypeError::new_err("Scalar decimal state must be (coefficient, scale)"))
}

fn pickle_temporal<T>(payload: &Bound<'_, PyAny>) -> PyResult<(T, TimeUnit, Timezone)>
where
    for<'py> T: FromPyObject<'py, 'py>,
{
    let (count, unit, zone) = payload.extract::<(T, String, String)>().map_err(|_| {
        PyTypeError::new_err("Scalar temporal state must be (count, unit, timezone)")
    })?;
    Ok((
        count,
        time_unit(&unit)?,
        Timezone::from_str(&zone).map_err(value_error)?,
    ))
}

/// Rebuild one exact native scalar from the private pickle/repr state.
#[allow(clippy::too_many_lines)] // One exhaustive match keeps the private wire auditable.
pub(crate) fn scalar_from_pickle_state(state: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
    if depth > MAX_PYTHON_DEPTH {
        return Err(PyValueError::new_err(format!(
            "Scalar pickle state exceeds {MAX_PYTHON_DEPTH} levels"
        )));
    }
    let state = state
        .cast::<PyTuple>()
        .map_err(|_| PyTypeError::new_err("Scalar pickle state must be a tagged tuple"))?;
    if state.is_empty() || state.len() > 2 {
        return Err(PyValueError::new_err(
            "Scalar pickle state must contain a tag and optional payload",
        ));
    }
    let tag = state.get_item(0)?.extract::<String>()?;
    let payload = || -> PyResult<Bound<'_, PyAny>> {
        if state.len() == 2 {
            state.get_item(1)
        } else {
            Err(PyValueError::new_err(format!(
                "Scalar pickle tag {tag:?} requires a payload"
            )))
        }
    };

    match tag.as_str() {
        "null" if state.len() == 1 => Ok(Scalar::Null),
        "null" => Err(PyValueError::new_err("null Scalar state has no payload")),
        "bool" => payload()?.extract::<bool>().map(Scalar::Bool),
        "i8" => payload()?.extract::<i8>().map(Scalar::I8),
        "i16" => payload()?.extract::<i16>().map(Scalar::I16),
        "i32" => payload()?.extract::<i32>().map(Scalar::I32),
        "i64" => payload()?.extract::<i64>().map(Scalar::I64),
        "u8" => payload()?.extract::<u8>().map(Scalar::U8),
        "u16" => payload()?.extract::<u16>().map(Scalar::U16),
        "u32" => payload()?.extract::<u32>().map(Scalar::U32),
        "u64" => payload()?.extract::<u64>().map(Scalar::U64),
        "i128" => payload()?.extract::<i128>().map(Scalar::I128),
        "u128" => payload()?.extract::<u128>().map(Scalar::U128),
        "f16" => payload()?
            .extract::<u16>()
            .map(|bits| Scalar::F16(Float16::from_f16(half::f16::from_bits(bits)))),
        "f32" => payload()?
            .extract::<u32>()
            .map(|bits| Scalar::F32(Float32::from_f32(f32::from_bits(bits)))),
        "f64" => payload()?
            .extract::<u64>()
            .map(|bits| Scalar::F64(Float64::from_f64(f64::from_bits(bits)))),
        "d128" => {
            let (coefficient, scale) = pickle_decimal(&payload()?)?;
            coefficient
                .parse::<i128>()
                .map(|coefficient| Scalar::d128(coefficient, scale))
                .map_err(|_| PyOverflowError::new_err("D128 coefficient is out of range"))
        }
        "d256" => {
            let (coefficient, scale) = pickle_decimal(&payload()?)?;
            coefficient
                .parse::<I256>()
                .map(|coefficient| Scalar::d256(coefficient, scale))
                .map_err(|error| PyOverflowError::new_err(error.to_string()))
        }
        "string" => payload()?.extract::<String>().map(Scalar::from),
        "enum" => {
            let (kind, value) = payload()?.extract::<(String, String)>()?;
            EnumScalar::from_parts(&kind, &value)
                .map(Scalar::Enum)
                .map_err(value_error)
        }
        "bytes" => pickle_bytes(&payload()?).map(Scalar::Bytes),
        "geospatial" => pickle_bytes(&payload()?).map(Scalar::Geospatial),
        "date32" => {
            let (count, unit, zone) = pickle_temporal::<i32>(&payload()?)?;
            Scalar::date32_in(count, unit, zone).map_err(value_error)
        }
        "date64" => {
            let (count, unit, zone) = pickle_temporal::<i64>(&payload()?)?;
            Scalar::date64_in(count, unit, zone).map_err(value_error)
        }
        "time32" => {
            let (count, unit, zone) = pickle_temporal::<i32>(&payload()?)?;
            Scalar::time32(count, unit, zone).map_err(value_error)
        }
        "time64" => {
            let (count, unit, zone) = pickle_temporal::<i64>(&payload()?)?;
            Scalar::time64(count, unit, zone).map_err(value_error)
        }
        "datetime64" => {
            let (count, unit, zone) = pickle_temporal::<i64>(&payload()?)?;
            Scalar::datetime64(count, unit, zone).map_err(value_error)
        }
        "duration32" => {
            let (count, unit, zone) = pickle_temporal::<i32>(&payload()?)?;
            Scalar::duration32_in(count, unit, zone).map_err(value_error)
        }
        "duration64" => {
            let (count, unit, zone) = pickle_temporal::<i64>(&payload()?)?;
            Scalar::duration64_in(count, unit, zone).map_err(value_error)
        }
        "sequence" => {
            let payload = payload()?;
            let values = payload
                .cast::<PyTuple>()
                .map_err(|_| PyTypeError::new_err("Scalar sequence state must be a tuple"))?;
            values
                .iter()
                .map(|value| scalar_from_pickle_state(&value, depth + 1))
                .collect::<PyResult<Vec<_>>>()
                .map(Scalar::from_sequence)
        }
        "mapping" => {
            let payload = payload()?;
            let entries = payload
                .cast::<PyTuple>()
                .map_err(|_| PyTypeError::new_err("Scalar mapping state must be a tuple"))?;
            let entries = entries
                .iter()
                .map(|entry| {
                    let entry = entry.cast::<PyTuple>().map_err(|_| {
                        PyTypeError::new_err("Scalar mapping entries must be pairs")
                    })?;
                    if entry.len() != 2 {
                        return Err(PyValueError::new_err(
                            "Scalar mapping entries must have length two",
                        ));
                    }
                    Ok((
                        scalar_from_pickle_state(&entry.get_item(0)?, depth + 1)?,
                        scalar_from_pickle_state(&entry.get_item(1)?, depth + 1)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Scalar::from_mapping(entries).map_err(value_error)
        }
        "record" => {
            let payload = payload()?;
            let entries = payload
                .cast::<PyTuple>()
                .map_err(|_| PyTypeError::new_err("Scalar record state must be a tuple"))?;
            let entries = entries
                .iter()
                .map(|entry| {
                    let entry = entry
                        .cast::<PyTuple>()
                        .map_err(|_| PyTypeError::new_err("Scalar record entries must be pairs"))?;
                    if entry.len() != 2 {
                        return Err(PyValueError::new_err(
                            "Scalar record entries must have length two",
                        ));
                    }
                    Ok((
                        entry.get_item(0)?.extract::<String>()?,
                        scalar_from_pickle_state(&entry.get_item(1)?, depth + 1)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Scalar::from_record(entries).map_err(value_error)
        }
        _ => Err(PyValueError::new_err(format!(
            "unknown Scalar pickle tag {tag:?}"
        ))),
    }
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
impl PyScalar {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        from_py(value).map(Self::from_inner)
    }

    /// Rebuild exact private state used by pickle and reconstructible repr.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyAny>) -> PyResult<Self> {
        scalar_from_pickle_state(state, 0).map(Self::from_inner)
    }

    /// Convert a Python-native value without a text intermediate.
    #[staticmethod]
    fn from_py(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        from_py(value).map(Self::from_inner)
    }

    /// Build an identity-preserving member of a core enum.
    #[staticmethod]
    fn from_enum(kind: &str, value: &str) -> PyResult<Self> {
        EnumScalar::from_parts(kind, value)
            .map(Scalar::from)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    fn f16(value: f64) -> Self {
        Self::from_inner(Scalar::F16(Float16::from_f16(half::f16::from_f64(value))))
    }

    #[staticmethod]
    fn f32(value: f32) -> Self {
        Self::from_inner(Scalar::F32(Float32::from_f32(value)))
    }

    #[staticmethod]
    fn f64(value: f64) -> Self {
        Self::from_inner(Scalar::F64(Float64::from_f64(value)))
    }

    #[staticmethod]
    #[pyo3(signature = (coefficient, scale=0))]
    fn d128(coefficient: i128, scale: i8) -> Self {
        Self::from_inner(Scalar::d128(coefficient, scale))
    }

    #[staticmethod]
    #[pyo3(signature = (coefficient, scale=0))]
    fn d256(coefficient: &Bound<'_, PyAny>, scale: i8) -> PyResult<Self> {
        Ok(Self::from_inner(Scalar::d256(
            i256_from_py(coefficient)?,
            scale,
        )))
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit="d", timezone=None))]
    fn date32(count: i32, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::date32_in(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit="ms", timezone=None))]
    fn date64(count: i64, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::date64_in(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit, timezone=None))]
    fn time32(count: i32, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::time32(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit, timezone=None))]
    fn time64(count: i64, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::time64(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit, timezone=None))]
    fn datetime64(count: i64, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::datetime64(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit, timezone=None))]
    fn duration32(count: i32, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::duration32_in(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (count, unit, timezone=None))]
    fn duration64(count: i64, unit: &str, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Scalar::duration64_in(count, time_unit(unit)?, timezone_or_naive(timezone)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Decode one `PyArrow` scalar through Arrow C Data.
    #[staticmethod]
    #[pyo3(signature = (value, field=None))]
    fn from_arrow_scalar(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let input = arrow_scalar_into_array(value)?;
        let field = exact_or_inferred_array_field(field, &input, "value")?;
        let input = field.cast_arrow_array(input, true).map_err(value_error)?;
        scalar_value(&field, input.as_ref())
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Decode one `PyArrow` array as an outer Sequence.
    #[staticmethod]
    #[pyo3(signature = (value, field=None))]
    fn from_arrow_array(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let input = arrow_array_from_pyarrow(value)?;
        let field = exact_or_inferred_array_field(field, &input, "item")?;
        let input = field.cast_arrow_array(input, true).map_err(value_error)?;
        array_to_value(&field, input.as_ref())
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Decode one `PyArrow` `RecordBatch` as an outer Sequence of rows.
    #[staticmethod]
    #[pyo3(signature = (value, field=None))]
    fn from_arrow_record_batch(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let batch = RecordBatch::from_pyarrow_bound(value)?;
        let batch = match field {
            Some(field) => core_root_field_from_value(field, "row")?
                .cast_arrow_batch(batch, true)
                .map_err(value_error)?,
            None => batch,
        };
        batch_to_value(&batch)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Decode one `PyArrow` Table through its Arrow C stream.
    #[staticmethod]
    #[pyo3(signature = (value, field=None))]
    fn from_arrow_table(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        ensure_pyarrow_instance(value, "Table")?;
        let root = field
            .map(|field| core_root_field_from_value(field, "row"))
            .transpose()?;
        let mut reader =
            arrow_array::ffi_stream::ArrowArrayStreamReader::from_pyarrow_bound(value)?;
        let mut rows = Vec::new();
        for batch in &mut reader {
            let batch = batch.map_err(value_error)?;
            let batch = match &root {
                Some(root) => root.cast_arrow_batch(batch, true).map_err(value_error)?,
                None => batch,
            };
            extend_rows(&mut rows, &batch_to_value(&batch).map_err(value_error)?)?;
        }
        Ok(Self::from_inner(Scalar::from_sequence(rows)))
    }

    /// Convert back to Python's native scalar and collection types.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        as_py(py, &self.inner)
    }

    /// Infer the exact native Field for this scalar value.
    fn into_field(&self) -> PyResult<PyField> {
        self.inner
            .inferred_scalar_field()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Infer the exact item Field for this non-empty outer Sequence.
    fn into_array_field(&self) -> PyResult<PyField> {
        self.inner
            .inferred_array_field()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Infer a non-null Struct root from named Record rows.
    fn into_struct_field(&self) -> PyResult<PyField> {
        self.inner
            .inferred_struct_field()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Materialize this scalar as one exact `PyArrow` scalar.
    #[pyo3(signature = (field=None))]
    fn into_arrow_scalar<'py>(
        &self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let field = field.map_or_else(
            || self.inner.inferred_scalar_field().map_err(value_error),
            core_field_from_value,
        )?;
        let array = scalar_array(&field, &self.inner).map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, Some(&field))?.get_item(0)
    }

    /// Materialize an outer Sequence as one exact `PyArrow` array.
    #[pyo3(signature = (field=None))]
    fn into_arrow_array<'py>(
        &self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let field = field.map_or_else(
            || self.inner.inferred_array_field().map_err(value_error),
            core_field_from_value,
        )?;
        let array = value_into_arrow_array(&field, &self.inner)?;
        arrow_array_to_pyarrow(py, &array, Some(&field))
    }

    /// Materialize an outer Sequence of rows as one `PyArrow` `RecordBatch`.
    #[pyo3(signature = (field=None))]
    fn into_arrow_record_batch<'py>(
        &self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let root = field.map_or_else(
            || self.inner.inferred_struct_field().map_err(value_error),
            |field| core_root_field_from_value(field, "row"),
        )?;
        value_into_arrow_batch(&root, &self.inner)?.into_pyarrow(py)
    }

    /// Materialize an outer Sequence of rows as one `PyArrow` Table.
    #[pyo3(signature = (field=None))]
    fn into_arrow_table<'py>(
        &self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = self.into_arrow_record_batch(py, field)?;
        py.import("pyarrow")?
            .getattr("Table")?
            .call_method1("from_batches", (PyList::new(py, [batch])?,))
    }

    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    /// The enum vocabulary name, or `None`.
    #[getter]
    fn enum_kind(&self) -> Option<&'static str> {
        self.inner.as_enum().map(|value| value.kind())
    }

    /// The canonical enum member spelling, or `None`.
    #[getter]
    fn enum_value(&self) -> Option<&'static str> {
        self.inner.as_enum().map(|value| value.as_str())
    }

    /// The compact zero-based enum member index, or `None`.
    #[getter]
    fn enum_ordinal(&self) -> Option<u8> {
        self.inner.as_enum().map(|value| value.ordinal())
    }

    /// The count carried by a temporal value, or `None`.
    #[getter]
    fn count(&self) -> Option<i64> {
        self.inner
            .as_date32()
            .map(|(count, _, _)| i64::from(count))
            .or_else(|| self.inner.as_date64().map(|(count, _, _)| count))
            .or_else(|| self.inner.as_time32().map(|(count, _, _)| i64::from(count)))
            .or_else(|| self.inner.as_time64().map(|(count, _, _)| count))
            .or_else(|| self.inner.as_datetime64().map(|(count, _, _)| count))
            .or_else(|| {
                self.inner
                    .as_duration32()
                    .map(|(count, _, _)| i64::from(count))
            })
            .or_else(|| self.inner.as_duration64().map(|(count, _, _)| count))
    }

    /// The unit carried by a temporal value, or `None`.
    #[getter]
    fn unit(&self) -> Option<&'static str> {
        self.inner
            .as_date32()
            .map(|(_, unit, _)| unit)
            .or_else(|| self.inner.as_date64().map(|(_, unit, _)| unit))
            .or_else(|| self.inner.as_time32().map(|(_, unit, _)| unit))
            .or_else(|| self.inner.as_time64().map(|(_, unit, _)| unit))
            .or_else(|| self.inner.as_datetime64().map(|(_, unit, _)| unit))
            .or_else(|| self.inner.as_duration32().map(|(_, unit, _)| unit))
            .or_else(|| self.inner.as_duration64().map(|(_, unit, _)| unit))
            .map(TimeUnit::as_str)
    }

    /// The non-null timezone marker carried by a temporal value, or `None`.
    #[getter]
    fn zone(&self) -> Option<&str> {
        self.inner.temporal_timezone().map(Timezone::as_str)
    }

    /// The exact decimal coefficient as a Python integer, or `None`.
    #[getter]
    fn unscaled(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let coefficient = self
            .inner
            .as_d128()
            .map(|(unscaled, _)| unscaled.to_string())
            .or_else(|| {
                self.inner
                    .as_d256()
                    .map(|(unscaled, _)| unscaled.to_string())
            });
        coefficient
            .map(|coefficient| {
                py.import("builtins")?
                    .getattr("int")?
                    .call1((coefficient,))
                    .map(Bound::unbind)
            })
            .transpose()
    }

    /// The scale carried by an exact decimal, or `None`.
    #[getter]
    fn scale(&self) -> Option<i32> {
        self.inner
            .as_d128()
            .map(|(_, scale)| i32::from(scale))
            .or_else(|| self.inner.as_d256().map(|(_, scale)| i32::from(scale)))
    }

    #[getter]
    fn data_type(&self) -> PyResult<PyDataType> {
        self.inner
            .data_type()
            .map(PyDataType::from_inner)
            .map_err(value_error)
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn as_bytes<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner.as_bytes().map(|value| PyBytes::new(py, value))
    }

    fn as_utf8(&self) -> Option<&str> {
        self.inner.as_utf8()
    }

    fn as_json_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.inner
            .as_json_bytes()
            .map(|value| PyBytes::new(py, &value))
            .map_err(value_error)
    }

    fn as_json_utf8(&self) -> PyResult<String> {
        self.inner.as_json_utf8().map_err(value_error)
    }

    /// Add an inferred Python/native value through the core's checked rules.
    fn add(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        binary_arithmetic(&self.inner, other, Scalar::checked_add)
    }

    /// Subtract an inferred Python/native value through the core's checked rules.
    fn subtract(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        binary_arithmetic(&self.inner, other, Scalar::checked_sub)
    }

    /// Multiply by an inferred Python/native value through the core's checked rules.
    fn multiply(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        binary_arithmetic(&self.inner, other, Scalar::checked_mul)
    }

    /// Divide by an inferred Python/native value through the core's checked rules.
    fn divide(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        binary_arithmetic(&self.inner, other, Scalar::checked_div)
    }

    /// Return the checked remainder for an inferred Python/native divisor.
    fn remainder(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        binary_arithmetic(&self.inner, other, Scalar::checked_rem)
    }

    /// Return the checked numeric negation.
    fn negate(&self) -> PyResult<Self> {
        self.inner
            .checked_neg()
            .map(Self::from_inner)
            .map_err(|error| arithmetic_error(&error))
    }

    /// Return the checked absolute numeric value.
    fn absolute(&self) -> PyResult<Self> {
        self.inner
            .checked_abs()
            .map(Self::from_inner)
            .map_err(|error| arithmetic_error(&error))
    }

    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.add(other)
    }

    fn __radd__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        reflected_arithmetic(other, &self.inner, Scalar::checked_add)
    }

    fn __sub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.subtract(other)
    }

    fn __rsub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        reflected_arithmetic(other, &self.inner, Scalar::checked_sub)
    }

    fn __mul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.multiply(other)
    }

    fn __rmul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        reflected_arithmetic(other, &self.inner, Scalar::checked_mul)
    }

    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.divide(other)
    }

    fn __rtruediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        reflected_arithmetic(other, &self.inner, Scalar::checked_div)
    }

    fn __mod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.remainder(other)
    }

    fn __rmod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        reflected_arithmetic(other, &self.inner, Scalar::checked_rem)
    }

    fn __neg__(&self) -> PyResult<Self> {
        self.negate()
    }

    fn __abs__(&self) -> PyResult<Self> {
        self.absolute()
    }

    /// Return the number of direct children or entries.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Return whether this is an empty sequence, mapping, or record.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over sequence children, mapping keys, or record values.
    fn __iter__(&self) -> PyScalarIterator {
        PyScalarIterator::new(self.inner.iter().cloned())
    }

    /// Return a sequence child, accepting Python's negative indexes.
    fn at(&self, index: isize) -> Option<Self> {
        self.child_at(index).cloned().map(Self::from_inner)
    }

    /// Look up one mapping key or record field without lowering the child.
    fn get(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        self.child_for_key(key)
            .map(|child| child.cloned().map(Self::from_inner))
    }

    /// Walk a dotted mapping/record/sequence path.
    fn path(&self, path: &str) -> Option<Self> {
        self.inner.path(path).cloned().map(Self::from_inner)
    }

    /// Return whether a mapping key, record name, or sequence value exists.
    fn has(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Some(values) = self.inner.as_sequence() {
            let key = from_py(key)?;
            return Ok(values.contains(&key));
        }
        Ok(self.child_for_key(key)?.is_some())
    }

    /// Persistently add or replace one mapping key or record field.
    fn set(&self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = from_py(value)?;
        let rebuilt = if self.inner.as_record().is_some() {
            let key = key
                .cast::<PyString>()
                .map_err(|_| PyTypeError::new_err("record keys must be str"))?;
            self.inner.with_field(key.to_str()?, value)
        } else {
            self.inner.with_key(from_py(key)?, value)
        };
        rebuilt.map(Self::from_inner).map_err(value_error)
    }

    /// Persistently remove one string mapping key or record field.
    fn remove(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        let key = key
            .cast::<PyString>()
            .map_err(|_| PyTypeError::new_err("remove() key must be str"))?;
        let rebuilt = if self.inner.as_record().is_some() {
            self.inner.without_field(key.to_str()?)
        } else {
            self.inner.without_key(key.to_str()?)
        };
        rebuilt.map(Self::from_inner).map_err(value_error)
    }

    /// Iterate over every mapping/record key as an exact value.
    fn keys(&self) -> PyScalarIterator {
        let keys = if let Some(entries) = self.inner.as_mapping() {
            entries.iter().map(|(key, _)| key.clone()).collect()
        } else if let Some(entries) = self.inner.as_record() {
            entries.keys().cloned().map(Scalar::from).collect()
        } else {
            Vec::new()
        };
        PyScalarIterator::new(keys)
    }

    /// Iterate over every mapping/record child as an exact value.
    fn values(&self) -> PyScalarIterator {
        let values = if let Some(entries) = self.inner.as_mapping() {
            entries.iter().map(|(_, value)| value.clone()).collect()
        } else if let Some(entries) = self.inner.as_record() {
            entries.values().cloned().collect()
        } else {
            Vec::new()
        };
        PyScalarIterator::new(values)
    }

    /// Iterate over mapping/record entries without natural-type lowering.
    fn items(&self) -> PyScalarEntryIterator {
        let entries = if let Some(entries) = self.inner.as_mapping() {
            entries.to_vec()
        } else if let Some(entries) = self.inner.as_record() {
            entries
                .iter()
                .map(|(key, value)| (Scalar::from(key.clone()), value.clone()))
                .collect()
        } else {
            Vec::new()
        };
        PyScalarEntryIterator::new(entries)
    }

    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        if self.inner.as_sequence().is_some() {
            if key.is_instance_of::<PyBool>() || !key.is_instance_of::<PyInt>() {
                return Err(PyTypeError::new_err("sequence indexes must be int"));
            }
            let index = key.extract::<isize>()?;
            return self
                .child_at(index)
                .cloned()
                .map(Self::from_inner)
                .ok_or_else(|| PyIndexError::new_err(index));
        }
        self.child_for_key(key)?
            .cloned()
            .map(Self::from_inner)
            .ok_or_else(|| PyKeyError::new_err(key.clone().unbind()))
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.has(key)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let state = scalar_pickle_state(py, &self.inner)?;
        Ok(format!(
            "Scalar._from_pickle({})",
            state.bind(py).repr()?.to_str()?
        ))
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

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (scalar_pickle_state(py, &self.inner)?,),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Owning lazy iterator over exact native children.
#[pyclass(name = "ScalarIterator", module = "yggdryl._native")]
pub(crate) struct PyScalarIterator {
    inner: std::vec::IntoIter<Scalar>,
}

impl PyScalarIterator {
    fn new(values: impl IntoIterator<Item = Scalar>) -> Self {
        Self {
            inner: values.into_iter().collect::<Vec<_>>().into_iter(),
        }
    }
}

#[pymethods]
impl PyScalarIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyScalar> {
        self.inner.next().map(PyScalar::from_inner)
    }

    fn __length_hint__(&self) -> usize {
        self.inner.len()
    }
}

/// Owning lazy iterator over exact native mapping/record entries.
#[pyclass(name = "ValueEntryIterator", module = "yggdryl._native")]
pub(crate) struct PyScalarEntryIterator {
    inner: std::vec::IntoIter<(Scalar, Scalar)>,
}

impl PyScalarEntryIterator {
    fn new(values: impl IntoIterator<Item = (Scalar, Scalar)>) -> Self {
        Self {
            inner: values.into_iter().collect::<Vec<_>>().into_iter(),
        }
    }
}

#[pymethods]
impl PyScalarEntryIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<(PyScalar, PyScalar)> {
        self.inner
            .next()
            .map(|(key, value)| (PyScalar::from_inner(key), PyScalar::from_inner(value)))
    }

    fn __length_hint__(&self) -> usize {
        self.inner.len()
    }
}

/// Convert one Python object into a core value.
///
/// # Errors
///
/// Returns an error for a cyclic graph, a graph deeper than the codec limit, a
/// mapping whose distinct Python keys collide as values, or an object that has
/// no value shape at all.
pub(crate) fn from_py(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    Encoder::default().convert(value, 0)
}

/// Convert one core value into the Python object that names it.
///
/// # Errors
///
/// Returns an error when a temporal carries a resolution finer than Python's
/// microsecond, when a date or time falls outside the range `datetime`
/// represents, or when distinct encoded mapping keys collide under Python
/// equality.
pub(crate) fn as_py(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    match value {
        Scalar::Null => Ok(py.None()),
        Scalar::Bool(value) => Ok(value.into_pyobject(py)?.to_owned().into_any().unbind()),
        Scalar::I8(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::I16(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::I32(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::I64(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::U8(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::U16(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::U32(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::U64(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::I128(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::U128(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Scalar::F16(value) => Ok(value.as_f64().into_pyobject(py)?.into_any().unbind()),
        Scalar::F32(value) => Ok(value.as_f64().into_pyobject(py)?.into_any().unbind()),
        Scalar::F64(value) => Ok(value.as_f64().into_pyobject(py)?.into_any().unbind()),
        Scalar::D128(unscaled, scale) => decimal_as_py(py, &unscaled.to_string(), *scale),
        Scalar::D256(unscaled, scale) => decimal_as_py(py, &unscaled.to_string(), *scale),
        Scalar::String(value) => Ok(PyString::new(py, value.as_str()).into_any().unbind()),
        Scalar::Enum(value) => Ok(PyString::new(py, value.as_str()).into_any().unbind()),
        // A geometry has no Python binding surface yet, so its WKB crosses as
        // its plain shape: bytes.
        Scalar::Bytes(value) | Scalar::Geospatial(value) => {
            Ok(PyBytes::new(py, value).into_any().unbind())
        }
        Scalar::Date32(..) | Scalar::Date64(..) => date_as_py(py, value),
        Scalar::Time32(..) | Scalar::Time64(..) => time_as_py(py, value),
        Scalar::DateTime64(..) => datetime_as_py(py, value),
        Scalar::Duration32(..) | Scalar::Duration64(..) => duration_as_py(py, value),
        Scalar::Sequence(items) => {
            let items = items
                .iter()
                .map(|item| as_py(py, item))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyList::new(py, items)?.into_any().unbind())
        }
        Scalar::Mapping(entries) => mapping_to_python(py, entries),
        Scalar::Record(entries) => {
            let output = PyDict::new(py);
            for (name, value) in entries.iter() {
                output.set_item(name.as_str(), as_py(py, value)?)?;
            }
            Ok(output.into_any().unbind())
        }
    }
}

/// Convert a typed value while restoring named struct objects at the boundary.
pub(crate) fn as_py_with_field(
    py: Python<'_>,
    value: &Scalar,
    field: &CoreField,
) -> PyResult<Py<PyAny>> {
    if value.is_null() {
        return as_py(py, value);
    }
    match field.data_type() {
        CoreDataType::Struct(fields) => {
            let output = PyDict::new(py);
            match value {
                Scalar::Sequence(values) if values.len() == fields.len() => {
                    for (child, value) in fields.iter().zip(values.iter()) {
                        output.set_item(child.name(), as_py_with_field(py, value, child)?)?;
                    }
                }
                Scalar::Record(values) => {
                    for child in fields {
                        let value = values.get(child.name()).ok_or_else(|| {
                            PyValueError::new_err(format!(
                                "typed record is missing field {:?}",
                                child.name()
                            ))
                        })?;
                        output.set_item(child.name(), as_py_with_field(py, value, child)?)?;
                    }
                }
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "expected {} typed struct values, got {}",
                        fields.len(),
                        value.kind()
                    )));
                }
            }
            Ok(output.into_any().unbind())
        }
        CoreDataType::List(child)
        | CoreDataType::ListView(child)
        | CoreDataType::FixedSizeList(child, _)
        | CoreDataType::LargeList(child)
        | CoreDataType::LargeListView(child) => {
            let values = value.as_sequence().ok_or_else(|| {
                PyValueError::new_err(format!(
                    "expected a typed list sequence, got {}",
                    value.kind()
                ))
            })?;
            let values = values
                .iter()
                .map(|value| as_py_with_field(py, value, child))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyList::new(py, values)?.into_any().unbind())
        }
        CoreDataType::Map(map) => {
            let [_key_field, value_field] = map.entries().fields() else {
                return Err(PyValueError::new_err(
                    "typed map entries need key and value fields",
                ));
            };
            let entries = value.as_mapping().ok_or_else(|| {
                PyValueError::new_err(format!("expected a typed mapping, got {}", value.kind()))
            })?;
            let output = PyDict::new(py);
            for (key, value) in entries {
                let key = as_py_key(py, key)?;
                if output.contains(&key)? {
                    return Err(PyValueError::new_err(
                        "distinct typed mapping keys collide under Python equality",
                    ));
                }
                // The key was already canonicalized under its field; Python's
                // hashable projection preserves that value. Values retain the
                // nested field names a dataclass target needs.
                output.set_item(key, as_py_with_field(py, value, value_field)?)?;
            }
            Ok(output.into_any().unbind())
        }
        CoreDataType::Union(fields, _) => {
            let Some([type_id, payload]) = value.as_sequence() else {
                return Err(PyValueError::new_err(
                    "typed union value must contain its type id and payload",
                ));
            };
            let type_id = type_id
                .as_i128()
                .and_then(|value| i8::try_from(value).ok())
                .ok_or_else(|| PyValueError::new_err("typed union id must fit i8"))?;
            let branch = fields
                .iter()
                .find_map(|(candidate, branch)| (candidate == type_id).then_some(branch))
                .ok_or_else(|| PyValueError::new_err("typed union id is not declared"))?;
            as_py_with_field(py, payload, branch)
        }
        CoreDataType::Dictionary(dictionary) => {
            let value_field = CoreField::new(
                field.name(),
                dictionary.value().clone(),
                field.is_nullable(),
            );
            as_py_with_field(py, value, &value_field)
        }
        CoreDataType::RunEndEncoded(encoded) => as_py_with_field(py, value, encoded.values()),
        _ => as_py(py, value),
    }
}

/// Build the Python dictionary one core mapping names.
fn mapping_to_python(py: Python<'_>, entries: &[(Scalar, Scalar)]) -> PyResult<Py<PyAny>> {
    let output = PyDict::new(py);
    for (key, value) in entries {
        let key = as_py_key(py, key)?;
        if output.contains(&key)? {
            return Err(PyValueError::new_err(
                "distinct encoded mapping keys collide under Python equality",
            ));
        }
        output.set_item(key, as_py(py, value)?)?;
    }
    Ok(output.into_any().unbind())
}

/// Convert one core value into a form Python can hash as a mapping key.
///
/// A sequence becomes a tuple and a mapping becomes a tuple of its entries,
/// because those are the hashable spellings of the same shapes. This is the one
/// place the two directions deliberately disagree: the typed value conversion
/// reads a tuple of pairs back as a mapping.
fn as_py_key(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    match value {
        Scalar::Sequence(items) => {
            let items = items
                .iter()
                .map(|item| as_py_key(py, item))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, items)?.into_any().unbind())
        }
        Scalar::Mapping(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| Ok((as_py_key(py, key)?, as_py_key(py, value)?)))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, entries)?.into_any().unbind())
        }
        Scalar::Record(entries) => {
            let entries = entries
                .iter()
                .map(|(name, value)| {
                    Ok((
                        PyString::new(py, name.as_str()).into_any().unbind(),
                        as_py_key(py, value)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, entries)?.into_any().unbind())
        }
        _ => as_py(py, value),
    }
}

/// The Python-object-to-value walk, carrying the identities already on the stack.
#[derive(Default)]
struct Encoder {
    active: HashSet<usize>,
}

impl Encoder {
    /// Convert one object, refusing to recurse past the codec's depth limit.
    fn convert(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
        if depth >= MAX_PYTHON_DEPTH {
            return Err(PyValueError::new_err(format!(
                "Python value exceeds the {MAX_PYTHON_DEPTH}-level codec limit"
            )));
        }
        if let Some(value) = self.convert_exact_builtin(value, depth)? {
            return Ok(value);
        }
        if let Some(value) = native_wrapper_to_value(value) {
            return Ok(value);
        }
        // A path is recognized by its protocol rather than by its class,
        // because every path-like object answers `__fspath__` and none of them
        // share a base class.
        if value.hasattr("__fspath__")? {
            return path_to_value(value);
        }
        let identity = type_identity(value)?;
        match identity.as_str() {
            "decimal.Decimal" => decimal_to_value(value),
            "uuid.UUID" => Ok(Scalar::String(value.str()?.to_str()?.into())),
            "datetime.datetime" => datetime_to_value(value),
            "datetime.time" => time_to_value(value),
            "datetime.date" => date_to_value(value),
            "datetime.timedelta" => duration_to_value(value),
            "builtins.complex" => complex_to_value(value),
            "builtins.range" | "builtins.slice" => self.convert_triple(value, depth),
            "collections.deque" => self.convert_iterator(value, depth),
            "collections.OrderedDict" | "collections.Counter" | "collections.defaultdict" => {
                self.convert_dict(value, depth)
            }
            _ => self.convert_other(value, &identity, depth),
        }
    }

    /// Convert the builtin types whose exact class needs no further inspection.
    ///
    /// Exact-class matching comes first because it is the hot path and because
    /// a subclass can mean something its base does not: an `enum.IntEnum` is an
    /// `int` whose value is the number it names, not the member itself.
    fn convert_exact_builtin(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Scalar>> {
        if value.is_none() {
            return Ok(Some(Scalar::Null));
        }
        if is_exact_type::<PyBool>(value) {
            return value.extract::<bool>().map(Scalar::Bool).map(Some);
        }
        if is_exact_type::<PyInt>(value) {
            return integer_to_value(value).map(Some);
        }
        if is_exact_type::<PyFloat>(value) {
            return value
                .extract::<f64>()
                .map(Float64::from_f64)
                .map(Scalar::F64)
                .map(Some);
        }
        if is_exact_type::<PyString>(value) {
            return Ok(Some(Scalar::String(
                value.cast::<PyString>()?.to_str()?.into(),
            )));
        }
        if is_exact_type::<PyBytes>(value) {
            return Ok(Some(Scalar::from(value.cast::<PyBytes>()?.as_bytes())));
        }
        if is_exact_type::<PyByteArray>(value) {
            return Ok(Some(Scalar::from(value.cast::<PyByteArray>()?.to_vec())));
        }
        if is_exact_type::<PyMemoryView>(value) {
            let bytes = value.call_method0("tobytes")?.cast_into::<PyBytes>()?;
            return Ok(Some(Scalar::from(bytes.as_bytes())));
        }
        if is_exact_type::<PyList>(value) {
            let items = value.cast::<PyList>()?;
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if is_exact_type::<PyTuple>(value) {
            let items = value.cast::<PyTuple>()?;
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if is_exact_type::<PySet>(value) {
            let items = value.cast::<PySet>()?;
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if is_exact_type::<PyFrozenSet>(value) {
            let items = value.cast::<PyFrozenSet>()?;
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if is_exact_type::<PyDict>(value) {
            return self.convert_dict(value, depth).map(Some);
        }
        Ok(None)
    }

    /// Convert anything left: a subclass, a dataclass, or a plain object.
    fn convert_other(
        &mut self,
        value: &Bound<'_, PyAny>,
        identity: &str,
        depth: usize,
    ) -> PyResult<Scalar> {
        if let Some(value) = self.convert_scalar_subclass(value, depth)? {
            return Ok(value);
        }
        if let Some(value) = self.convert_collection_subclass(value, depth)? {
            return Ok(value);
        }
        let py = value.py();
        let dataclasses = py.import("dataclasses")?;
        let is_dataclass = dataclasses
            .getattr("is_dataclass")?
            .call1((value,))?
            .is_truthy()?;
        if is_dataclass && value.cast::<PyType>().is_err() {
            return self.convert_dataclass(value, depth, &dataclasses);
        }
        if let Some(record) = self.convert_plain_object(value, depth)? {
            return Ok(record);
        }
        Err(PyTypeError::new_err(format!(
            "unsupported value type {identity}; use a dataclass, mapping, or supported scalar"
        )))
    }

    /// Convert a subclass of a scalar type as the scalar it is.
    ///
    /// Every check here is an `isinstance`, so a subclass keeps its base's
    /// shape instead of decaying into whatever its instance dictionary holds.
    fn convert_scalar_subclass(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Scalar>> {
        let py = value.py();
        // An enum member is the value it names: the member's class is the
        // identity being dropped, and its value is what a schema declares.
        if value.is_instance(&py.import("enum")?.getattr("Enum")?)? {
            return self.convert(&value.getattr("value")?, depth + 1).map(Some);
        }
        if value.cast::<PyComplex>().is_ok() {
            return complex_to_value(value).map(Some);
        }
        if let Ok(bytes) = value.cast::<PyByteArray>() {
            return Ok(Some(Scalar::from(bytes.to_vec())));
        }
        if value.is_instance(&py.import("decimal")?.getattr("Decimal")?)? {
            return decimal_to_value(value).map(Some);
        }
        let datetime = py.import("datetime")?;
        // A `datetime` is a `date`, so the narrower class is asked first.
        if value.is_instance(&datetime.getattr("datetime")?)? {
            return datetime_to_value(value).map(Some);
        }
        if value.is_instance(&datetime.getattr("date")?)? {
            return date_to_value(value).map(Some);
        }
        if value.is_instance(&datetime.getattr("time")?)? {
            return time_to_value(value).map(Some);
        }
        if value.is_instance(&datetime.getattr("timedelta")?)? {
            return duration_to_value(value).map(Some);
        }
        if value.is_instance_of::<PyInt>() {
            return integer_to_value(value).map(Some);
        }
        if value.is_instance_of::<PyFloat>() {
            return Ok(Some(Scalar::F64(Float64::from_f64(value.extract()?))));
        }
        if let Ok(text) = value.cast::<PyString>() {
            return Ok(Some(Scalar::String(text.to_str()?.into())));
        }
        if let Ok(bytes) = value.cast::<PyBytes>() {
            return Ok(Some(Scalar::from(bytes.as_bytes())));
        }
        Ok(None)
    }

    /// Convert a subclass of a collection type as the collection it is.
    fn convert_collection_subclass(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Scalar>> {
        let py = value.py();
        if value.is_instance(&py.import("collections")?.getattr("deque")?)? {
            return self.convert_iterator(value, depth).map(Some);
        }
        if let Ok(items) = value.cast::<PyTuple>() {
            // A named tuple names its members, so a record is the shape that
            // keeps them; an ordinary tuple has only positions.
            if value.hasattr("_fields")? {
                return self.convert_named_tuple(value, depth).map(Some);
            }
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if let Ok(items) = value.cast::<PyList>() {
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if let Ok(items) = value.cast::<PySet>() {
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if let Ok(items) = value.cast::<PyFrozenSet>() {
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if value.cast::<PyDict>().is_ok() {
            return self.convert_dict(value, depth).map(Some);
        }
        if value.is_instance(&py.import("collections.abc")?.getattr("Mapping")?)? {
            return self.convert_mapping(value, depth).map(Some);
        }
        Ok(None)
    }

    /// Convert a borrowed run of children into a sequence.
    ///
    /// A set is sorted on the way out, because a document written twice from
    /// one value has to be the same document both times.
    fn convert_sequence<'py, I>(
        &mut self,
        owner: &Bound<'py, PyAny>,
        items: I,
        depth: usize,
        sort: bool,
    ) -> PyResult<Scalar>
    where
        I: Iterator<Item = Bound<'py, PyAny>>,
    {
        self.with_cycle_check(owner, |encoder| {
            let mut values = items
                .map(|item| encoder.convert(&item, depth + 1))
                .collect::<PyResult<Vec<_>>>()?;
            if sort {
                values.sort_unstable();
            }
            Ok(Scalar::from_sequence(values))
        })
    }

    /// Convert anything that only knows how to iterate into a sequence.
    fn convert_iterator(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
        self.with_cycle_check(value, |encoder| {
            let items = value
                .try_iter()?
                .map(|item| encoder.convert(&item?, depth + 1))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(Scalar::from_sequence(items))
        })
    }

    /// Convert a `range` or a `slice` into its start, stop, and step.
    fn convert_triple(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
        let values = ["start", "stop", "step"]
            .into_iter()
            .map(|name| self.convert(&value.getattr(name)?, depth + 1))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Scalar::from_sequence(values))
    }

    /// Convert any dictionary, including a subclass, into a mapping.
    fn convert_dict(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
        let entries = value.cast::<PyDict>()?;
        self.convert_entries(value, entries, depth)
    }

    /// Convert any Mapping implementation without materializing a dict first.
    fn convert_mapping(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
        self.with_cycle_check(value, |encoder| {
            let entries = value
                .call_method0("items")?
                .try_iter()?
                .map(|entry| {
                    let entry = entry?.cast_into::<PyTuple>()?;
                    if entry.len() != 2 {
                        return Err(PyTypeError::new_err(
                            "a mapping items() iterator must yield key/value pairs",
                        ));
                    }
                    Ok((
                        encoder.convert(&entry.get_item(0)?, depth + 1)?,
                        encoder.convert(&entry.get_item(1)?, depth + 1)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Scalar::from_mapping(entries).map_err(value_error)
        })
    }

    /// Convert one dictionary's entries into a mapping, its keys included.
    fn convert_entries(
        &mut self,
        owner: &Bound<'_, PyAny>,
        entries: &Bound<'_, PyDict>,
        depth: usize,
    ) -> PyResult<Scalar> {
        self.with_cycle_check(owner, |encoder| {
            let entries = entries
                .iter()
                .map(|(key, item)| {
                    Ok((
                        encoder.convert(&key, depth + 1)?,
                        encoder.convert(&item, depth + 1)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Scalar::from_mapping(entries).map_err(value_error)
        })
    }

    /// Convert a plain object's dictionary and slots into one record.
    fn convert_plain_object(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Scalar>> {
        let attributes = value
            .getattr("__dict__")
            .ok()
            .and_then(|attributes| attributes.cast_into::<PyDict>().ok());
        // copyreg owns Python's inherited/name-mangled slot discovery for the
        // pickle protocol; reusing it avoids a second, subtly different walk.
        let names = value
            .py()
            .import("copyreg")?
            .getattr("_slotnames")?
            .call1((value.get_type(),))?;
        if attributes.is_none() && names.is_none() {
            return Ok(None);
        }
        self.with_cycle_check(value, |encoder| {
            let mut entries = Vec::new();
            let mut seen = HashSet::new();
            if let Some(attributes) = &attributes {
                for (name, item) in attributes.iter() {
                    let name = name.extract::<String>().map_err(|_| {
                        PyTypeError::new_err("record attribute names must be strings")
                    })?;
                    seen.insert(name.clone());
                    entries.push((name, encoder.convert(&item, depth + 1)?));
                }
            }
            if !names.is_none() {
                for name in names.try_iter()? {
                    let name = name?.extract::<String>()?;
                    if seen.contains(&name) || !value.hasattr(name.as_str())? {
                        continue;
                    }
                    entries.push((
                        name.clone(),
                        encoder.convert(&value.getattr(name.as_str())?, depth + 1)?,
                    ));
                }
            }
            Scalar::from_record(entries).map_err(value_error)
        })
        .map(Some)
    }

    /// Convert a named tuple into the record its field names describe.
    fn convert_named_tuple(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Scalar> {
        let names = value.getattr("_fields")?;
        self.with_cycle_check(value, |encoder| {
            let entries = names
                .try_iter()?
                .map(|name| {
                    let name = name?.extract::<String>()?;
                    let item = value.getattr(name.as_str())?;
                    Ok((name, encoder.convert(&item, depth + 1)?))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Scalar::from_record(entries).map_err(value_error)
        })
    }

    /// Convert a dataclass instance into the record of its declared fields.
    ///
    /// The cached tuple a field-decorated dataclass publishes is read first,
    /// because the encode path must not allocate a `dataclasses.fields` tuple
    /// per instance.
    fn convert_dataclass(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
        dataclasses: &Bound<'_, PyModule>,
    ) -> PyResult<Scalar> {
        self.with_cycle_check(value, |encoder| {
            let mut entries = Vec::new();
            let cached_fields = value
                .get_type()
                .getattr("__yggdryl_scalar_fields__")
                .ok()
                .and_then(|cached| cached.cast_into::<PyTuple>().ok())
                .and_then(|cached| cached.get_item(1).ok())
                .and_then(|fields| fields.cast_into::<PyTuple>().ok());
            if let Some(fields) = cached_fields {
                for field in fields.iter() {
                    push_dataclass_field(encoder, value, &field, depth, &mut entries)?;
                }
            } else {
                let marker = dataclasses.getattr("_FIELD")?;
                let fields = value
                    .get_type()
                    .getattr("__dataclass_fields__")?
                    .cast_into::<PyDict>()?;
                for (_, field) in fields.iter() {
                    if field.getattr("_field_type")?.is(&marker) {
                        push_dataclass_field(encoder, value, &field, depth, &mut entries)?;
                    }
                }
            }
            Scalar::from_record(entries).map_err(value_error)
        })
    }

    /// Run one conversion with this object marked as being on the stack.
    fn with_cycle_check<F>(&mut self, value: &Bound<'_, PyAny>, convert: F) -> PyResult<Scalar>
    where
        F: FnOnce(&mut Self) -> PyResult<Scalar>,
    {
        let identity = value.as_ptr() as usize;
        if !self.active.insert(identity) {
            return Err(PyValueError::new_err(
                "cyclic Python values cannot be serialized",
            ));
        }
        let converted = convert(self);
        self.active.remove(&identity);
        converted
    }
}

/// Append one dataclass field's name and converted value.
fn push_dataclass_field(
    encoder: &mut Encoder,
    value: &Bound<'_, PyAny>,
    field: &Bound<'_, PyAny>,
    depth: usize,
    entries: &mut Vec<(String, Scalar)>,
) -> PyResult<()> {
    let name = field.getattr("name")?.extract::<String>()?;
    entries.push((
        name.clone(),
        encoder.convert(&value.getattr(name.as_str())?, depth + 1)?,
    ));
    Ok(())
}

/// Convert a native Yggdryl wrapper into its canonical text, when it is one.
///
/// Each of these round-trips through its own `from_str`, so the text is the
/// whole value; what a document loses is only which wrapper class held it.
fn native_wrapper_to_value(value: &Bound<'_, PyAny>) -> Option<Scalar> {
    if let Ok(value) = value.extract::<PyRef<'_, PyScalar>>() {
        return Some(value.inner.clone());
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyDataType>>() {
        return Some(Scalar::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyField>>() {
        return Some(Scalar::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUri>>() {
        return Some(Scalar::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrl>>() {
        return Some(Scalar::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrn>>() {
        return Some(Scalar::String(value.inner.to_string().into()));
    }
    None
}

/// Convert any path-like object into the string its file system uses.
fn path_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    let py = value.py();
    let os = py.import("os")?;
    let path = os.getattr("fspath")?.call1((value,))?;
    // A bytes path is still a path, so decoding it with the file system's own
    // rules keeps it readable rather than turning it into a byte blob.
    let path = if path.cast::<PyBytes>().is_ok() {
        os.getattr("fsdecode")?.call1((path,))?
    } else {
        path
    };
    Ok(Scalar::String(path.cast::<PyString>()?.to_str()?.into()))
}

/// Convert a Python integer into the narrowest native integer that holds it.
fn integer_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    if let Ok(value) = value.extract::<i64>() {
        return Ok(Scalar::I64(value));
    }
    if let Ok(value) = value.extract::<u64>() {
        return Ok(Scalar::U64(value));
    }
    if let Ok(value) = value.extract::<i128>() {
        return Ok(Scalar::I128(value));
    }
    if let Ok(value) = value.extract::<u128>() {
        return Ok(Scalar::U128(value));
    }
    // Python's integers are unbounded and no native integer is. The decimal
    // text is the only exact shape left, so the magnitude survives and the type
    // does not: an integer wider than 128 bits reads back as a string.
    Ok(Scalar::String(value.str()?.to_str()?.into()))
}

/// Convert a complex number into the pair of floats it is.
fn complex_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    let complex = value.cast::<PyComplex>()?;
    Ok(Scalar::from_sequence([
        Scalar::F64(Float64::from_f64(complex.real())),
        Scalar::F64(Float64::from_f64(complex.imag())),
    ]))
}

/// Convert a `decimal.Decimal` into the exact coefficient and scale it names.
fn decimal_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    let parts = value.call_method0("as_tuple")?;
    let Ok(exponent) = parts.getattr("exponent")?.extract::<i32>() else {
        // A non-finite decimal spells its exponent `n`, `N`, or `F`. No exact
        // decimal names an infinity or a NaN, so the float that does is the
        // honest shape and the document says so.
        return Ok(Scalar::F64(Float64::from_f64(value.extract::<f64>()?)));
    };
    let scale = exponent
        .checked_neg()
        .and_then(|scale| i8::try_from(scale).ok())
        .ok_or_else(|| {
            PyOverflowError::new_err(format!(
                "decimal exponent {exponent} has no scale in -128..=127"
            ))
        })?;
    let negative = parts.getattr("sign")?.extract::<i32>()? == 1;
    let mut digits = String::new();
    if negative {
        digits.push('-');
    }
    for digit in parts.getattr("digits")?.try_iter()? {
        let digit = digit?.extract::<u8>()?;
        digits.push(char::from(b'0' + digit));
    }
    let unscaled = digits.parse::<I256>().map_err(|_| {
        PyOverflowError::new_err("decimal coefficient exceeds the 256 bits D256 holds")
    })?;
    if let Some(unscaled) = unscaled.as_i128() {
        return Ok(Scalar::d128(unscaled, scale));
    }
    Ok(Scalar::d256(unscaled, scale))
}

/// Build the `decimal.Decimal` one coefficient and scale name.
fn decimal_as_py(py: Python<'_>, unscaled: &str, scale: i8) -> PyResult<Py<PyAny>> {
    // `<coefficient>E<-scale>` is exact and keeps the written precision, so a
    // value at scale 2 comes back as `10.50`. `Decimal(unscaled).scaleb(-scale)`
    // would instead round the coefficient at the active context's precision.
    let text = format!("{unscaled}E{}", -i32::from(scale));
    py.import("decimal")?
        .getattr("Decimal")?
        .call1((text,))
        .map(Bound::unbind)
}

/// Convert a `datetime.date` into its day count since the Unix epoch.
fn date_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    let ordinal = value.call_method0("toordinal")?.extract::<i64>()?;
    let days = i32::try_from(ordinal - EPOCH_ORDINAL)
        .map_err(|_| PyOverflowError::new_err("date is outside the days a date value counts"))?;
    Ok(Scalar::date32(days))
}

/// Build the `datetime.date` one epoch day count names.
fn date_as_py(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    let days = value.temporal_count_at(TimeUnit::Day).ok_or_else(|| {
        PyValueError::new_err(format!(
            "a {} does not name an exact whole-day date",
            value.kind()
        ))
    })?;
    py.import("datetime")?
        .getattr("date")?
        .getattr("fromordinal")?
        .call1((days + EPOCH_ORDINAL,))
        .map(Bound::unbind)
}

/// Convert a `datetime.time` into its microsecond count since midnight.
///
/// Time32/64 has no Arrow timezone parameter, so a zoned Python time is
/// refused by the core rather than losing its zone during type inference.
fn time_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    Scalar::time64(
        microseconds_of_day(value)?,
        TimeUnit::Microsecond,
        temporal_zone(value)?,
    )
    .map_err(value_error)
}

/// Read the zone attached to a Python time-like value, or the explicit naive
/// zone when it has none.
fn temporal_zone(value: &Bound<'_, PyAny>) -> PyResult<Timezone> {
    let zone = value.getattr("tzinfo")?;
    if zone.is_none() {
        Ok(Timezone::NAIVE)
    } else {
        core_timezone_from_value(&zone)
    }
}

/// Build the `datetime.time` one microsecond count since midnight names.
fn time_as_py(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    let count = exact_microseconds(value)?;
    if !(0..MICROSECONDS_PER_DAY).contains(&count) {
        return Err(PyValueError::new_err(format!(
            "a time of day must be within one day of midnight, got {count} microseconds"
        )));
    }
    let (hour, minute, second, microsecond) = split_day(count);
    let datetime = py.import("datetime")?;
    let zone = value.temporal_timezone().ok_or_else(|| {
        PyValueError::new_err(format!("expected a time value, got {}", value.kind()))
    })?;
    if zone.is_naive() {
        return datetime
            .getattr("time")?
            .call1((hour, minute, second, microsecond))
            .map(Bound::unbind);
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("tzinfo", zone_to_tzinfo(py, zone, 0)?)?;
    datetime
        .getattr("time")?
        .call((hour, minute, second, microsecond), Some(&kwargs))
        .map(Bound::unbind)
}

/// Convert a `datetime.timedelta` into its elapsed microsecond count.
fn duration_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    Scalar::duration64(timedelta_microseconds(value)?, TimeUnit::Microsecond).map_err(value_error)
}

/// Build the `datetime.timedelta` one elapsed microsecond count names.
fn duration_as_py(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("microseconds", exact_microseconds(value)?)?;
    py.import("datetime")?
        .getattr("timedelta")?
        .call((), Some(&kwargs))
        .map(Bound::unbind)
}

/// Convert a `datetime.datetime` into a UTC-relative count and its zone.
///
/// The count Arrow defines is always relative to UTC, so an aware value moves
/// by the offset in force at that instant - which is exactly the offset Python
/// computes, daylight saving and `fold` included. A naive value has no offset
/// to apply and carries no zone.
fn datetime_to_value(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    let days = value.call_method0("toordinal")?.extract::<i64>()? - EPOCH_ORDINAL;
    let time_of_day = microseconds_of_day(value)?;
    let local = days
        .checked_mul(MICROSECONDS_PER_DAY)
        .and_then(|days| days.checked_add(time_of_day))
        .ok_or_else(overflowing_timestamp)?;
    let offset = value.call_method0("utcoffset")?;
    if offset.is_none() {
        return Scalar::datetime64(local, TimeUnit::Microsecond, Timezone::NAIVE)
            .map_err(value_error);
    }
    let count = local
        .checked_sub(timedelta_microseconds(&offset)?)
        .ok_or_else(overflowing_timestamp)?;
    let zone = core_timezone_from_value(&value.getattr("tzinfo")?)?;
    Scalar::datetime64(count, TimeUnit::Microsecond, zone).map_err(value_error)
}

/// The error a datetime beyond a 64-bit microsecond count reports.
fn overflowing_timestamp() -> PyErr {
    PyOverflowError::new_err("datetime is outside the microseconds a timestamp counts")
}

/// Build the `datetime.datetime` one UTC-relative count and zone name.
fn datetime_as_py(py: Python<'_>, value: &Scalar) -> PyResult<Py<PyAny>> {
    let count = exact_microseconds(value)?;
    let (_, _, zone) = value.as_datetime64().ok_or_else(|| {
        PyValueError::new_err(format!("expected a datetime64, got {}", value.kind()))
    })?;
    let datetime = py.import("datetime")?;
    let date = datetime
        .getattr("date")?
        .getattr("fromordinal")?
        .call1((count.div_euclid(MICROSECONDS_PER_DAY) + EPOCH_ORDINAL,))?;
    let (hour, minute, second, microsecond) = split_day(count.rem_euclid(MICROSECONDS_PER_DAY));
    let arguments = (
        date.getattr("year")?.extract::<i64>()?,
        date.getattr("month")?.extract::<i64>()?,
        date.getattr("day")?.extract::<i64>()?,
        hour,
        minute,
        second,
        microsecond,
    );
    if zone.is_naive() {
        return datetime
            .getattr("datetime")?
            .call1(arguments)
            .map(Bound::unbind);
    }
    // The count is UTC, so the value is built in UTC and then moved into the
    // zone it was written in. Building it in that zone directly would need the
    // local reading, which is exactly what the offset was taken out of.
    let kwargs = PyDict::new(py);
    kwargs.set_item("tzinfo", datetime.getattr("timezone")?.getattr("utc")?)?;
    let instant = datetime
        .getattr("datetime")?
        .call(arguments, Some(&kwargs))?;
    let tzinfo = zone_to_tzinfo(py, zone, count.div_euclid(1_000_000))?;
    instant
        .call_method1("astimezone", (tzinfo,))
        .map(Bound::unbind)
}

/// Return the Python `tzinfo` that answers for one core zone.
fn zone_to_tzinfo<'py>(
    py: Python<'py>,
    zone: &Timezone,
    epoch_seconds: i64,
) -> PyResult<Bound<'py, PyAny>> {
    let datetime = py.import("datetime")?;
    if zone.is_utc() {
        return datetime.getattr("timezone")?.getattr("utc");
    }
    // A place names rules rather than an offset, and `zoneinfo` is where Python
    // keeps them, so the value comes back carrying the same live zone it went
    // in with rather than a frozen offset.
    if !zone.is_fixed()
        && let Ok(tzinfo) = py
            .import("zoneinfo")?
            .getattr("ZoneInfo")?
            .call1((zone.as_str(),))
    {
        return Ok(tzinfo);
    }
    // A fixed offset, or a platform with no time zone database: this build's
    // own rules answer for the instant, which is all a fixed offset can carry.
    let offset = zone.offset_at(epoch_seconds).ok_or_else(|| {
        PyValueError::new_err(format!(
            "no offset rules for time zone {:?} in this build or in Python's database",
            zone.as_str()
        ))
    })?;
    let delta = datetime.getattr("timedelta")?.call1((0, offset))?;
    datetime.getattr("timezone")?.call1((delta, zone.as_str()))
}

/// Split a microsecond count within one day into its clock fields.
const fn split_day(count: i64) -> (i64, i64, i64, i64) {
    let hour = count / 3_600_000_000;
    let rest = count % 3_600_000_000;
    let minute = rest / 60_000_000;
    let rest = rest % 60_000_000;
    (hour, minute, rest / 1_000_000, rest % 1_000_000)
}

/// Read the microseconds elapsed since midnight from any clock-shaped object.
fn microseconds_of_day(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    let hour = value.getattr("hour")?.extract::<i64>()?;
    let minute = value.getattr("minute")?.extract::<i64>()?;
    let second = value.getattr("second")?.extract::<i64>()?;
    let microsecond = value.getattr("microsecond")?.extract::<i64>()?;
    Ok(((hour * 60 + minute) * 60 + second) * 1_000_000 + microsecond)
}

/// Read a `datetime.timedelta` as one microsecond count.
fn timedelta_microseconds(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    let days = i128::from(value.getattr("days")?.extract::<i64>()?);
    let seconds = i128::from(value.getattr("seconds")?.extract::<i64>()?);
    let microseconds = i128::from(value.getattr("microseconds")?.extract::<i64>()?);
    // A Python timedelta spans about 2.7 million years and a 64-bit microsecond
    // count spans about 292 thousand, so the wide arithmetic is what makes the
    // overflow reportable instead of wrapping.
    let total = (days * 86_400 + seconds) * 1_000_000 + microseconds;
    i64::try_from(total).map_err(|_| {
        PyOverflowError::new_err("timedelta exceeds the microseconds a duration counts")
    })
}

/// Restate one temporal at Python's microsecond resolution, or refuse.
fn exact_microseconds(value: &Scalar) -> PyResult<i64> {
    value.temporal_count_at(TimeUnit::Microsecond).ok_or_else(|| {
        PyValueError::new_err(format!(
            "a {} at this resolution has no exact microsecond count, which is all datetime holds",
            value.kind()
        ))
    })
}

/// Return the dotted `module.qualname` of a value's class.
fn type_identity(value: &Bound<'_, PyAny>) -> PyResult<String> {
    let value_type = value.get_type();
    let module = value_type.getattr("__module__")?.extract::<String>()?;
    let qualname = value_type.getattr("__qualname__")?.extract::<String>()?;
    Ok(format!("{module}.{qualname}"))
}

/// Return whether a value's class is exactly `T` rather than a subclass of it.
fn is_exact_type<T: PyTypeInfo>(value: &Bound<'_, PyAny>) -> bool {
    value.get_type().is(value.py().get_type::<T>())
}
