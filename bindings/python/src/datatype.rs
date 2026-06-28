//! The `DataType` pyclass — the simplified, Arrow-compatible logical type.

use pyo3::prelude::*;
use std::collections::BTreeMap;
use yggdryl_core::Timezone as CoreTimezone;
use yggdryl_schema::{
    Charset, DataType as CoreDataType, IntervalUnit, MergeStrategy, Numeric, UnionMode,
};

use crate::field::Field;
use crate::timezone::Timezone;
use crate::{schema_err, time_unit_from};

/// A logical data type (primitive / logical / nested, plus the ``any`` wildcard).
#[pyclass(name = "DataType", module = "yggdryl")]
#[derive(Clone)]
pub struct DataType {
    pub(crate) inner: CoreDataType,
}

fn wrap(inner: CoreDataType) -> DataType {
    DataType { inner }
}

#[pymethods]
impl DataType {
    /// Parse a canonical type string (e.g. ``"int64"``, ``"timestamp[us, UTC]"``,
    /// ``"struct[id: int64 not null, name: utf8]"``).
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        CoreDataType::from_str(value).map(wrap).map_err(schema_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        DataType::new(value)
    }

    // ---- constructors ----

    /// The ``any`` wildcard (matches and merges with everything).
    #[staticmethod]
    fn any() -> Self {
        wrap(CoreDataType::Any)
    }

    /// The null type.
    #[staticmethod]
    fn null() -> Self {
        wrap(CoreDataType::Null)
    }

    /// The boolean type.
    #[staticmethod]
    fn boolean() -> Self {
        wrap(CoreDataType::Boolean)
    }

    /// The fixed-width integer for `(bits, signed)` — the builder over the concrete
    /// ``int8`` … ``uint64`` types (default ``int64``). Only the standard widths
    /// (8/16/32/64) are types; a non-standard width rounds up to the next supported one.
    #[staticmethod]
    #[pyo3(signature = (bits = 64, signed = true))]
    fn int(bits: u16, signed: bool) -> Self {
        wrap(CoreDataType::int(bits, signed))
    }

    /// A signed 8-bit integer (``int8``).
    #[staticmethod]
    fn int8() -> Self {
        wrap(CoreDataType::int8())
    }

    /// A signed 16-bit integer (``int16``).
    #[staticmethod]
    fn int16() -> Self {
        wrap(CoreDataType::int16())
    }

    /// A signed 32-bit integer (``int32``).
    #[staticmethod]
    fn int32() -> Self {
        wrap(CoreDataType::int32())
    }

    /// A signed 64-bit integer (``int64``).
    #[staticmethod]
    fn int64() -> Self {
        wrap(CoreDataType::int64())
    }

    /// An unsigned 8-bit integer (``uint8``).
    #[staticmethod]
    fn uint8() -> Self {
        wrap(CoreDataType::uint8())
    }

    /// An unsigned 16-bit integer (``uint16``).
    #[staticmethod]
    fn uint16() -> Self {
        wrap(CoreDataType::uint16())
    }

    /// An unsigned 32-bit integer (``uint32``).
    #[staticmethod]
    fn uint32() -> Self {
        wrap(CoreDataType::uint32())
    }

    /// An unsigned 64-bit integer (``uint64``).
    #[staticmethod]
    fn uint64() -> Self {
        wrap(CoreDataType::uint64())
    }

    /// A signed integer at the default width (``int64``).
    #[staticmethod]
    fn integer() -> Self {
        wrap(CoreDataType::integer())
    }

    /// The fixed-width float for `bits` — the builder over ``float16`` / ``float32`` /
    /// ``float64`` (default 64). Only the IEEE widths (16/32/64) are types; a
    /// non-standard width rounds up to the next supported one.
    #[staticmethod]
    #[pyo3(signature = (bits = 64))]
    fn float(bits: u16) -> Self {
        wrap(CoreDataType::float(bits))
    }

    /// A half-precision (16-bit) float (``float16``).
    #[staticmethod]
    fn float16() -> Self {
        wrap(CoreDataType::float16())
    }

    /// A single-precision (32-bit) float (``float32``).
    #[staticmethod]
    fn float32() -> Self {
        wrap(CoreDataType::float32())
    }

    /// A double-precision (64-bit) float (``float64``).
    #[staticmethod]
    fn float64() -> Self {
        wrap(CoreDataType::float64())
    }

    /// A float at the default width (``float64``).
    #[staticmethod]
    fn floating() -> Self {
        wrap(CoreDataType::floating())
    }

    /// A decimal with `(precision, scale)`, stored in `bits` (32/64/128/256;
    /// default 128).
    #[staticmethod]
    #[pyo3(signature = (precision, scale = 0, bits = 128))]
    fn decimal(precision: u8, scale: i8, bits: u16) -> Self {
        wrap(CoreDataType::decimal_with(precision, scale, bits))
    }

    /// A 32-bit decimal with `(precision, scale)` (``decimal32``).
    #[staticmethod]
    #[pyo3(signature = (precision, scale = 0))]
    fn decimal32(precision: u8, scale: i8) -> Self {
        wrap(CoreDataType::decimal32(precision, scale))
    }

    /// A 64-bit decimal with `(precision, scale)` (``decimal64``).
    #[staticmethod]
    #[pyo3(signature = (precision, scale = 0))]
    fn decimal64(precision: u8, scale: i8) -> Self {
        wrap(CoreDataType::decimal64(precision, scale))
    }

    /// A 128-bit decimal with `(precision, scale)` (``decimal128``).
    #[staticmethod]
    #[pyo3(signature = (precision, scale = 0))]
    fn decimal128(precision: u8, scale: i8) -> Self {
        wrap(CoreDataType::decimal128(precision, scale))
    }

    /// A 256-bit decimal with `(precision, scale)` (``decimal256``).
    #[staticmethod]
    #[pyo3(signature = (precision, scale = 0))]
    fn decimal256(precision: u8, scale: i8) -> Self {
        wrap(CoreDataType::decimal256(precision, scale))
    }

    /// A string with the given charset, large/view flags and optional fixed `size`
    /// (``None`` = variable-length).
    #[staticmethod]
    #[pyo3(signature = (charset = "utf8", large = false, view = false, size = None))]
    fn varchar(charset: &str, large: bool, view: bool, size: Option<i32>) -> PyResult<Self> {
        let charset = Charset::from_str(charset).map_err(|e| schema_err(e.into()))?;
        Ok(wrap(CoreDataType::varchar_with(charset, large, view, size)))
    }

    /// A fixed-length UTF-8 string of `size` characters (SQL ``char(n)``).
    #[staticmethod]
    fn fixed_size_varchar(size: i32) -> Self {
        wrap(CoreDataType::fixed_size_varchar(size))
    }

    /// Variable-length opaque bytes (``large`` for 64-bit offsets, ``view`` layout).
    #[staticmethod]
    #[pyo3(signature = (large = false, view = false))]
    fn binary(large: bool, view: bool) -> Self {
        wrap(CoreDataType::Binary {
            large,
            view,
            size: None,
        })
    }

    /// Fixed-width opaque bytes of `size` bytes.
    #[staticmethod]
    fn fixed_size_binary(size: i32) -> Self {
        wrap(CoreDataType::fixed_size_binary(size))
    }

    /// A calendar date (``large`` selects millisecond/64-bit over day/32-bit).
    #[staticmethod]
    #[pyo3(signature = (large = false))]
    fn date(large: bool) -> Self {
        wrap(CoreDataType::Date { large })
    }

    /// A time of day in the given unit (``"s"`` / ``"ms"`` / ``"us"`` / ``"ns"``).
    #[staticmethod]
    fn time(unit: &str) -> PyResult<Self> {
        Ok(wrap(CoreDataType::Time {
            unit: time_unit_from(unit)?,
        }))
    }

    /// A timestamp in the given unit, optionally zoned (a zone name).
    #[staticmethod]
    #[pyo3(signature = (unit, timezone = None))]
    fn timestamp(unit: &str, timezone: Option<&str>) -> PyResult<Self> {
        let tz = match timezone {
            Some(name) => Some(CoreTimezone::from_str(name).map_err(crate::time_err)?),
            None => None,
        };
        Ok(wrap(CoreDataType::timestamp(time_unit_from(unit)?, tz)))
    }

    /// Elapsed time in the given unit.
    #[staticmethod]
    fn duration(unit: &str) -> PyResult<Self> {
        Ok(wrap(CoreDataType::Duration {
            unit: time_unit_from(unit)?,
        }))
    }

    /// A calendar interval (``"year_month"`` / ``"day_time"`` / ``"month_day_nano"``).
    #[staticmethod]
    fn interval(unit: &str) -> PyResult<Self> {
        Ok(wrap(CoreDataType::Interval {
            unit: IntervalUnit::from_str(unit).map_err(schema_err)?,
        }))
    }

    /// A dictionary of `key` indices into `value`s.
    #[staticmethod]
    fn dictionary(key: &DataType, value: &DataType) -> Self {
        wrap(CoreDataType::dictionary(
            key.inner.clone(),
            value.inner.clone(),
        ))
    }

    /// JSON text (a string-backed logical type).
    #[staticmethod]
    fn json() -> Self {
        wrap(CoreDataType::json())
    }

    /// A BSON document (a binary-backed logical type).
    #[staticmethod]
    fn bson() -> Self {
        wrap(CoreDataType::bson())
    }

    /// A variable-length list of the item :class:`Field`.
    #[staticmethod]
    fn list(item: &Field) -> Self {
        wrap(CoreDataType::list(item.inner.clone()))
    }

    /// A 64-bit-offset large list of the item :class:`Field`.
    #[staticmethod]
    fn large_list(item: &Field) -> Self {
        wrap(CoreDataType::large_list(item.inner.clone()))
    }

    /// A fixed-length list of the item :class:`Field`, `size` elements long.
    #[staticmethod]
    fn fixed_size_list(item: &Field, size: i32) -> Self {
        wrap(CoreDataType::fixed_size_list(item.inner.clone(), size))
    }

    /// A struct of the given :class:`Field` list (a struct type is a schema).
    #[staticmethod]
    fn struct_(fields: Vec<Field>) -> Self {
        wrap(CoreDataType::struct_(
            fields.into_iter().map(|f| f.inner).collect(),
        ))
    }

    /// A map from `key` to `value`.
    #[staticmethod]
    #[pyo3(signature = (key, value, sorted = false))]
    fn map(key: &DataType, value: &DataType, sorted: bool) -> Self {
        wrap(CoreDataType::map(
            key.inner.clone(),
            value.inner.clone(),
            sorted,
        ))
    }

    /// A union of the given alternatives (``mode`` is ``"sparse"`` or ``"dense"``).
    #[staticmethod]
    #[pyo3(signature = (fields, mode = "sparse"))]
    fn union(fields: Vec<Field>, mode: &str) -> PyResult<Self> {
        let mode = UnionMode::from_str(mode).map_err(schema_err)?;
        Ok(wrap(CoreDataType::union(
            fields.into_iter().map(|f| f.inner).collect(),
            mode,
        )))
    }

    /// A run-end-encoded type of `run_ends` (an integer) and `values`.
    #[staticmethod]
    fn run_end_encoded(run_ends: &DataType, values: &DataType) -> Self {
        wrap(CoreDataType::run_end_encoded(
            run_ends.inner.clone(),
            values.inner.clone(),
        ))
    }

    // ---- accessors ----

    /// The category: ``"any"`` / ``"primitive"`` / ``"logical"`` / ``"nested"``.
    #[getter]
    fn category(&self) -> &'static str {
        self.inner.category().as_str()
    }

    /// The physical width in bytes for byte-aligned fixed-width types, else ``None``.
    #[getter]
    fn byte_size(&self) -> Option<u16> {
        self.inner.byte_size()
    }

    /// Whether this uses the large (64-bit offset / wide) form.
    #[getter]
    fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether this uses the view layout.
    #[getter]
    fn is_view(&self) -> bool {
        self.inner.is_view()
    }

    /// Whether this type has a fixed (non-variable) length.
    #[getter]
    fn is_fixed_size(&self) -> bool {
        self.inner.is_fixed_size()
    }

    /// The string charset, if a string type.
    #[getter]
    fn charset(&self) -> Option<&'static str> {
        self.inner.charset().map(|c| c.as_str())
    }

    /// Whether a numeric type is signed — the integer flag, always ``True`` for
    /// floats / decimals — else ``None`` (the :class:`Numeric` interface).
    #[getter]
    fn signed(&self) -> Option<bool> {
        self.inner.signed()
    }

    /// The native Rust storage type name of a fixed-width numeric type (``"i32"`` /
    /// ``"f16"`` / ``"i128"`` / ``"i256"`` / …), else ``None``.
    #[getter]
    fn name(&self) -> Option<&'static str> {
        self.inner.name()
    }

    /// The time unit of a temporal type carrying one, else ``None``.
    #[getter]
    fn time_unit(&self) -> Option<&'static str> {
        self.inner.time_unit().map(|u| u.as_str())
    }

    /// The display :class:`Timezone` of a timestamp, else ``None``.
    #[getter]
    fn timezone(&self) -> Option<Timezone> {
        self.inner
            .timezone()
            .cloned()
            .map(|inner| Timezone { inner })
    }

    /// The ``(precision, scale)`` of a decimal type, else ``None``.
    #[getter]
    fn decimal_parts(&self) -> Option<(u8, i8)> {
        self.inner.decimal_parts()
    }

    /// The child :class:`Field` list of a nested type.
    fn children(&self) -> Vec<Field> {
        self.inner
            .children()
            .into_iter()
            .map(|f| Field { inner: f.clone() })
            .collect()
    }

    // ---- predicates ----
    fn is_any(&self) -> bool {
        self.inner.is_any()
    }
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }
    fn is_boolean(&self) -> bool {
        self.inner.is_boolean()
    }
    fn is_integer(&self) -> bool {
        self.inner.is_integer()
    }
    fn is_signed_integer(&self) -> bool {
        self.inner.is_signed_integer()
    }
    fn is_unsigned_integer(&self) -> bool {
        self.inner.is_unsigned_integer()
    }
    fn is_floating(&self) -> bool {
        self.inner.is_floating()
    }
    fn is_numeric(&self) -> bool {
        self.inner.is_numeric()
    }
    fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }
    fn is_string(&self) -> bool {
        self.inner.is_string()
    }
    fn is_primitive(&self) -> bool {
        self.inner.is_primitive()
    }
    fn is_logical(&self) -> bool {
        self.inner.is_logical()
    }
    fn is_temporal(&self) -> bool {
        self.inner.is_temporal()
    }
    fn is_decimal(&self) -> bool {
        self.inner.is_decimal()
    }
    fn is_dictionary(&self) -> bool {
        self.inner.is_dictionary()
    }
    fn is_json(&self) -> bool {
        self.inner.is_json()
    }
    fn is_bson(&self) -> bool {
        self.inner.is_bson()
    }
    fn is_nested(&self) -> bool {
        self.inner.is_nested()
    }
    fn is_list(&self) -> bool {
        self.inner.is_list()
    }
    fn is_struct(&self) -> bool {
        self.inner.is_struct()
    }
    fn is_union(&self) -> bool {
        self.inner.is_union()
    }
    fn is_map(&self) -> bool {
        self.inner.is_map()
    }

    // ---- conversion / merge ----

    /// Whether a value of this type can be cast to `other`.
    fn can_cast_to(&self, other: &DataType) -> bool {
        self.inner.can_cast_to(&other.inner)
    }

    /// The least type both can widen to, or ``None``.
    fn common_type(&self, other: &DataType) -> Option<DataType> {
        self.inner.common_type(&other.inner).map(wrap)
    }

    /// Merge with `other` under a strategy (``"strict"`` / ``"promote"`` /
    /// ``"permissive"``).
    #[pyo3(signature = (other, strategy = "promote"))]
    fn merge(&self, other: &DataType, strategy: &str) -> PyResult<DataType> {
        let strategy = MergeStrategy::from_str(strategy).map_err(schema_err)?;
        self.inner
            .merge(&other.inner, strategy)
            .map(wrap)
            .map_err(schema_err)
    }

    // ---- serialisation ----

    /// Render to a dict (the single ``type`` key).
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Build from a dict (the ``type`` key).
    #[staticmethod]
    fn from_mapping(fields: BTreeMap<String, String>) -> PyResult<Self> {
        CoreDataType::from_mapping(&fields)
            .map(wrap)
            .map_err(schema_err)
    }

    /// Serialise to a lossless structural JSON string.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Parse from the structural JSON of :meth:`to_json`.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreDataType::from_json(value).map(wrap).map_err(schema_err)
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("DataType('{}')", self.inner)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        crate::hash_str(&self.inner.to_string())
    }

    /// Reconstruct losslessly through structural JSON.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(PyObject, (String,))> {
        let from_json = py.get_type_bound::<Self>().getattr("from_json")?;
        Ok((from_json.into(), (self.inner.to_json(),)))
    }
}
