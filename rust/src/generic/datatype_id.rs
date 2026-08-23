use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::generic::DataTypeKind;
use crate::{Error, Result};

/// A parameter-free discriminant naming exactly one [`crate::DataType`] variant.
///
/// `DataTypeId` is the stable, copyable identity of a datatype variant, in the
/// spirit of Arrow's type id. It carries no precision, unit, timezone, width,
/// or child fields, so it compares and hashes without touching nested state and
/// is the value bindings use for type names and annotations.
///
/// Use [`DataTypeKind`] through [`Self::kind`] when only the family matters.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DataTypeId {
    /// Null values.
    Null,
    /// Boolean values.
    Boolean,
    /// Signed 8-bit integers.
    Int8,
    /// Signed 16-bit integers.
    Int16,
    /// Signed 32-bit integers.
    Int32,
    /// Signed 64-bit integers.
    Int64,
    /// Unsigned 8-bit integers.
    UInt8,
    /// Unsigned 16-bit integers.
    UInt16,
    /// Unsigned 32-bit integers.
    UInt32,
    /// Unsigned 64-bit integers.
    UInt64,
    /// IEEE 16-bit floating point.
    Float16,
    /// IEEE 32-bit floating point.
    Float32,
    /// IEEE 64-bit floating point.
    Float64,
    /// Timestamp with a resolution and optional timezone.
    Timestamp,
    /// Days since the Unix epoch.
    Date32,
    /// Milliseconds since the Unix epoch representing whole days.
    Date64,
    /// 32-bit time of day.
    Time32,
    /// 64-bit time of day.
    Time64,
    /// 32-bit elapsed time.
    Duration32,
    /// 64-bit elapsed time.
    Duration64,
    /// Calendar interval.
    Interval,
    /// Variable-width binary with 32-bit offsets.
    Binary,
    /// Fixed-width binary.
    FixedSizeBinary,
    /// Variable-width binary with 64-bit offsets.
    LargeBinary,
    /// Binary view layout.
    BinaryView,
    /// UTF-8 with 32-bit offsets.
    Utf8,
    /// UTF-8 with 64-bit offsets.
    LargeUtf8,
    /// UTF-8 view layout.
    Utf8View,
    /// Variable list with 32-bit offsets.
    List,
    /// Variable list-view with 32-bit offsets.
    ListView,
    /// Fixed-length list.
    FixedSizeList,
    /// Variable list with 64-bit offsets.
    LargeList,
    /// Variable list-view with 64-bit offsets.
    LargeListView,
    /// Ordered struct fields.
    Struct,
    /// Tagged union fields.
    Union,
    /// Dictionary-encoded values.
    Dictionary,
    /// Exact decimal backed by 32 bits.
    Decimal32,
    /// Exact decimal backed by 64 bits.
    Decimal64,
    /// Exact decimal backed by 128 bits.
    Decimal128,
    /// Exact decimal backed by 256 bits.
    Decimal256,
    /// Arrow map entries.
    Map,
    /// Run-end encoded values.
    RunEndEncoded,
    /// Self-describing semi-structured values.
    Variant,
    /// Geospatial features on a planar coordinate system.
    Geometry,
    /// Geospatial features on the surface of a sphere or spheroid.
    Geography,
}

impl DataTypeId {
    /// Every identifier in canonical declaration order.
    pub const ALL: [Self; 45] = [
        Self::Null,
        Self::Boolean,
        Self::Int8,
        Self::Int16,
        Self::Int32,
        Self::Int64,
        Self::UInt8,
        Self::UInt16,
        Self::UInt32,
        Self::UInt64,
        Self::Float16,
        Self::Float32,
        Self::Float64,
        Self::Timestamp,
        Self::Date32,
        Self::Date64,
        Self::Time32,
        Self::Time64,
        Self::Duration32,
        Self::Duration64,
        Self::Interval,
        Self::Binary,
        Self::FixedSizeBinary,
        Self::LargeBinary,
        Self::BinaryView,
        Self::Utf8,
        Self::LargeUtf8,
        Self::Utf8View,
        Self::List,
        Self::ListView,
        Self::FixedSizeList,
        Self::LargeList,
        Self::LargeListView,
        Self::Struct,
        Self::Union,
        Self::Dictionary,
        Self::Decimal32,
        Self::Decimal64,
        Self::Decimal128,
        Self::Decimal256,
        Self::Map,
        Self::RunEndEncoded,
        Self::Variant,
        Self::Geometry,
        Self::Geography,
    ];

    /// Parse a canonical lowercase datatype name.
    ///
    /// This accepts only the parameter-free variant name. A complete datatype
    /// expression such as `decimal128(10, 2)` belongs to
    /// [`crate::DataType::from_str`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownDataType`] naming the unrecognized input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase name without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Int8 => "int8",
            Self::Int16 => "int16",
            Self::Int32 => "int32",
            Self::Int64 => "int64",
            Self::UInt8 => "uint8",
            Self::UInt16 => "uint16",
            Self::UInt32 => "uint32",
            Self::UInt64 => "uint64",
            Self::Float16 => "float16",
            Self::Float32 => "float32",
            Self::Float64 => "float64",
            Self::Timestamp => "timestamp",
            Self::Date32 => "date32",
            Self::Date64 => "date64",
            Self::Time32 => "time32",
            Self::Time64 => "time64",
            Self::Duration32 => "duration32",
            Self::Duration64 => "duration64",
            Self::Interval => "interval",
            Self::Binary => "binary",
            Self::FixedSizeBinary => "fixed_size_binary",
            Self::LargeBinary => "large_binary",
            Self::BinaryView => "binary_view",
            Self::Utf8 => "utf8",
            Self::LargeUtf8 => "large_utf8",
            Self::Utf8View => "utf8_view",
            Self::List => "list",
            Self::ListView => "list_view",
            Self::FixedSizeList => "fixed_size_list",
            Self::LargeList => "large_list",
            Self::LargeListView => "large_list_view",
            Self::Struct => "struct",
            Self::Union => "union",
            Self::Dictionary => "dictionary",
            Self::Decimal32 => "decimal32",
            Self::Decimal64 => "decimal64",
            Self::Decimal128 => "decimal128",
            Self::Decimal256 => "decimal256",
            Self::Map => "map",
            Self::RunEndEncoded => "run_end_encoded",
            Self::Variant => "variant",
            Self::Geometry => "geometry",
            Self::Geography => "geography",
        }
    }

    /// Return the coarse family this identifier belongs to.
    pub const fn kind(self) -> DataTypeKind {
        match self {
            Self::Null => DataTypeKind::Null,
            Self::Boolean => DataTypeKind::Boolean,
            Self::Int8
            | Self::Int16
            | Self::Int32
            | Self::Int64
            | Self::UInt8
            | Self::UInt16
            | Self::UInt32
            | Self::UInt64 => DataTypeKind::Integer,
            Self::Float16 | Self::Float32 | Self::Float64 => DataTypeKind::Floating,
            Self::Decimal32 | Self::Decimal64 | Self::Decimal128 | Self::Decimal256 => {
                DataTypeKind::Decimal
            }
            Self::Timestamp
            | Self::Date32
            | Self::Date64
            | Self::Time32
            | Self::Time64
            | Self::Duration32
            | Self::Duration64
            | Self::Interval => DataTypeKind::Temporal,
            Self::Binary | Self::FixedSizeBinary | Self::LargeBinary | Self::BinaryView => {
                DataTypeKind::Binary
            }
            Self::Utf8 | Self::LargeUtf8 | Self::Utf8View => DataTypeKind::String,
            Self::List
            | Self::ListView
            | Self::FixedSizeList
            | Self::LargeList
            | Self::LargeListView => DataTypeKind::List,
            Self::Struct => DataTypeKind::Struct,
            Self::Union => DataTypeKind::Union,
            Self::Map => DataTypeKind::Map,
            Self::Dictionary => DataTypeKind::Dictionary,
            Self::RunEndEncoded => DataTypeKind::RunEndEncoded,
            Self::Variant => DataTypeKind::Variant,
            Self::Geometry | Self::Geography => DataTypeKind::Geospatial,
        }
    }

    /// Return whether the variant carries parameters beyond its identity.
    ///
    /// A parameterized identifier cannot round-trip through [`Self::as_str`]
    /// alone; its complete spelling belongs to [`crate::DataType`].
    pub const fn is_parameterized(self) -> bool {
        matches!(
            self,
            Self::Timestamp
                | Self::Time32
                | Self::Time64
                | Self::Duration32
                | Self::Duration64
                | Self::Interval
                | Self::FixedSizeBinary
                | Self::List
                | Self::ListView
                | Self::FixedSizeList
                | Self::LargeList
                | Self::LargeListView
                | Self::Struct
                | Self::Union
                | Self::Dictionary
                | Self::Decimal32
                | Self::Decimal64
                | Self::Decimal128
                | Self::Decimal256
                | Self::Map
                | Self::RunEndEncoded
                | Self::Geometry
                | Self::Geography
        )
    }

    /// Return whether the variant is a signed or unsigned integer.
    pub const fn is_integer(self) -> bool {
        matches!(self.kind(), DataTypeKind::Integer)
    }

    /// Return whether the variant is a signed integer.
    pub const fn is_signed_integer(self) -> bool {
        matches!(self, Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64)
    }

    /// Return whether the variant is an unsigned integer.
    pub const fn is_unsigned_integer(self) -> bool {
        matches!(
            self,
            Self::UInt8 | Self::UInt16 | Self::UInt32 | Self::UInt64
        )
    }

    /// Return whether the variant is IEEE binary floating point.
    pub const fn is_floating(self) -> bool {
        matches!(self.kind(), DataTypeKind::Floating)
    }

    /// Return whether the variant is an exact decimal.
    pub const fn is_decimal(self) -> bool {
        matches!(self.kind(), DataTypeKind::Decimal)
    }

    /// Return whether the variant is a date, time, timestamp, duration, or interval.
    pub const fn is_temporal(self) -> bool {
        matches!(self.kind(), DataTypeKind::Temporal)
    }

    /// Return whether the variant stores opaque bytes.
    pub const fn is_binary(self) -> bool {
        matches!(self.kind(), DataTypeKind::Binary)
    }

    /// Return whether the variant stores UTF-8 text.
    pub const fn is_string(self) -> bool {
        matches!(self.kind(), DataTypeKind::String)
    }

    /// Return whether the variant always holds child fields.
    ///
    /// Wrapper variants report `false`; their nesting depends on the value
    /// type they encode, which only [`crate::DataType`] knows.
    pub const fn is_nested(self) -> bool {
        self.kind().is_nested()
    }

    /// Return whether the variant transparently encodes another value type.
    pub const fn is_wrapper(self) -> bool {
        self.kind().is_wrapper()
    }

    /// Return the fixed byte width of one value, when the variant has one.
    ///
    /// Variable-width, view, parameterized, and nested layouts return `None`.
    pub const fn fixed_byte_width(self) -> Option<usize> {
        match self {
            Self::Boolean => Some(1),
            Self::Int8 | Self::UInt8 => Some(1),
            Self::Int16 | Self::UInt16 | Self::Float16 => Some(2),
            Self::Int32 | Self::UInt32 | Self::Float32 | Self::Date32 | Self::Decimal32 => Some(4),
            Self::Int64
            | Self::UInt64
            | Self::Float64
            | Self::Date64
            | Self::Duration64
            | Self::Timestamp
            | Self::Decimal64 => Some(8),
            Self::Duration32 => Some(4),
            Self::Decimal128 => Some(16),
            Self::Decimal256 => Some(32),
            _ => None,
        }
    }
}

impl FromStr for DataTypeId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|id| value.eq_ignore_ascii_case(id.as_str()))
            .ok_or_else(|| Error::UnknownDataType(format_smolstr!("{value}")))
    }
}

impl From<DataTypeId> for DataTypeKind {
    fn from(value: DataTypeId) -> Self {
        value.kind()
    }
}

impl fmt::Display for DataTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DataTypeId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DataTypeId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{DataTypeId, DataTypeKind};

    #[test]
    fn names_round_trip_case_insensitively() {
        for id in DataTypeId::ALL {
            assert_eq!(DataTypeId::from_str(id.as_str()).unwrap(), id);
            assert_eq!(
                DataTypeId::from_str(&id.as_str().to_uppercase()).unwrap(),
                id
            );
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = DataTypeId::ALL.iter().map(|id| id.as_str()).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total);
    }

    #[test]
    fn every_kind_is_reachable() {
        for kind in DataTypeKind::ALL {
            assert!(
                DataTypeId::ALL.iter().any(|id| id.kind() == kind),
                "no identifier maps to {kind}"
            );
        }
    }

    #[test]
    fn unknown_name_reports_the_input() {
        let error = DataTypeId::from_str("int33").unwrap_err();
        assert!(error.to_string().contains("\"int33\""), "{error}");
    }

    #[test]
    fn integer_predicates_partition_the_family() {
        for id in DataTypeId::ALL.into_iter().filter(|id| id.is_integer()) {
            assert_ne!(id.is_signed_integer(), id.is_unsigned_integer());
        }
    }

    #[test]
    fn fixed_widths_match_their_layout() {
        assert_eq!(DataTypeId::Int32.fixed_byte_width(), Some(4));
        assert_eq!(DataTypeId::Decimal256.fixed_byte_width(), Some(32));
        assert_eq!(DataTypeId::Utf8.fixed_byte_width(), None);
        assert_eq!(DataTypeId::Struct.fixed_byte_width(), None);
    }
}
