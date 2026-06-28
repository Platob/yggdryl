//! The `DataType` napi class — a primitive / logical / nested type tagged by a `u8`
//! `DataTypeId`.

use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{TimeUnit, Timezone as CoreTimezone};
use yggdryl_schema::{DataType as CoreDataType, IntervalUnit};

use crate::err;
use crate::field::Field;

/// A logical data type (primitive / logical / nested).
#[napi]
pub struct DataType {
    pub(crate) inner: CoreDataType,
}

fn wrap(inner: CoreDataType) -> DataType {
    DataType { inner }
}

#[napi]
impl DataType {
    // ---- primitive constructors ----

    /// The null type.
    #[napi(factory, js_name = "null")]
    pub fn null_() -> Self {
        wrap(CoreDataType::null())
    }
    /// The boolean type.
    #[napi(factory)]
    pub fn boolean() -> Self {
        wrap(CoreDataType::boolean())
    }
    /// A signed 8-bit integer.
    #[napi(factory)]
    pub fn int8() -> Self {
        wrap(CoreDataType::int8())
    }
    /// A signed 16-bit integer.
    #[napi(factory)]
    pub fn int16() -> Self {
        wrap(CoreDataType::int16())
    }
    /// A signed 32-bit integer.
    #[napi(factory)]
    pub fn int32() -> Self {
        wrap(CoreDataType::int32())
    }
    /// A signed 64-bit integer.
    #[napi(factory)]
    pub fn int64() -> Self {
        wrap(CoreDataType::int64())
    }
    /// An unsigned 8-bit integer.
    #[napi(factory)]
    pub fn uint8() -> Self {
        wrap(CoreDataType::uint8())
    }
    /// An unsigned 16-bit integer.
    #[napi(factory)]
    pub fn uint16() -> Self {
        wrap(CoreDataType::uint16())
    }
    /// An unsigned 32-bit integer.
    #[napi(factory)]
    pub fn uint32() -> Self {
        wrap(CoreDataType::uint32())
    }
    /// An unsigned 64-bit integer.
    #[napi(factory)]
    pub fn uint64() -> Self {
        wrap(CoreDataType::uint64())
    }
    /// A half-precision (16-bit) float.
    #[napi(factory)]
    pub fn float16() -> Self {
        wrap(CoreDataType::float16())
    }
    /// A single-precision (32-bit) float.
    #[napi(factory)]
    pub fn float32() -> Self {
        wrap(CoreDataType::float32())
    }
    /// A double-precision (64-bit) float.
    #[napi(factory)]
    pub fn float64() -> Self {
        wrap(CoreDataType::float64())
    }
    /// A UTF-8 string.
    #[napi(factory)]
    pub fn utf8() -> Self {
        wrap(CoreDataType::utf8())
    }
    /// Opaque bytes.
    #[napi(factory)]
    pub fn binary() -> Self {
        wrap(CoreDataType::binary())
    }

    // ---- logical constructors ----

    /// A decimal with `(precision, scale)`.
    #[napi(factory)]
    pub fn decimal(precision: u8, scale: Option<i32>) -> Result<Self> {
        let scale = i8::try_from(scale.unwrap_or(0))
            .map_err(|_| err("decimal scale out of range, expected -128..=127"))?;
        Ok(wrap(CoreDataType::decimal(precision, scale)))
    }
    /// A calendar date.
    #[napi(factory)]
    pub fn date() -> Self {
        wrap(CoreDataType::date())
    }
    /// A time of day in the given unit.
    #[napi(factory)]
    pub fn time(unit: String) -> Result<Self> {
        Ok(wrap(CoreDataType::time(
            TimeUnit::from_str(&unit).map_err(err)?,
        )))
    }
    /// A timestamp in the given unit, optionally zoned (a zone name).
    #[napi(factory)]
    pub fn timestamp(unit: String, timezone: Option<String>) -> Result<Self> {
        let tz = match timezone {
            Some(name) => Some(CoreTimezone::from_str(&name).map_err(err)?),
            None => None,
        };
        Ok(wrap(CoreDataType::timestamp(
            TimeUnit::from_str(&unit).map_err(err)?,
            tz,
        )))
    }
    /// An elapsed duration in the given unit.
    #[napi(factory)]
    pub fn duration(unit: String) -> Result<Self> {
        Ok(wrap(CoreDataType::duration(
            TimeUnit::from_str(&unit).map_err(err)?,
        )))
    }
    /// A calendar interval (`"year_month"` / `"day_time"` / `"month_day_nano"`).
    #[napi(factory)]
    pub fn interval(unit: String) -> Result<Self> {
        let unit = IntervalUnit::from_name(&unit)
            .ok_or_else(|| err(format!("unknown interval unit '{unit}'")))?;
        Ok(wrap(CoreDataType::interval(unit)))
    }
    /// JSON text (a string-backed logical type).
    #[napi(factory)]
    pub fn json() -> Self {
        wrap(CoreDataType::json())
    }
    /// A BSON document (a binary-backed logical type).
    #[napi(factory)]
    pub fn bson() -> Self {
        wrap(CoreDataType::bson())
    }

    // ---- nested constructors ----

    /// A list of the item `Field`.
    #[napi(factory)]
    pub fn list(item: &Field) -> Self {
        wrap(CoreDataType::list(item.inner.clone()))
    }
    /// A struct of the given `Field` list (a struct type is a schema).
    #[napi(factory, js_name = "struct")]
    pub fn struct_(fields: Vec<&Field>) -> Self {
        wrap(CoreDataType::struct_(
            fields.into_iter().map(|f| f.inner.clone()).collect(),
        ))
    }
    /// A map from `key` to `value`.
    #[napi(factory)]
    pub fn map(key: &DataType, value: &DataType) -> Self {
        wrap(CoreDataType::map(key.inner.clone(), value.inner.clone()))
    }
    /// A union of the given alternative `Field` list.
    #[napi(factory)]
    pub fn union(fields: Vec<&Field>) -> Self {
        wrap(CoreDataType::union(
            fields.into_iter().map(|f| f.inner.clone()).collect(),
        ))
    }
    /// A dictionary of `key` indices into `value`s.
    #[napi(factory)]
    pub fn dictionary(key: &DataType, value: &DataType) -> Self {
        wrap(CoreDataType::dictionary(
            key.inner.clone(),
            value.inner.clone(),
        ))
    }
    /// A run-end-encoded type of `runEnds` (an integer) and `values`.
    #[napi(factory, js_name = "runEndEncoded")]
    pub fn run_end_encoded(run_ends: &DataType, values: &DataType) -> Self {
        wrap(CoreDataType::run_end_encoded(
            run_ends.inner.clone(),
            values.inner.clone(),
        ))
    }

    // ---- accessors ----

    /// The stable `u8` type id.
    #[napi(getter, js_name = "typeId")]
    pub fn type_id(&self) -> u8 {
        self.inner.type_id().as_u8()
    }
    /// The canonical name (`"int32"` / `"decimal"` / `"list"` / …).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }
    /// The category: `"primitive"` / `"logical"` / `"nested"`.
    #[napi(getter)]
    pub fn category(&self) -> String {
        self.inner.category().name().to_string()
    }
    /// The `[precision, scale]` of a decimal type, else null.
    #[napi(getter, js_name = "decimalParts")]
    pub fn decimal_parts(&self) -> Option<Vec<i32>> {
        self.inner
            .decimal_parts()
            .map(|(p, s)| vec![p as i32, s as i32])
    }
    /// The child `Field` list of a nested type (empty otherwise).
    #[napi]
    pub fn fields(&self) -> Vec<Field> {
        self.inner
            .fields()
            .iter()
            .map(|f| Field { inner: f.clone() })
            .collect()
    }

    #[napi(js_name = "isPrimitive")]
    pub fn is_primitive(&self) -> bool {
        self.inner.is_primitive()
    }
    #[napi(js_name = "isLogical")]
    pub fn is_logical(&self) -> bool {
        self.inner.is_logical()
    }
    #[napi(js_name = "isNested")]
    pub fn is_nested(&self) -> bool {
        self.inner.is_nested()
    }

    /// `true` if the two types are equal.
    #[napi]
    pub fn equals(&self, other: &DataType) -> bool {
        self.inner == other.inner
    }

    /// A stable hash of the type.
    #[napi(js_name = "hashCode")]
    pub fn hash_code(&self) -> BigInt {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        BigInt::from(hasher.finish())
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.name().to_string()
    }
}
