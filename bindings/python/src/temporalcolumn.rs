//! The `yggdryl.temporal` submodule's **columnar** temporal types — one nullable temporal column
//! per concept+width (`Date32Serie` / `Date64Serie`, `Time32Serie` / `Time64Serie`, `Ts32Serie` /
//! `Ts64Serie` / `Ts96Serie`, `Duration32Serie` / `Duration64Serie`), mirroring
//! `yggdryl_core::io::fixed`'s `TemporalSerie<B>` and registered alongside the temporal value types.
//!
//! A column fixes one `(unit, tz)` (Arrow's model): a value pushed / set at another resolution is
//! re-expressed at the column's unit (a guided `ValueError` if it does not fit). **Cells cross two
//! ways** — as the value type's ISO-8601 string (`get` / `push` / `set` / the constructor) and as the
//! raw epoch/physical count as a Python int (`get_epoch` / `from_epochs`) — matching the temporal
//! *value* types' string / epoch conventions. A column is mutable, so (like the decimal `Serie`) it
//! is deliberately unhashable.
//!
//! Each column also speaks the **zero-copy Arrow C Data Interface** (PyCapsule protocol), so
//! `pyarrow.array(col)` exports it with no payload copy and `Ts64Serie.from_arrow(pa_array)` imports
//! it back. The exported schema carries the column's `(unit, tz)` in field metadata (the recovery
//! path for `ts96`, whose `FixedSizeBinary(12)` Arrow type carries neither).

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::ffi::CString;

use arrow_array::ffi::{from_ffi, to_ffi, FFI_ArrowArray};
use arrow_array::Array;
use arrow_schema::ffi::FFI_ArrowSchema;
use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyCapsule, PyList};

use yggdryl_core::io::fixed::temporal::{
    self as core, TemporalBacking, TemporalNative, TemporalSerie,
};
use yggdryl_core::io::{DataTypeId, IoError};

use crate::temporal::{Date32, Date64, Duration32, Duration64, Time32, Time64, Ts32, Ts64, Ts96};
use crate::types::{DataType, Field};

/// Maps a core [`TemporalError`](core::TemporalError) to a Python `ValueError` (guided text intact).
fn temporal_err(error: core::TemporalError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Maps an [`IoError`] (byte-codec / Arrow interop) to a Python `ValueError`.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Maps an Arrow error to a Python `ValueError`.
fn arrow_err(error: arrow_schema::ArrowError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Parses a time-unit string (`"ns"`, `"second"`, …) or raises `ValueError`.
fn parse_unit(text: &str) -> PyResult<core::TimeUnit> {
    core::TimeUnit::parse(text)
        .ok_or_else(|| PyValueError::new_err(format!("unknown time unit: {text:?}")))
}

/// Parses a timezone string (`"UTC"`, `"Europe/Paris"`, `"+02:00"`, `"naive"`/`""`) or raises
/// `ValueError`.
fn parse_tz(text: &str) -> PyResult<core::Tz> {
    core::Tz::parse(text)
        .ok_or_else(|| PyValueError::new_err(format!("unknown timezone: {text:?}")))
}

/// A capsule name (`"arrow_schema"` / `"arrow_array"`), as the Arrow PyCapsule protocol requires.
fn capsule_name(name: &str) -> CString {
    CString::new(name).expect("a static ASCII capsule name has no interior NUL")
}

/// The Arrow [`Field`](arrow_schema::Field) (name `""`) carrying a temporal column's exact logical
/// type for the C Data Interface — `data_type` from the array itself (so the exported schema always
/// matches the exported array) plus the `(unit, tz)` in metadata (the recovery path for `ts96`,
/// whose `FixedSizeBinary(12)` carries neither).
fn arrow_field_of<B: TemporalBacking>(
    serie: &TemporalSerie<B>,
    data_type: arrow_schema::DataType,
) -> arrow_schema::Field {
    let template = serie.to_field("").to_arrow();
    arrow_schema::Field::new("", data_type, serie.has_nulls())
        .with_metadata(template.metadata().clone())
}

/// Generates one nullable temporal **column** wrapper for a concept+width.
///
/// `$Serie` is the pyclass, `$CoreSerie` the core `TemporalSerie` alias it wraps, `$CoreValue` the
/// value type it parses ISO strings into / rebuilds from a count, and `$Value` the temporal *value*
/// wrapper `get_scalar` hands back.
macro_rules! py_temporal_col {
    ($Serie:ident, $CoreSerie:ty, $CoreValue:ty, $Value:ident, $id:expr, $lit:literal) => {
        #[doc = concat!("A nullable column of `", $lit, "` values at one `(unit, tz)`.")]
        #[pyclass(module = "yggdryl.temporal")]
        #[derive(Clone)]
        pub struct $Serie {
            pub(crate) inner: $CoreSerie,
        }

        #[pymethods]
        impl $Serie {
            /// A column at `(unit, tz)` from a list of ISO-8601-string-or-`None` (empty by default).
            /// Each present value is re-expressed at the column's unit (a `ValueError` if it does not
            /// fit). `tz` defaults to `"naive"` (only the timestamp columns carry a real zone).
            #[new]
            #[pyo3(signature = (unit, tz = "naive", values = None))]
            fn new(unit: &str, tz: &str, values: Option<Vec<Option<String>>>) -> PyResult<Self> {
                let unit = parse_unit(unit)?;
                let tz = parse_tz(tz)?;
                match values {
                    None => Ok(Self {
                        inner: <$CoreSerie>::new(unit, tz),
                    }),
                    Some(values) => {
                        let mut options = Vec::with_capacity(values.len());
                        for value in values {
                            options.push(match value {
                                Some(text) => {
                                    Some(text.parse::<$CoreValue>().map_err(temporal_err)?)
                                }
                                None => None,
                            });
                        }
                        <$CoreSerie>::from_options(unit, tz, &options)
                            .map(|inner| Self { inner })
                            .map_err(temporal_err)
                    }
                }
            }

            /// A column at `(unit, tz)` from a list of raw epoch/physical-count-`int`-or-`None` — the
            /// count is interpreted in the column's `unit` (the inverse of [`get_epoch`](Self::get_epoch)).
            #[staticmethod]
            #[pyo3(signature = (unit, tz = "naive", values = None))]
            fn from_epochs(
                unit: &str,
                tz: &str,
                values: Option<Vec<Option<i128>>>,
            ) -> PyResult<Self> {
                let unit = parse_unit(unit)?;
                let tz = parse_tz(tz)?;
                let values = values.unwrap_or_default();
                let mut options = Vec::with_capacity(values.len());
                for value in values {
                    options.push(match value {
                        Some(count) => Some(
                            <$CoreValue as TemporalNative>::from_count(count, unit, tz)
                                .map_err(temporal_err)?,
                        ),
                        None => None,
                    });
                }
                <$CoreSerie>::from_options(unit, tz, &options)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }

            /// Appends one element — an ISO-8601 string (re-expressed at the column's unit), or
            /// `None` for a null.
            #[pyo3(signature = (value = None))]
            fn push(&mut self, value: Option<&str>) -> PyResult<()> {
                let parsed = match value {
                    Some(text) => Some(text.parse::<$CoreValue>().map_err(temporal_err)?),
                    None => None,
                };
                self.inner.push(parsed).map_err(temporal_err)
            }

            /// The value at `index` as its ISO-8601 string, or `None` if null or out of range.
            fn get(&self, index: usize) -> Option<String> {
                self.inner.get(index).map(|value| value.to_string())
            }

            /// The raw epoch/physical count at `index` as a Python `int`, or `None` if null or out of
            /// range (the inverse of [`from_epochs`](Self::from_epochs)).
            fn get_epoch(&self, index: usize) -> Option<i128> {
                self.inner.get_count(index)
            }

            /// Element `index` as the temporal **value** type, or `None` if null or out of range.
            fn get_scalar(&self, index: usize) -> Option<$Value> {
                self.inner.get(index).map(|inner| $Value { inner })
            }

            /// Overwrites element `index` — an ISO-8601 string (re-expressed at the column's unit), or
            /// `None` for a null; raises `ValueError` out of range or if the value does not fit.
            #[pyo3(signature = (index, value = None))]
            fn set(&mut self, index: usize, value: Option<&str>) -> PyResult<()> {
                let parsed = match value {
                    Some(text) => Some(text.parse::<$CoreValue>().map_err(temporal_err)?),
                    None => None,
                };
                self.inner.set(index, parsed).map_err(temporal_err)
            }

            /// The column resolution as its abbreviation (`"s"`, `"ns"`, `"d"`, …).
            #[getter]
            fn unit(&self) -> String {
                self.inner.unit().abbreviation().to_string()
            }

            /// The column timezone name (empty for naive).
            #[getter]
            fn timezone(&self) -> String {
                self.inner.timezone().name()
            }

            /// The number of null elements.
            #[getter]
            fn null_count(&self) -> usize {
                self.inner.null_count()
            }

            /// Whether the column carries any nulls.
            #[getter]
            fn has_nulls(&self) -> bool {
                self.inner.has_nulls()
            }

            /// Whether the column is empty.
            fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// This column's [`DataType`].
            #[getter]
            fn data_type(&self) -> DataType {
                DataType::of($id)
            }

            /// A [`Field`] naming this column — its temporal [`DataType`], nullability (inferred from
            /// its nulls), and the `(unit, tz)` recorded in metadata.
            fn to_field(&self, name: &str) -> Field {
                Field {
                    inner: self.inner.to_field(name).erase(),
                }
            }

            /// The elements as a list of ISO-string-or-`None`, in order.
            fn to_strings(&self) -> Vec<Option<String>> {
                (0..self.inner.len())
                    .map(|index| self.inner.get(index).map(|value| value.to_string()))
                    .collect()
            }

            /// This column's canonical bytes (`[len][unit][tz][flags][validity?][counts]`).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a column from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                <$CoreSerie>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(io_err)
            }

            fn __len__(&self) -> usize {
                self.inner.len()
            }

            fn __bool__(&self) -> bool {
                !self.inner.is_empty()
            }

            /// Random access — `col[i]` returns the ISO string or `None` (negative indices allowed);
            /// raises `IndexError` out of range.
            fn __getitem__(&self, index: isize) -> PyResult<Option<String>> {
                let len = self.inner.len() as isize;
                let resolved = if index < 0 { index + len } else { index };
                if resolved < 0 || resolved >= len {
                    return Err(PyIndexError::new_err("Serie index out of range"));
                }
                Ok(self
                    .inner
                    .get(resolved as usize)
                    .map(|value| value.to_string()))
            }

            /// Iterates the elements as ISO-string-or-`None`, in order.
            fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                Ok(PyList::new_bound(py, self.to_strings())
                    .call_method0("__iter__")?
                    .unbind())
            }

            fn __eq__(&self, other: &Self) -> bool {
                self.inner == other.inner
            }

            /// An explicit copy.
            fn copy(&self) -> Self {
                self.clone()
            }
            fn __copy__(&self) -> Self {
                self.clone()
            }
            fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
                self.clone()
            }

            /// Pickles through `deserialize_bytes`.
            fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
                let ctor = py
                    .get_type_bound::<$Serie>()
                    .getattr("deserialize_bytes")?
                    .unbind();
                let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                    .into_any()
                    .unbind();
                Ok((ctor, (state,)))
            }

            fn __repr__(&self) -> String {
                format!(
                    "{}(len={}, unit={:?}, tz={:?}, null_count={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.unit().abbreviation(),
                    self.inner.timezone().name(),
                    self.inner.null_count()
                )
            }

            // ---- zero-copy Arrow C Data Interface (PyCapsule protocol) ---------------------

            /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
            fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
                let array = self.inner.to_arrow_array().map_err(io_err)?;
                let field = arrow_field_of(&self.inner, array.data_type().clone());
                let schema = FFI_ArrowSchema::try_from(&field).map_err(arrow_err)?;
                let capsule = PyCapsule::new_bound(py, schema, Some(capsule_name("arrow_schema")))?;
                Ok(capsule.into_any().unbind())
            }

            /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
            /// `requested_schema` hint is accepted for protocol compatibility and ignored (this column
            /// always exports its native schema).
            #[pyo3(signature = (requested_schema = None))]
            fn __arrow_c_array__(
                &self,
                py: Python<'_>,
                requested_schema: Option<PyObject>,
            ) -> PyResult<(PyObject, PyObject)> {
                let _ = requested_schema;
                let array = self.inner.to_arrow_array().map_err(io_err)?;
                let field = arrow_field_of(&self.inner, array.data_type().clone());
                let data = array.into_data();
                // `to_ffi` derives a schema from the data type alone; we export our metadata-carrying
                // `field` instead (same type, so it still matches the array exactly).
                let (ffi_array, _bare_schema) = to_ffi(&data).map_err(arrow_err)?;
                let ffi_schema = FFI_ArrowSchema::try_from(&field).map_err(arrow_err)?;
                let schema_capsule =
                    PyCapsule::new_bound(py, ffi_schema, Some(capsule_name("arrow_schema")))?;
                let array_capsule =
                    PyCapsule::new_bound(py, ffi_array, Some(capsule_name("arrow_array")))?;
                Ok((
                    schema_capsule.into_any().unbind(),
                    array_capsule.into_any().unbind(),
                ))
            }

            /// Imports any object exposing the Arrow C Data Interface (a pyarrow temporal array) into
            /// this column, zero-copy on the native-width path — the inverse of `__arrow_c_array__`.
            #[staticmethod]
            fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
                let pair = obj.call_method0("__arrow_c_array__")?;
                let (schema_cap, array_cap): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
                    pair.extract()?;
                let schema_cap = schema_cap.downcast::<PyCapsule>()?;
                let array_cap = array_cap.downcast::<PyCapsule>()?;
                // Move the FFI structs out of the producer's capsules, then blank the sources so the
                // producer's capsule destructors see a released struct and do not double-free.
                let (ffi_array, ffi_schema) = unsafe {
                    let array_ptr = array_cap.pointer() as *mut FFI_ArrowArray;
                    let schema_ptr = schema_cap.pointer() as *mut FFI_ArrowSchema;
                    let array = std::ptr::read(array_ptr);
                    let schema = std::ptr::read(schema_ptr);
                    std::ptr::write(array_ptr, FFI_ArrowArray::empty());
                    std::ptr::write(schema_ptr, FFI_ArrowSchema::empty());
                    (array, schema)
                };
                // Recover the full field (type + metadata) so a `ts96` `FixedSizeBinary` column
                // restores its `(unit, tz)` from the reserved metadata keys.
                let field = arrow_schema::Field::try_from(&ffi_schema).map_err(arrow_err)?;
                // SAFETY: the FFI structs were produced by a conforming Arrow C Data Interface
                // exporter, and we took ownership of them above (blanking the sources).
                let data = unsafe { from_ffi(ffi_array, &ffi_schema) }.map_err(arrow_err)?;
                let array = arrow_array::make_array(data);
                <$CoreSerie>::from_arrow_array(array.as_ref(), &field)
                    .map(|inner| Self { inner })
                    .map_err(io_err)
            }
        }
    };
}

py_temporal_col!(
    Date32Serie,
    core::Date32Serie,
    core::Date32,
    Date32,
    DataTypeId::Date32,
    "date32"
);
py_temporal_col!(
    Date64Serie,
    core::Date64Serie,
    core::Date64,
    Date64,
    DataTypeId::Date64,
    "date64"
);
py_temporal_col!(
    Time32Serie,
    core::Time32Serie,
    core::Time32,
    Time32,
    DataTypeId::Time32,
    "time32"
);
py_temporal_col!(
    Time64Serie,
    core::Time64Serie,
    core::Time64,
    Time64,
    DataTypeId::Time64,
    "time64"
);
py_temporal_col!(
    Ts32Serie,
    core::Ts32Serie,
    core::Ts32,
    Ts32,
    DataTypeId::Ts32,
    "ts32"
);
py_temporal_col!(
    Ts64Serie,
    core::Ts64Serie,
    core::Ts64,
    Ts64,
    DataTypeId::Ts64,
    "ts64"
);
py_temporal_col!(
    Ts96Serie,
    core::Ts96Serie,
    core::Ts96,
    Ts96,
    DataTypeId::Ts96,
    "ts96"
);
py_temporal_col!(
    Duration32Serie,
    core::Duration32Serie,
    core::Duration32,
    Duration32,
    DataTypeId::Duration32,
    "duration32"
);
py_temporal_col!(
    Duration64Serie,
    core::Duration64Serie,
    core::Duration64,
    Duration64,
    DataTypeId::Duration64,
    "duration64"
);

/// Adds the temporal `Serie` column classes to the `yggdryl.temporal` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Date32Serie>()?;
    module.add_class::<Date64Serie>()?;
    module.add_class::<Time32Serie>()?;
    module.add_class::<Time64Serie>()?;
    module.add_class::<Ts32Serie>()?;
    module.add_class::<Ts64Serie>()?;
    module.add_class::<Ts96Serie>()?;
    module.add_class::<Duration32Serie>()?;
    module.add_class::<Duration64Serie>()?;
    Ok(())
}
