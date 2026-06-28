//! The central [`DataType`] — a value's logical type, split into three category types
//! ([`PrimitiveType`] / [`LogicalType`] / [`NestedType`]), each tagged by a stable
//! [`DataTypeId`] (`u8`).

mod id;
mod logical;
mod nested;
mod primitive;

pub use id::{DataTypeId, TypeCategory};
pub use logical::{IntervalUnit, LogicalType};
pub use nested::NestedType;
pub use primitive::PrimitiveType;

use crate::Field;
use yggdryl_core::{TimeUnit, Timezone};

/// A logical data type. It is exactly one of the three [categories](TypeCategory) —
/// [primitive](PrimitiveType), [logical](LogicalType) or [nested](NestedType) — and
/// every type carries a stable [`type_id`](DataType::type_id) (`u8`) and a
/// [`name`](DataType::name).
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, TypeCategory};
/// assert_eq!(DataType::int32().type_id(), DataTypeId::Int32);
/// assert_eq!(DataType::int32().name(), "int32");
/// assert_eq!(DataType::decimal(10, 2).category(), TypeCategory::Logical);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataType {
    /// A scalar [primitive](PrimitiveType) (null, boolean, integers, floats, string, bytes).
    Primitive(PrimitiveType),
    /// A [logical](LogicalType) type (decimal, temporal, JSON/BSON).
    Logical(LogicalType),
    /// A [nested](NestedType) container (list, struct, map, union, …).
    Nested(NestedType),
}

impl DataType {
    // ---- the two universal accessors ----

    /// The stable `u8` [`DataTypeId`] of this type.
    pub fn type_id(&self) -> DataTypeId {
        match self {
            DataType::Primitive(t) => t.type_id(),
            DataType::Logical(t) => t.type_id(),
            DataType::Nested(t) => t.type_id(),
        }
    }

    /// The canonical lowercase name (`"int32"`, `"decimal"`, `"list"`, …).
    pub fn name(&self) -> &'static str {
        self.type_id().name()
    }

    /// The [`TypeCategory`] this type falls under.
    pub fn category(&self) -> TypeCategory {
        match self {
            DataType::Primitive(_) => TypeCategory::Primitive,
            DataType::Logical(_) => TypeCategory::Logical,
            DataType::Nested(_) => TypeCategory::Nested,
        }
    }

    // ---- category access ----

    /// The [`PrimitiveType`] if this is a primitive, else `None`.
    pub fn as_primitive(&self) -> Option<PrimitiveType> {
        match self {
            DataType::Primitive(t) => Some(*t),
            _ => None,
        }
    }

    /// The [`LogicalType`] if this is a logical type, else `None`.
    pub fn as_logical(&self) -> Option<&LogicalType> {
        match self {
            DataType::Logical(t) => Some(t),
            _ => None,
        }
    }

    /// The [`NestedType`] if this is a nested type, else `None`.
    pub fn as_nested(&self) -> Option<&NestedType> {
        match self {
            DataType::Nested(t) => Some(t),
            _ => None,
        }
    }

    /// Whether this is a [primitive](PrimitiveType) scalar.
    pub fn is_primitive(&self) -> bool {
        matches!(self, DataType::Primitive(_))
    }

    /// Whether this is a [logical](LogicalType) type.
    pub fn is_logical(&self) -> bool {
        matches!(self, DataType::Logical(_))
    }

    /// Whether this is a [nested](NestedType) container.
    pub fn is_nested(&self) -> bool {
        matches!(self, DataType::Nested(_))
    }

    // ---- primitive constructors ----

    /// The null type.
    pub fn null() -> DataType {
        DataType::Primitive(PrimitiveType::Null)
    }
    /// The boolean type.
    pub fn boolean() -> DataType {
        DataType::Primitive(PrimitiveType::Boolean)
    }
    /// A signed 8-bit integer.
    pub fn int8() -> DataType {
        DataType::Primitive(PrimitiveType::Int8)
    }
    /// A signed 16-bit integer.
    pub fn int16() -> DataType {
        DataType::Primitive(PrimitiveType::Int16)
    }
    /// A signed 32-bit integer.
    pub fn int32() -> DataType {
        DataType::Primitive(PrimitiveType::Int32)
    }
    /// A signed 64-bit integer.
    pub fn int64() -> DataType {
        DataType::Primitive(PrimitiveType::Int64)
    }
    /// An unsigned 8-bit integer.
    pub fn uint8() -> DataType {
        DataType::Primitive(PrimitiveType::UInt8)
    }
    /// An unsigned 16-bit integer.
    pub fn uint16() -> DataType {
        DataType::Primitive(PrimitiveType::UInt16)
    }
    /// An unsigned 32-bit integer.
    pub fn uint32() -> DataType {
        DataType::Primitive(PrimitiveType::UInt32)
    }
    /// An unsigned 64-bit integer.
    pub fn uint64() -> DataType {
        DataType::Primitive(PrimitiveType::UInt64)
    }
    /// A half-precision (16-bit) float.
    pub fn float16() -> DataType {
        DataType::Primitive(PrimitiveType::Float16)
    }
    /// A single-precision (32-bit) float.
    pub fn float32() -> DataType {
        DataType::Primitive(PrimitiveType::Float32)
    }
    /// A double-precision (64-bit) float.
    pub fn float64() -> DataType {
        DataType::Primitive(PrimitiveType::Float64)
    }
    /// A UTF-8 string.
    pub fn utf8() -> DataType {
        DataType::Primitive(PrimitiveType::Utf8)
    }
    /// Opaque bytes.
    pub fn binary() -> DataType {
        DataType::Primitive(PrimitiveType::Binary)
    }

    // ---- logical constructors ----

    /// A decimal with `(precision, scale)`.
    pub fn decimal(precision: u8, scale: i8) -> DataType {
        DataType::Logical(LogicalType::Decimal { precision, scale })
    }
    /// A calendar date.
    pub fn date() -> DataType {
        DataType::Logical(LogicalType::Date)
    }
    /// A time of day in the given resolution.
    pub fn time(unit: TimeUnit) -> DataType {
        DataType::Logical(LogicalType::Time { unit })
    }
    /// A timestamp, optionally zoned.
    pub fn timestamp(unit: TimeUnit, timezone: Option<Timezone>) -> DataType {
        DataType::Logical(LogicalType::Timestamp { unit, timezone })
    }
    /// An elapsed duration in the given resolution.
    pub fn duration(unit: TimeUnit) -> DataType {
        DataType::Logical(LogicalType::Duration { unit })
    }
    /// A calendar interval in the given resolution.
    pub fn interval(unit: IntervalUnit) -> DataType {
        DataType::Logical(LogicalType::Interval { unit })
    }
    /// JSON text (string-backed).
    pub fn json() -> DataType {
        DataType::Logical(LogicalType::Json)
    }
    /// A BSON document (binary-backed).
    pub fn bson() -> DataType {
        DataType::Logical(LogicalType::Bson)
    }

    /// The `(precision, scale)` of a decimal type, else `None`.
    pub fn decimal_parts(&self) -> Option<(u8, i8)> {
        match self {
            DataType::Logical(LogicalType::Decimal { precision, scale }) => {
                Some((*precision, *scale))
            }
            _ => None,
        }
    }

    // ---- nested constructors ----

    /// A list of the given element [`Field`].
    pub fn list(item: Field) -> DataType {
        DataType::Nested(NestedType::List(Box::new(item)))
    }
    /// A struct of the given [`Field`]s.
    pub fn struct_(fields: Vec<Field>) -> DataType {
        DataType::Nested(NestedType::Struct(fields))
    }
    /// A map from `key` to `value`.
    pub fn map(key: DataType, value: DataType) -> DataType {
        DataType::Nested(NestedType::Map {
            key: Box::new(key),
            value: Box::new(value),
        })
    }
    /// A union of the given alternatives.
    pub fn union(fields: Vec<Field>) -> DataType {
        DataType::Nested(NestedType::Union(fields))
    }
    /// A dictionary of `key` indices into `value`s.
    pub fn dictionary(key: DataType, value: DataType) -> DataType {
        DataType::Nested(NestedType::Dictionary {
            key: Box::new(key),
            value: Box::new(value),
        })
    }
    /// A run-end-encoded type of `run_ends` (an integer) and `values`.
    pub fn run_end_encoded(run_ends: DataType, values: DataType) -> DataType {
        DataType::Nested(NestedType::RunEndEncoded {
            run_ends: Box::new(run_ends),
            values: Box::new(values),
        })
    }

    /// The immediate child [`Field`]s of a nested type (empty for primitive / logical
    /// types and for the key/value containers).
    pub fn fields(&self) -> &[Field] {
        match self {
            DataType::Nested(t) => t.fields(),
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Field;

    #[test]
    fn ids_categories_and_names() {
        assert_eq!(DataType::int32().type_id(), DataTypeId::Int32);
        assert_eq!(DataType::int32().type_id().as_u8(), 4);
        assert_eq!(DataType::int32().name(), "int32");
        assert_eq!(DataType::int32().category(), TypeCategory::Primitive);
        assert_eq!(DataType::decimal(10, 2).category(), TypeCategory::Logical);
        assert_eq!(DataType::decimal(10, 2).decimal_parts(), Some((10, 2)));
        assert_eq!(DataType::utf8().decimal_parts(), None);
        assert_eq!(
            DataType::list(Field::new("item", DataType::int32())).category(),
            TypeCategory::Nested
        );
        // The id, the category enum and the inner enum agree.
        for dt in [
            DataType::null(),
            DataType::boolean(),
            DataType::uint64(),
            DataType::float16(),
            DataType::utf8(),
            DataType::binary(),
            DataType::date(),
            DataType::json(),
            DataType::struct_(vec![]),
        ] {
            assert_eq!(dt.category(), dt.type_id().category());
            assert_eq!(dt.name(), dt.type_id().name());
        }
    }

    #[test]
    fn category_access_and_predicates() {
        let p = DataType::int8();
        assert!(p.is_primitive() && !p.is_logical() && !p.is_nested());
        assert_eq!(p.as_primitive(), Some(PrimitiveType::Int8));
        assert!(p.as_primitive().unwrap().is_integer());
        assert!(DataType::float64().as_primitive().unwrap().is_float());

        let l = DataType::timestamp(TimeUnit::Microsecond, None);
        assert!(l.is_logical());
        assert!(matches!(
            l.as_logical(),
            Some(LogicalType::Timestamp { .. })
        ));

        let n = DataType::struct_(vec![
            Field::new("a", DataType::int32()),
            Field::new("b", DataType::utf8()),
        ]);
        assert!(n.is_nested());
        assert_eq!(n.fields().len(), 2);
        assert_eq!(DataType::int32().fields().len(), 0);
    }
}
