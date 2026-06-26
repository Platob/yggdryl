//! The [`DataType`] enum — the logical type of a column — and its
//! [`DataTypeError`]. Mirrors `arrow_schema::DataType` exactly, split (as Arrow's
//! own types are) into the [`PrimitiveType`], [`LogicalType`] and [`NestedType`]
//! families. **All `DataType` logic lives here**; each family owns its own
//! module.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::parse::split_head;

mod logical;
mod nested;
mod primitive;

pub use logical::{IntervalUnit, LogicalType, TimeUnit};
pub use nested::{NestedType, UnionMode};
pub use primitive::PrimitiveType;

/// Error returned when a [`DataType`] (or one of its families) cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataTypeError {
    /// The input was empty.
    Empty,
    /// The leading name matched no known type in any family.
    Unknown(String),
    /// The name was recognised but its parameters or body were malformed.
    Invalid(String),
}

impl fmt::Display for DataTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataTypeError::Empty => write!(f, "data type is empty"),
            DataTypeError::Unknown(name) => write!(f, "unknown data type '{name}'"),
            DataTypeError::Invalid(detail) => write!(f, "invalid data type: {detail}"),
        }
    }
}

impl std::error::Error for DataTypeError {}

/// A child [`Field`](crate::Field) that fails to parse makes the *enclosing*
/// nested type [`Invalid`](DataTypeError::Invalid) — never
/// [`Unknown`](DataTypeError::Unknown), which is reserved for an unrecognised
/// leading name (so the family-routing in [`DataType::from_str`] stays correct).
impl From<crate::FieldError> for DataTypeError {
    fn from(err: crate::FieldError) -> DataTypeError {
        DataTypeError::Invalid(err.to_string())
    }
}

/// The logical type of a column, partitioned into the three Arrow families:
/// [`PrimitiveType`] (flat scalars), [`LogicalType`] (semantic types over a
/// physical layout) and [`NestedType`] (types carrying child
/// [`Field`](crate::Field)s).
///
/// The partition is total and disjoint, so under the `arrow` feature
/// [`to_arrow`](DataType::to_arrow) / [`from_arrow`](DataType::from_arrow) form a
/// lossless bijection with `arrow_schema::DataType`.
///
/// ```
/// use yggdryl_saga::{DataType, PrimitiveType};
///
/// let dt = DataType::from(PrimitiveType::Int64);
/// assert!(dt.is_primitive());
/// assert_eq!(dt.to_str(), "int64");
///
/// // Nested types round-trip through their canonical string.
/// let nested = DataType::from_str("map<entries: struct<key: utf8 not null, value: int64>>").unwrap();
/// assert!(nested.is_nested());
/// assert_eq!(DataType::from_str(&nested.to_str()).unwrap(), nested);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataType {
    /// A flat, child-less scalar.
    Primitive(PrimitiveType),
    /// A semantic type over a physical layout (temporal, interval, decimal).
    Logical(LogicalType),
    /// A type carrying child [`Field`](crate::Field)s.
    Nested(NestedType),
}

impl From<PrimitiveType> for DataType {
    fn from(p: PrimitiveType) -> DataType {
        DataType::Primitive(p)
    }
}

impl From<LogicalType> for DataType {
    fn from(l: LogicalType) -> DataType {
        DataType::Logical(l)
    }
}

impl From<NestedType> for DataType {
    fn from(n: NestedType) -> DataType {
        DataType::Nested(n)
    }
}

impl DataType {
    /// `true` if this is a [`PrimitiveType`].
    pub fn is_primitive(&self) -> bool {
        matches!(self, DataType::Primitive(_))
    }

    /// `true` if this is a [`LogicalType`].
    pub fn is_logical(&self) -> bool {
        matches!(self, DataType::Logical(_))
    }

    /// `true` if this is a [`NestedType`].
    pub fn is_nested(&self) -> bool {
        matches!(self, DataType::Nested(_))
    }

    /// `true` for the numeric primitives (see [`PrimitiveType::is_numeric`]).
    pub fn is_numeric(&self) -> bool {
        matches!(self, DataType::Primitive(p) if p.is_numeric())
    }

    /// Parses any canonical type string, trying the primitive, logical and nested
    /// families in turn (e.g. `int64`, `timestamp(us, UTC)`, `list<item: int64>`).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<DataType, DataTypeError> {
        log_event!(trace, "DataType::from_str {input:?}");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(DataTypeError::Empty);
        }
        let head =
            split_head(trimmed).ok_or_else(|| DataTypeError::Invalid(trimmed.to_string()))?;

        // Each family claims its own names: an `Unknown` means "not mine, try the
        // next"; any other error means the name matched but the input was bad.
        match PrimitiveType::from_head(&head) {
            Err(DataTypeError::Unknown(_)) => {}
            other => return other.map(DataType::Primitive),
        }
        match LogicalType::from_head(&head) {
            Err(DataTypeError::Unknown(_)) => {}
            other => return other.map(DataType::Logical),
        }
        match NestedType::from_head(&head) {
            Err(DataTypeError::Unknown(_)) => {}
            other => return other.map(DataType::Nested),
        }
        Err(DataTypeError::Unknown(head.name.to_string()))
    }

    /// Renders the canonical type string — the inverse of
    /// [`from_str`](DataType::from_str).
    pub fn to_str(&self) -> String {
        match self {
            DataType::Primitive(p) => p.to_str(),
            DataType::Logical(l) => l.to_str(),
            DataType::Nested(n) => n.to_str(),
        }
    }

    /// Converts to the matching `arrow_schema::DataType` (infallible).
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::DataType {
        self.into()
    }

    /// Builds a [`DataType`] from an `arrow_schema::DataType` (infallible — every
    /// Arrow type maps to exactly one family).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(dt: &arrow_schema::DataType) -> DataType {
        dt.into()
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

/// Conversion to `arrow_schema::DataType`, delegating to each family.
#[cfg(feature = "arrow")]
impl From<&DataType> for arrow_schema::DataType {
    fn from(d: &DataType) -> arrow_schema::DataType {
        match d {
            DataType::Primitive(p) => p.into(),
            DataType::Logical(l) => l.into(),
            DataType::Nested(n) => n.into(),
        }
    }
}

/// Conversion from `arrow_schema::DataType` — a total match over every Arrow
/// variant, sorting each into its family.
#[cfg(feature = "arrow")]
impl From<&arrow_schema::DataType> for DataType {
    fn from(a: &arrow_schema::DataType) -> DataType {
        use arrow_schema::DataType as A;
        match a {
            A::Null => PrimitiveType::Null.into(),
            A::Boolean => PrimitiveType::Boolean.into(),
            A::Int8 => PrimitiveType::Int8.into(),
            A::Int16 => PrimitiveType::Int16.into(),
            A::Int32 => PrimitiveType::Int32.into(),
            A::Int64 => PrimitiveType::Int64.into(),
            A::UInt8 => PrimitiveType::UInt8.into(),
            A::UInt16 => PrimitiveType::UInt16.into(),
            A::UInt32 => PrimitiveType::UInt32.into(),
            A::UInt64 => PrimitiveType::UInt64.into(),
            A::Float16 => PrimitiveType::Float16.into(),
            A::Float32 => PrimitiveType::Float32.into(),
            A::Float64 => PrimitiveType::Float64.into(),
            A::Binary => PrimitiveType::Binary.into(),
            A::LargeBinary => PrimitiveType::LargeBinary.into(),
            A::BinaryView => PrimitiveType::BinaryView.into(),
            A::FixedSizeBinary(n) => PrimitiveType::FixedSizeBinary(*n).into(),
            A::Utf8 => PrimitiveType::Utf8.into(),
            A::LargeUtf8 => PrimitiveType::LargeUtf8.into(),
            A::Utf8View => PrimitiveType::Utf8View.into(),
            A::Date32 => LogicalType::Date32.into(),
            A::Date64 => LogicalType::Date64.into(),
            A::Time32(u) => LogicalType::Time32((*u).into()).into(),
            A::Time64(u) => LogicalType::Time64((*u).into()).into(),
            A::Timestamp(u, tz) => {
                LogicalType::Timestamp((*u).into(), tz.as_ref().map(|s| s.to_string())).into()
            }
            A::Duration(u) => LogicalType::Duration((*u).into()).into(),
            A::Interval(u) => LogicalType::Interval((*u).into()).into(),
            A::Decimal32(p, s) => LogicalType::Decimal32(*p, *s).into(),
            A::Decimal64(p, s) => LogicalType::Decimal64(*p, *s).into(),
            A::Decimal128(p, s) => LogicalType::Decimal128(*p, *s).into(),
            A::Decimal256(p, s) => LogicalType::Decimal256(*p, *s).into(),
            A::List(f) => NestedType::List(Box::new(f.as_ref().into())).into(),
            A::LargeList(f) => NestedType::LargeList(Box::new(f.as_ref().into())).into(),
            A::ListView(f) => NestedType::ListView(Box::new(f.as_ref().into())).into(),
            A::LargeListView(f) => NestedType::LargeListView(Box::new(f.as_ref().into())).into(),
            A::FixedSizeList(f, n) => {
                NestedType::FixedSizeList(Box::new(f.as_ref().into()), *n).into()
            }
            A::Struct(fields) => {
                NestedType::Struct(fields.iter().map(|f| f.as_ref().into()).collect()).into()
            }
            A::Map(f, sorted) => NestedType::Map(Box::new(f.as_ref().into()), *sorted).into(),
            A::Union(fields, mode) => NestedType::Union(
                fields.iter().map(|(_, f)| f.as_ref().into()).collect(),
                (*mode).into(),
            )
            .into(),
            A::Dictionary(key, value) => NestedType::Dictionary(
                Box::new(key.as_ref().into()),
                Box::new(value.as_ref().into()),
            )
            .into(),
            A::RunEndEncoded(run_ends, values) => NestedType::RunEndEncoded(
                Box::new(run_ends.as_ref().into()),
                Box::new(values.as_ref().into()),
            )
            .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_round_trip() {
        for (s, dt) in [
            ("null", DataType::from(PrimitiveType::Null)),
            ("boolean", PrimitiveType::Boolean.into()),
            ("int64", PrimitiveType::Int64.into()),
            ("uint8", PrimitiveType::UInt8.into()),
            ("float64", PrimitiveType::Float64.into()),
            ("utf8", PrimitiveType::Utf8.into()),
            ("large_binary", PrimitiveType::LargeBinary.into()),
            (
                "fixed_size_binary(16)",
                PrimitiveType::FixedSizeBinary(16).into(),
            ),
        ] {
            assert_eq!(DataType::from_str(s).unwrap(), dt, "{s}");
            assert_eq!(dt.to_str(), s, "{s}");
        }
        // Aliases resolve to the canonical variant.
        assert_eq!(DataType::from_str("bool").unwrap().to_str(), "boolean");
        assert_eq!(DataType::from_str("string").unwrap().to_str(), "utf8");
        assert_eq!(DataType::from_str("double").unwrap().to_str(), "float64");
    }

    #[test]
    fn logical_round_trips() {
        for s in [
            "date32",
            "date64",
            "time32(ms)",
            "time64(us)",
            "timestamp(ns)",
            "timestamp(us, UTC)",
            "timestamp(s, America/New_York)",
            "duration(ns)",
            "interval(month_day_nano)",
            "decimal32(9, 2)",
            "decimal64(18, 4)",
            "decimal128(38, 10)",
            "decimal256(76, 0)",
        ] {
            let dt = DataType::from_str(s).unwrap();
            assert!(dt.is_logical(), "{s}");
            assert_eq!(dt.to_str(), s, "{s}");
        }
    }

    #[test]
    fn nested_round_trips() {
        for s in [
            "list<item: int64>",
            "large_list<item: utf8 not null>",
            "list_view<item: float64>",
            "fixed_size_list(3)<item: float32>",
            "struct<>",
            "struct<id: int64, name: utf8 not null>",
            "map<entries: struct<key: utf8 not null, value: int64>>",
            "map(sorted)<entries: struct<key: int32 not null, value: utf8>>",
            "union(sparse)<a: int64, b: utf8>",
            "union(dense)<a: int64>",
            "dictionary<int32, utf8>",
            "run_end_encoded<run_ends: int32 not null, values: utf8>",
        ] {
            let dt = DataType::from_str(s).unwrap();
            assert!(dt.is_nested(), "{s}");
            assert_eq!(dt.to_str(), s, "{s}");
            // Re-parsing the rendered form is idempotent.
            assert_eq!(DataType::from_str(&dt.to_str()).unwrap(), dt, "{s}");
        }
    }

    #[test]
    fn deeply_nested_round_trips() {
        let s = "list<item: struct<ts: timestamp(ns, UTC) not null, tags: map<entries: struct<key: utf8 not null, value: list<item: int64>>>>>";
        let dt = DataType::from_str(s).unwrap();
        assert_eq!(dt.to_str(), s);
    }

    #[test]
    fn errors_are_actionable() {
        assert_eq!(DataType::from_str(""), Err(DataTypeError::Empty));
        assert_eq!(
            DataType::from_str("notatype"),
            Err(DataTypeError::Unknown("notatype".to_string()))
        );
        // Recognised name, bad parameters → Invalid (not Unknown).
        assert!(matches!(
            DataType::from_str("decimal128(38)"),
            Err(DataTypeError::Invalid(_))
        ));
        assert!(matches!(
            DataType::from_str("timestamp(weeks)"),
            Err(DataTypeError::Invalid(_))
        ));
        assert!(matches!(
            DataType::from_str("list<int64"),
            Err(DataTypeError::Invalid(_))
        ));
    }

    #[test]
    fn predicates() {
        assert!(DataType::from_str("int32").unwrap().is_numeric());
        assert!(DataType::from_str("float64").unwrap().is_numeric());
        assert!(!DataType::from_str("utf8").unwrap().is_numeric());
        assert!(DataType::from_str("date32").unwrap().is_logical());
        assert!(DataType::from_str("struct<a: int64>").unwrap().is_nested());
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn arrow_bijection() {
        use arrow_schema::{DataType as A, Field as AField, TimeUnit as AUnit};
        use std::sync::Arc;

        let cases = [
            A::Null,
            A::Int64,
            A::Float64,
            A::Utf8,
            A::FixedSizeBinary(16),
            A::Timestamp(AUnit::Nanosecond, Some("UTC".into())),
            A::Date32,
            A::Decimal128(38, 10),
            A::Decimal32(9, 2),
            A::List(Arc::new(AField::new("item", A::Int64, true))),
            A::Struct(
                vec![
                    AField::new("id", A::Int64, false),
                    AField::new("name", A::Utf8, true),
                ]
                .into(),
            ),
            A::Dictionary(Box::new(A::Int32), Box::new(A::Utf8)),
        ];
        for a in cases {
            let ours = DataType::from_arrow(&a);
            assert_eq!(ours.to_arrow(), a, "arrow round-trip for {a:?}");
        }
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn arrow_string_and_back() {
        use crate::Field;
        // Our string parser and the Arrow bridge agree on a non-trivial schema.
        let dt = DataType::from_str("struct<id: int64 not null, tags: list<item: utf8>>").unwrap();
        let arrow = dt.to_arrow();
        assert_eq!(DataType::from_arrow(&arrow), dt);
        // The struct's first child is non-nullable, the list child nullable.
        if let arrow_schema::DataType::Struct(fields) = &arrow {
            assert!(!fields[0].is_nullable());
        } else {
            panic!("expected a struct");
        }
        let _ = Field::new("x", dt, true); // Field is reachable from here.
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trips_structurally() {
        let dt = DataType::from_str("struct<id: int64 not null, t: timestamp(us, UTC)>").unwrap();
        let json = serde_json::to_string(&dt).unwrap();
        assert_eq!(serde_json::from_str::<DataType>(&json).unwrap(), dt);
    }
}
