//! The `Scalar` pyclass — a single atomic value carrying its full data type, with
//! lossless Arrow round-tripping. A thin wrapper over [`yggdryl_scalar`]'s `Scalar`; all
//! logic lives in the core, so the Python and Node bindings behave identically.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple,
};
use yggdryl_scalar::{from_bytes, i256, Interval, ScalarError, ScalarValue as CoreScalar};
use yggdryl_schema::DataType as CoreDataType;

use crate::datatype::DataType;
use crate::{scalar_err, schema_err};

/// A single, type-erased value that knows its own :class:`DataType` and round-trips
/// losslessly to and from Apache Arrow. Build one from a Python value
/// (``Scalar(42)`` / ``Scalar("hi")`` / ``Scalar(5, "int32")``), a canonical string
/// (:meth:`from_str`), bytes (:meth:`from_bytes`) or a component map
/// (:meth:`from_mapping`); read its native :attr:`value`, its :attr:`data_type`, and
/// serialise it through :meth:`to_str` / :meth:`to_bytes`.
#[pyclass(name = "Scalar", module = "yggdryl")]
#[derive(Clone)]
pub struct Scalar {
    pub(crate) inner: CoreScalar,
}

fn wrap(inner: CoreScalar) -> Scalar {
    Scalar { inner }
}

/// Resolves a `DataType` pyclass **or** a type string to a core [`CoreDataType`].
fn resolve_dtype(obj: &Bound<'_, PyAny>) -> PyResult<CoreDataType> {
    if let Ok(dt) = obj.extract::<DataType>() {
        return Ok(dt.inner);
    }
    let text: String = obj.extract()?;
    CoreDataType::from_str(&text).map_err(schema_err)
}

/// Builds a scalar of an explicit `dtype` from a Python value (`None` → a typed null).
/// The primitive / string / binary / JSON / BSON families build directly; the richer
/// types (decimal, temporal, nested) go through :meth:`from_str` / :meth:`from_bytes`.
fn build_typed(value: &Bound<'_, PyAny>, dtype: &CoreDataType) -> PyResult<CoreScalar> {
    use CoreDataType as D;
    if value.is_none() {
        return Ok(CoreScalar::null(dtype.clone()));
    }
    Ok(match dtype {
        D::Boolean => CoreScalar::boolean(value.extract()?),
        D::Int { bits, signed } => CoreScalar::int(value.extract::<i128>()?, *bits, *signed),
        D::Float { bits } => CoreScalar::float(value.extract::<f64>()?, *bits),
        D::Varchar {
            charset,
            large,
            view,
            size,
        } => CoreScalar::Utf8 {
            value: value.extract()?,
            charset: *charset,
            large: *large,
            view: *view,
            size: *size,
        },
        D::Binary { large, view, size } => CoreScalar::Binary {
            value: value.extract()?,
            large: *large,
            view: *view,
            size: *size,
        },
        D::Json => CoreScalar::json(value.extract::<String>()?),
        D::Bson => CoreScalar::bson(value.extract::<Vec<u8>>()?),
        other => {
            return Err(scalar_err(ScalarError::Unsupported(format!(
                "construct a '{}' scalar via Scalar.from_str / from_bytes / from_mapping",
                other.to_str()
            ))))
        }
    })
}

/// Infers a scalar from a Python value (boolean checked before int, since Python `bool`
/// subclasses `int`): `bool` → bool, `int` → int64, `float` → float64, `str` → utf8,
/// `bytes` → binary.
fn infer(value: &Bound<'_, PyAny>) -> PyResult<CoreScalar> {
    if value.is_instance_of::<PyBool>() {
        Ok(CoreScalar::boolean(value.extract()?))
    } else if value.is_instance_of::<PyInt>() {
        Ok(CoreScalar::int(value.extract::<i128>()?, 64, true))
    } else if value.is_instance_of::<PyFloat>() {
        Ok(CoreScalar::float(value.extract::<f64>()?, 64))
    } else if value.is_instance_of::<PyString>() {
        Ok(CoreScalar::utf8(value.extract::<String>()?))
    } else if value.is_instance_of::<PyBytes>() || value.is_instance_of::<PyByteArray>() {
        Ok(CoreScalar::binary(value.extract::<Vec<u8>>()?))
    } else {
        Err(PyTypeError::new_err(format!(
            "cannot infer a scalar from '{}'; pass an explicit dtype",
            value.get_type().name()?
        )))
    }
}

/// Renders an unscaled decimal value as its scaled decimal string.
fn decimal_string(value: i256, scale: i8) -> String {
    let digits = value.to_string();
    if scale <= 0 {
        return format!("{digits}{}", "0".repeat((-(scale as i32)) as usize));
    }
    let scale = scale as usize;
    let (sign, digits) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits.as_str()),
    };
    if digits.len() > scale {
        let point = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    } else {
        format!("{sign}0.{}{}", "0".repeat(scale - digits.len()), digits)
    }
}

/// Maps an [`Interval`] to a Python dict of its calendar components.
fn interval_to_py(py: Python<'_>, interval: &Interval) -> PyObject {
    let dict = PyDict::new_bound(py);
    match interval {
        Interval::YearMonth(months) => {
            dict.set_item("months", months).unwrap();
        }
        Interval::DayTime { days, millis } => {
            dict.set_item("days", days).unwrap();
            dict.set_item("millis", millis).unwrap();
        }
        Interval::MonthDayNano {
            months,
            days,
            nanos,
        } => {
            dict.set_item("months", months).unwrap();
            dict.set_item("days", days).unwrap();
            dict.set_item("nanos", nanos).unwrap();
        }
    }
    dict.into()
}

/// Maps a core [`CoreScalar`] to the matching Python object: primitives become native
/// values, temporal scalars the :class:`Date` / :class:`Time` / :class:`DateTime` /
/// :class:`Duration` classes, decimals / intervals a string / dict, and the nested
/// types a list / dict (recursively).
pub(crate) fn value_to_py(py: Python<'_>, scalar: &CoreScalar) -> PyResult<PyObject> {
    use CoreScalar as S;
    Ok(match scalar {
        S::Null(_) => py.None(),
        S::Boolean(b) => b.into_py(py),
        S::Int { value, .. } => value.into_py(py),
        S::Float { value, .. } => value.0.into_py(py),
        S::Utf8 { value, .. } => value.into_py(py),
        S::Json(v) => v.into_py(py),
        S::Binary { value, .. } => PyBytes::new_bound(py, value).into(),
        S::Bson(v) => PyBytes::new_bound(py, v).into(),
        S::Timezone(tz) => crate::timezone::Timezone { inner: tz.clone() }.into_py(py),
        S::Decimal { value, scale, .. } => decimal_string(*value, *scale).into_py(py),
        S::Date { .. } => crate::date::Date {
            inner: scalar.as_date().expect("a date scalar reads as a Date"),
        }
        .into_py(py),
        S::Time { .. } => match scalar.as_time() {
            Some(time) => crate::pytime::Time { inner: time }.into_py(py),
            None => py.None(),
        },
        S::Timestamp { .. } => crate::datetime::DateTime {
            inner: scalar
                .as_datetime()
                .expect("a timestamp scalar reads as a DateTime"),
        }
        .into_py(py),
        S::Duration { .. } => crate::duration::Duration {
            inner: scalar
                .as_duration()
                .expect("a duration scalar reads as a Duration"),
        }
        .into_py(py),
        S::Interval(interval) => interval_to_py(py, interval),
        S::List { values, .. } => {
            let items = values
                .iter()
                .map(|v| value_to_py(py, v))
                .collect::<PyResult<Vec<_>>>()?;
            PyList::new_bound(py, items).into()
        }
        S::Struct { fields, values } => {
            let dict = PyDict::new_bound(py);
            for (field, value) in fields.iter().zip(values) {
                dict.set_item(field.name(), value_to_py(py, value)?)?;
            }
            dict.into()
        }
        S::Map { entries, .. } => {
            let pairs = entries
                .iter()
                .map(|(k, v)| {
                    let tuple = PyTuple::new_bound(py, [value_to_py(py, k)?, value_to_py(py, v)?]);
                    Ok(tuple.into())
                })
                .collect::<PyResult<Vec<PyObject>>>()?;
            PyList::new_bound(py, pairs).into()
        }
    })
}

#[pymethods]
impl Scalar {
    /// Build a scalar from a Python value. Without `dtype` the type is inferred
    /// (`bool` / `int` → int64 / `float` → float64 / `str` → utf8 / `bytes` → binary);
    /// pass `dtype` (a :class:`DataType` or type string) to build a specific type, and
    /// pass ``None`` with a `dtype` for a typed null.
    #[new]
    #[pyo3(signature = (value, dtype = None))]
    fn new(value: &Bound<'_, PyAny>, dtype: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let inner = match dtype {
            Some(obj) => build_typed(value, &resolve_dtype(obj)?)?,
            None => infer(value)?,
        };
        Ok(wrap(inner))
    }

    /// A typed null of `dtype` (a :class:`DataType` or type string).
    #[staticmethod]
    fn null(dtype: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(wrap(CoreScalar::null(resolve_dtype(dtype)?)))
    }

    /// Parse a scalar from its canonical string (``"42::int64"``, ``"'hi'::utf8"``).
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreScalar::from_str(value).map(wrap).map_err(scalar_err)
    }

    /// Reconstruct a scalar from its Arrow-IPC :meth:`to_bytes` form.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        from_bytes(data).map(wrap).map_err(scalar_err)
    }

    /// Build a scalar from a ``{"type": ..., "value": ...}`` component map.
    #[staticmethod]
    fn from_mapping(mapping: BTreeMap<String, String>) -> PyResult<Self> {
        CoreScalar::from_mapping(&mapping)
            .map(wrap)
            .map_err(scalar_err)
    }

    /// The scalar's :class:`DataType`.
    #[getter]
    fn data_type(&self) -> DataType {
        DataType {
            inner: self.inner.data_type(),
        }
    }

    /// Whether this is a null value.
    #[getter]
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The native Python value (``None`` for a null; a :class:`DateTime` / :class:`Date`
    /// / :class:`Time` / :class:`Duration` for temporals; a ``list`` / ``dict`` for the
    /// nested types).
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<PyObject> {
        value_to_py(py, &self.inner)
    }

    /// The value as a `bool`, or ``None``.
    fn as_bool(&self) -> Option<bool> {
        self.inner.as_bool()
    }

    /// The value as an `int`, or ``None``.
    fn as_int(&self) -> Option<i128> {
        self.inner.as_i128()
    }

    /// The value as a `float`, or ``None``.
    fn as_float(&self) -> Option<f64> {
        self.inner.as_f64()
    }

    /// The value as a `str`, or ``None``.
    fn as_str(&self) -> Option<String> {
        self.inner.as_str().map(str::to_string)
    }

    /// The scalar as a native ``dict`` — a struct as ``{field: value}`` or a map as
    /// ``{key: value}`` (recursively). Raises for a non-nested scalar.
    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        use CoreScalar as S;
        match &self.inner {
            S::Struct { fields, values } => {
                let dict = PyDict::new_bound(py);
                for (field, value) in fields.iter().zip(values) {
                    dict.set_item(field.name(), value_to_py(py, value)?)?;
                }
                Ok(dict.into())
            }
            S::Map { entries, .. } => {
                let dict = PyDict::new_bound(py);
                for (key, value) in entries {
                    dict.set_item(value_to_py(py, key)?, value_to_py(py, value)?)?;
                }
                Ok(dict.into())
            }
            other => Err(PyTypeError::new_err(format!(
                "to_dict requires a struct or map scalar, got '{}'",
                other.data_type().to_str()
            ))),
        }
    }

    /// The scalar as a Python **dataclass instance** (struct scalars only): builds a
    /// dataclass named `name` with one field per struct field and returns an instance
    /// (attribute access, ``record.id``). The native-record convenience over
    /// :meth:`to_dict`.
    #[pyo3(signature = (name = "Record"))]
    fn as_dataclass(&self, py: Python<'_>, name: &str) -> PyResult<PyObject> {
        let dict_obj = self.to_dict(py)?;
        let dict = dict_obj.downcast_bound::<PyDict>(py)?;
        let dataclasses = py.import_bound("dataclasses")?;
        let field_names: Vec<String> = dict
            .keys()
            .iter()
            .map(|key| key.extract::<String>())
            .collect::<PyResult<_>>()?;
        let cls = dataclasses.call_method1("make_dataclass", (name, field_names))?;
        cls.call((), Some(dict)).map(|obj| obj.into())
    }

    /// The value as `bytes`, or ``None``.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner.as_bytes().map(|b| PyBytes::new_bound(py, b))
    }

    /// The canonical string (``"42::int64"``).
    fn to_str(&self) -> String {
        self.inner.to_str()
    }

    /// The ``{"type": ..., "value": ...}`` component map.
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Serialise to lossless Arrow-IPC bytes (round-trips via :meth:`from_bytes`).
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.inner.to_bytes().map_err(scalar_err)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.to_bytes(py)
    }

    fn __eq__(&self, other: &Scalar) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        // Hash through the core `Scalar`'s `Hash` (floats by canonical bits), so the
        // Python `__hash__`/`__eq__` contract holds (-0.0 == 0.0, NaN == NaN).
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __str__(&self) -> String {
        self.inner.to_str()
    }

    fn __repr__(&self) -> String {
        format!("Scalar({})", self.inner.to_str())
    }

    /// Reconstruct losslessly through Arrow-IPC bytes (pickle / copy).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(PyObject, (PyObject,))> {
        let from_bytes = py.get_type_bound::<Self>().getattr("from_bytes")?;
        let bytes = PyBytes::new_bound(py, &self.inner.to_bytes().map_err(scalar_err)?);
        Ok((from_bytes.into(), (bytes.into(),)))
    }
}
