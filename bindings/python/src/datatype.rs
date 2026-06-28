//! The `DataType` pyclass — a primitive / logical / nested type tagged by a `u8`
//! `DataTypeId`.

use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use yggdryl_core::Timezone as CoreTimezone;
use yggdryl_schema::{DataType as CoreDataType, IntervalUnit};

use crate::field::Field;
use crate::{time_err, time_unit_from};

/// A logical data type (primitive / logical / nested).
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
    // ---- primitive constructors ----

    /// The null type.
    #[staticmethod]
    fn null() -> Self {
        wrap(CoreDataType::null())
    }
    /// The boolean type.
    #[staticmethod]
    fn boolean() -> Self {
        wrap(CoreDataType::boolean())
    }
    /// A signed 8-bit integer.
    #[staticmethod]
    fn int8() -> Self {
        wrap(CoreDataType::int8())
    }
    /// A signed 16-bit integer.
    #[staticmethod]
    fn int16() -> Self {
        wrap(CoreDataType::int16())
    }
    /// A signed 32-bit integer.
    #[staticmethod]
    fn int32() -> Self {
        wrap(CoreDataType::int32())
    }
    /// A signed 64-bit integer.
    #[staticmethod]
    fn int64() -> Self {
        wrap(CoreDataType::int64())
    }
    /// An unsigned 8-bit integer.
    #[staticmethod]
    fn uint8() -> Self {
        wrap(CoreDataType::uint8())
    }
    /// An unsigned 16-bit integer.
    #[staticmethod]
    fn uint16() -> Self {
        wrap(CoreDataType::uint16())
    }
    /// An unsigned 32-bit integer.
    #[staticmethod]
    fn uint32() -> Self {
        wrap(CoreDataType::uint32())
    }
    /// An unsigned 64-bit integer.
    #[staticmethod]
    fn uint64() -> Self {
        wrap(CoreDataType::uint64())
    }
    /// A half-precision (16-bit) float.
    #[staticmethod]
    fn float16() -> Self {
        wrap(CoreDataType::float16())
    }
    /// A single-precision (32-bit) float.
    #[staticmethod]
    fn float32() -> Self {
        wrap(CoreDataType::float32())
    }
    /// A double-precision (64-bit) float.
    #[staticmethod]
    fn float64() -> Self {
        wrap(CoreDataType::float64())
    }
    /// A UTF-8 string.
    #[staticmethod]
    fn utf8() -> Self {
        wrap(CoreDataType::utf8())
    }
    /// Opaque bytes.
    #[staticmethod]
    fn binary() -> Self {
        wrap(CoreDataType::binary())
    }

    // ---- logical constructors ----

    /// A decimal with `(precision, scale)`.
    #[staticmethod]
    #[pyo3(signature = (precision, scale = 0))]
    fn decimal(precision: u8, scale: i8) -> Self {
        wrap(CoreDataType::decimal(precision, scale))
    }
    /// A calendar date.
    #[staticmethod]
    fn date() -> Self {
        wrap(CoreDataType::date())
    }
    /// A time of day in the given unit (``"s"`` / ``"ms"`` / ``"us"`` / ``"ns"``).
    #[staticmethod]
    fn time(unit: &str) -> PyResult<Self> {
        Ok(wrap(CoreDataType::time(time_unit_from(unit)?)))
    }
    /// A timestamp in the given unit, optionally zoned (a zone name).
    #[staticmethod]
    #[pyo3(signature = (unit, timezone = None))]
    fn timestamp(unit: &str, timezone: Option<&str>) -> PyResult<Self> {
        let tz = match timezone {
            Some(name) => Some(CoreTimezone::from_str(name).map_err(time_err)?),
            None => None,
        };
        Ok(wrap(CoreDataType::timestamp(time_unit_from(unit)?, tz)))
    }
    /// An elapsed duration in the given unit.
    #[staticmethod]
    fn duration(unit: &str) -> PyResult<Self> {
        Ok(wrap(CoreDataType::duration(time_unit_from(unit)?)))
    }
    /// A calendar interval (``"year_month"`` / ``"day_time"`` / ``"month_day_nano"``).
    #[staticmethod]
    fn interval(unit: &str) -> PyResult<Self> {
        let unit = IntervalUnit::from_name(unit)
            .ok_or_else(|| PyValueError::new_err(format!("unknown interval unit '{unit}'")))?;
        Ok(wrap(CoreDataType::interval(unit)))
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

    // ---- nested constructors ----

    /// A list of the item :class:`Field`.
    #[staticmethod]
    fn list(item: &Field) -> Self {
        wrap(CoreDataType::list(item.inner.clone()))
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
    fn map(key: &DataType, value: &DataType) -> Self {
        wrap(CoreDataType::map(key.inner.clone(), value.inner.clone()))
    }
    /// A union of the given alternative :class:`Field` list.
    #[staticmethod]
    fn union(fields: Vec<Field>) -> Self {
        wrap(CoreDataType::union(
            fields.into_iter().map(|f| f.inner).collect(),
        ))
    }
    /// A dictionary of `key` indices into `value`s.
    #[staticmethod]
    fn dictionary(key: &DataType, value: &DataType) -> Self {
        wrap(CoreDataType::dictionary(
            key.inner.clone(),
            value.inner.clone(),
        ))
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

    /// The stable ``u8`` type id.
    #[getter]
    fn type_id(&self) -> u8 {
        self.inner.type_id().as_u8()
    }
    /// The canonical name (``"int32"`` / ``"decimal"`` / ``"list"`` / …).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    /// The category: ``"primitive"`` / ``"logical"`` / ``"nested"``.
    #[getter]
    fn category(&self) -> &'static str {
        self.inner.category().name()
    }
    /// The ``(precision, scale)`` of a decimal type, else ``None``.
    #[getter]
    fn decimal_parts(&self) -> Option<(u8, i8)> {
        self.inner.decimal_parts()
    }
    /// The child :class:`Field` list of a nested type (empty otherwise).
    fn fields(&self) -> Vec<Field> {
        self.inner
            .fields()
            .iter()
            .map(|f| Field { inner: f.clone() })
            .collect()
    }

    fn is_primitive(&self) -> bool {
        self.inner.is_primitive()
    }
    fn is_logical(&self) -> bool {
        self.inner.is_logical()
    }
    fn is_nested(&self) -> bool {
        self.inner.is_nested()
    }

    // ---- dunders ----

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __str__(&self) -> &'static str {
        self.inner.name()
    }

    fn __repr__(&self) -> String {
        format!("DataType.{}", self.inner.name())
    }
}
