//! The `DataType` napi class — the simplified, Arrow-compatible logical type.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{TimeUnit, Timezone as CoreTimezone};
use yggdryl_schema::{Charset, DataType as CoreDataType, IntervalUnit, MergeStrategy, UnionMode};

use crate::field::Field;
use crate::timezone::Timezone;
use crate::{err, to_mapping};

/// A logical data type (primitive / logical / nested, plus the `any` wildcard).
#[napi]
pub struct DataType {
    pub(crate) inner: CoreDataType,
}

fn wrap(inner: CoreDataType) -> DataType {
    DataType { inner }
}

#[napi]
impl DataType {
    /// Parse a canonical type string (e.g. `"int64"`, `"timestamp[us, UTC]"`).
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreDataType::from_str(&value).map(wrap).map_err(err)
    }

    /// Alias for the constructor.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        DataType::new(value)
    }

    // ---- constructors ----

    /// The `any` wildcard.
    #[napi(factory)]
    pub fn any() -> Self {
        wrap(CoreDataType::Any)
    }

    /// The null type.
    #[napi(factory, js_name = "null")]
    pub fn null_() -> Self {
        wrap(CoreDataType::Null)
    }

    /// The boolean type.
    #[napi(factory)]
    pub fn boolean() -> Self {
        wrap(CoreDataType::Boolean)
    }

    /// An integer of `bits` width (commonly 8/16/32/64, but any width is allowed;
    /// default 64), signed by default.
    #[napi(factory)]
    pub fn int(bits: Option<u16>, signed: Option<bool>) -> Self {
        wrap(CoreDataType::int(
            bits.unwrap_or(64),
            signed.unwrap_or(true),
        ))
    }

    /// A generic signed integer at the default width (`int64`).
    #[napi(factory)]
    pub fn integer() -> Self {
        wrap(CoreDataType::integer())
    }

    /// An integer type wide enough to hold a byte buffer/view (width =
    /// `data.length * 8` bits; empty → default `int64`), signed by default.
    #[napi(factory, js_name = "intFromBytes")]
    pub fn int_from_bytes(data: Uint8Array, signed: Option<bool>) -> Self {
        wrap(CoreDataType::int_from_bytes(&data, signed.unwrap_or(true)))
    }

    /// A floating-point type of `bits` width (commonly 16/32/64, but any width is
    /// allowed; default 64).
    #[napi(factory)]
    pub fn float(bits: Option<u16>) -> Self {
        wrap(CoreDataType::float(bits.unwrap_or(64)))
    }

    /// A float at the default width (`float64`).
    #[napi(factory)]
    pub fn floating() -> Self {
        wrap(CoreDataType::floating())
    }

    /// A float type wide enough to hold a byte buffer/view (2 → `float16`, 4 →
    /// `float32`, 8 → `float64`; empty → default `float64`).
    #[napi(factory, js_name = "floatFromBytes")]
    pub fn float_from_bytes(data: Uint8Array) -> Self {
        wrap(CoreDataType::float_from_bytes(&data))
    }

    /// A decimal with `(precision, scale)`, stored in `bits` (default 128).
    #[napi(factory)]
    pub fn decimal(precision: u8, scale: Option<i32>, bits: Option<u16>) -> Result<Self> {
        // Validate the scale fits i8 rather than silently wrapping (Python raises too).
        let scale = scale.unwrap_or(0);
        let scale = i8::try_from(scale)
            .map_err(|_| err("decimal scale out of range, expected -128..=127"))?;
        Ok(wrap(CoreDataType::decimal_with(
            precision,
            scale,
            bits.unwrap_or(128),
        )))
    }

    /// A string with the given charset, large/view flags and optional fixed `size`
    /// (omitted = variable-length).
    #[napi(factory)]
    pub fn varchar(
        charset: Option<String>,
        large: Option<bool>,
        view: Option<bool>,
        size: Option<i32>,
    ) -> Result<Self> {
        let charset = Charset::from_str(charset.as_deref().unwrap_or("utf8")).map_err(err)?;
        Ok(wrap(CoreDataType::varchar_with(
            charset,
            large.unwrap_or(false),
            view.unwrap_or(false),
            size,
        )))
    }

    /// A fixed-length UTF-8 string of `size` characters (SQL `char(n)`).
    #[napi(factory, js_name = "fixedSizeVarchar")]
    pub fn fixed_size_varchar(size: i32) -> Self {
        wrap(CoreDataType::fixed_size_varchar(size))
    }

    /// Variable-length opaque bytes.
    #[napi(factory)]
    pub fn binary(large: Option<bool>, view: Option<bool>) -> Self {
        wrap(CoreDataType::Binary {
            large: large.unwrap_or(false),
            view: view.unwrap_or(false),
            size: None,
        })
    }

    /// Fixed-width opaque bytes of `size` bytes.
    #[napi(factory, js_name = "fixedSizeBinary")]
    pub fn fixed_size_binary(size: i32) -> Self {
        wrap(CoreDataType::fixed_size_binary(size))
    }

    /// A calendar date (`large` selects millisecond/64-bit over day/32-bit).
    #[napi(factory)]
    pub fn date(large: Option<bool>) -> Self {
        wrap(CoreDataType::Date {
            large: large.unwrap_or(false),
        })
    }

    /// A time of day in the given unit.
    #[napi(factory)]
    pub fn time(unit: String) -> Result<Self> {
        Ok(wrap(CoreDataType::Time {
            unit: TimeUnit::from_str(&unit).map_err(err)?,
        }))
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

    /// Elapsed time in the given unit.
    #[napi(factory)]
    pub fn duration(unit: String) -> Result<Self> {
        Ok(wrap(CoreDataType::Duration {
            unit: TimeUnit::from_str(&unit).map_err(err)?,
        }))
    }

    /// A calendar interval (`"year_month"` / `"day_time"` / `"month_day_nano"`).
    #[napi(factory)]
    pub fn interval(unit: String) -> Result<Self> {
        Ok(wrap(CoreDataType::Interval {
            unit: IntervalUnit::from_str(&unit).map_err(err)?,
        }))
    }

    /// A dictionary of `key` indices into `value`s.
    #[napi(factory)]
    pub fn dictionary(key: &DataType, value: &DataType) -> Self {
        wrap(CoreDataType::dictionary(
            key.inner.clone(),
            value.inner.clone(),
        ))
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

    /// A variable-length list of the item `Field`.
    #[napi(factory)]
    pub fn list(item: &Field) -> Self {
        wrap(CoreDataType::list(item.inner.clone()))
    }

    /// A 64-bit-offset large list of the item `Field`.
    #[napi(factory, js_name = "largeList")]
    pub fn large_list(item: &Field) -> Self {
        wrap(CoreDataType::large_list(item.inner.clone()))
    }

    /// A fixed-length list of the item `Field`, `size` elements long.
    #[napi(factory, js_name = "fixedSizeList")]
    pub fn fixed_size_list(item: &Field, size: i32) -> Self {
        wrap(CoreDataType::fixed_size_list(item.inner.clone(), size))
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
    pub fn map(key: &DataType, value: &DataType, sorted: Option<bool>) -> Self {
        wrap(CoreDataType::map(
            key.inner.clone(),
            value.inner.clone(),
            sorted.unwrap_or(false),
        ))
    }

    /// A union of the given alternatives (`mode` is `"sparse"` or `"dense"`).
    #[napi(factory)]
    pub fn union(fields: Vec<&Field>, mode: Option<String>) -> Result<Self> {
        let mode = UnionMode::from_str(mode.as_deref().unwrap_or("sparse")).map_err(err)?;
        Ok(wrap(CoreDataType::union(
            fields.into_iter().map(|f| f.inner.clone()).collect(),
            mode,
        )))
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

    /// The category: `"any"` / `"primitive"` / `"logical"` / `"nested"`.
    #[napi(getter)]
    pub fn category(&self) -> String {
        self.inner.category().as_str().to_string()
    }

    /// The physical width in bits for fixed-width types, else null.
    #[napi(getter, js_name = "bitSize")]
    pub fn bit_size(&self) -> Option<u16> {
        self.inner.bit_size()
    }

    /// The physical width in bytes for byte-aligned fixed-width types, else null.
    #[napi(getter, js_name = "byteSize")]
    pub fn byte_size(&self) -> Option<u16> {
        self.inner.byte_size()
    }

    /// Whether this uses the large form.
    #[napi(getter, js_name = "isLarge")]
    pub fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether this uses the view layout.
    #[napi(getter, js_name = "isView")]
    pub fn is_view(&self) -> bool {
        self.inner.is_view()
    }

    /// Whether this type has a fixed (non-variable) length.
    #[napi(getter, js_name = "isFixedSize")]
    pub fn is_fixed_size(&self) -> bool {
        self.inner.is_fixed_size()
    }

    /// The physical (storage) `DataType` backing a logical type (identity otherwise).
    #[napi(js_name = "physicalType")]
    pub fn physical_type(&self) -> DataType {
        wrap(self.inner.physical_type())
    }

    /// The string charset, if a string type.
    #[napi(getter)]
    pub fn charset(&self) -> Option<String> {
        self.inner.charset().map(|c| c.as_str().to_string())
    }

    /// The time unit of a temporal type carrying one, else null.
    #[napi(getter, js_name = "timeUnit")]
    pub fn time_unit(&self) -> Option<String> {
        self.inner.time_unit().map(|u| u.as_str().to_string())
    }

    /// The display `Timezone` of a timestamp, else null.
    #[napi(getter)]
    pub fn timezone(&self) -> Option<Timezone> {
        self.inner
            .timezone()
            .cloned()
            .map(|inner| Timezone { inner })
    }

    /// The `[precision, scale]` of a decimal type, else null.
    #[napi(getter, js_name = "decimalParts")]
    pub fn decimal_parts(&self) -> Option<Vec<i32>> {
        self.inner
            .decimal_parts()
            .map(|(p, s)| vec![p as i32, s as i32])
    }

    /// The child `Field` list of a nested type.
    #[napi]
    pub fn children(&self) -> Vec<Field> {
        self.inner
            .children()
            .into_iter()
            .map(|f| Field { inner: f.clone() })
            .collect()
    }

    // ---- predicates ----
    #[napi(js_name = "isAny")]
    pub fn is_any(&self) -> bool {
        self.inner.is_any()
    }
    #[napi(js_name = "isNull")]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }
    #[napi(js_name = "isBoolean")]
    pub fn is_boolean(&self) -> bool {
        self.inner.is_boolean()
    }
    #[napi(js_name = "isInteger")]
    pub fn is_integer(&self) -> bool {
        self.inner.is_integer()
    }
    #[napi(js_name = "isSignedInteger")]
    pub fn is_signed_integer(&self) -> bool {
        self.inner.is_signed_integer()
    }
    #[napi(js_name = "isUnsignedInteger")]
    pub fn is_unsigned_integer(&self) -> bool {
        self.inner.is_unsigned_integer()
    }
    #[napi(js_name = "isFloating")]
    pub fn is_floating(&self) -> bool {
        self.inner.is_floating()
    }
    #[napi(js_name = "isNumeric")]
    pub fn is_numeric(&self) -> bool {
        self.inner.is_numeric()
    }
    #[napi(js_name = "isString")]
    pub fn is_string(&self) -> bool {
        self.inner.is_string()
    }
    #[napi(js_name = "isBinary")]
    pub fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }
    #[napi(js_name = "isPrimitive")]
    pub fn is_primitive(&self) -> bool {
        self.inner.is_primitive()
    }
    #[napi(js_name = "isLogical")]
    pub fn is_logical(&self) -> bool {
        self.inner.is_logical()
    }
    #[napi(js_name = "isTemporal")]
    pub fn is_temporal(&self) -> bool {
        self.inner.is_temporal()
    }
    #[napi(js_name = "isDecimal")]
    pub fn is_decimal(&self) -> bool {
        self.inner.is_decimal()
    }
    #[napi(js_name = "isDictionary")]
    pub fn is_dictionary(&self) -> bool {
        self.inner.is_dictionary()
    }
    #[napi(js_name = "isJson")]
    pub fn is_json(&self) -> bool {
        self.inner.is_json()
    }
    #[napi(js_name = "isBson")]
    pub fn is_bson(&self) -> bool {
        self.inner.is_bson()
    }
    #[napi(js_name = "isNested")]
    pub fn is_nested(&self) -> bool {
        self.inner.is_nested()
    }
    #[napi(js_name = "isList")]
    pub fn is_list(&self) -> bool {
        self.inner.is_list()
    }
    #[napi(js_name = "isStruct")]
    pub fn is_struct(&self) -> bool {
        self.inner.is_struct()
    }
    #[napi(js_name = "isUnion")]
    pub fn is_union(&self) -> bool {
        self.inner.is_union()
    }
    #[napi(js_name = "isMap")]
    pub fn is_map(&self) -> bool {
        self.inner.is_map()
    }

    // ---- conversion / merge ----

    /// Whether a value of this type can be cast to `other`.
    #[napi(js_name = "canCastTo")]
    pub fn can_cast_to(&self, other: &DataType) -> bool {
        self.inner.can_cast_to(&other.inner)
    }

    /// The least type both can widen to, or null.
    #[napi(js_name = "commonType")]
    pub fn common_type(&self, other: &DataType) -> Option<DataType> {
        self.inner.common_type(&other.inner).map(wrap)
    }

    /// Merge with `other` under a strategy (`"strict"` / `"promote"` / `"permissive"`).
    #[napi]
    pub fn merge(&self, other: &DataType, strategy: Option<String>) -> Result<DataType> {
        let strategy =
            MergeStrategy::from_str(strategy.as_deref().unwrap_or("promote")).map_err(err)?;
        self.inner
            .merge(&other.inner, strategy)
            .map(wrap)
            .map_err(err)
    }

    // ---- serialisation ----

    /// Render to an object (the single `type` key).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Build from an object (the `type` key).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreDataType::from_mapping(&to_mapping(fields))
            .map(wrap)
            .map_err(err)
    }

    /// The canonical string as bytes.
    #[napi(js_name = "toBytes")]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// `true` if the two types are equal.
    #[napi]
    pub fn equals(&self, other: &DataType) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to a lossless structural JSON string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Parse from the structural JSON of `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        CoreDataType::from_json(&value).map(wrap).map_err(err)
    }
}
