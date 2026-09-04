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
    /// Signed 128-bit integers.
    ///
    /// Arrow has no 128-bit integer layout, so no [`crate::DataType`] answers
    /// this identifier. It names the width [`crate::Scalar::I128`] stores and
    /// the canonical identity a negative integer of any width carries into
    /// [`crate::Scalar::write_bytes`].
    Int128,
    /// Unsigned 128-bit integers.
    ///
    /// The unsigned half of the pair [`Self::Int128`] documents.
    UInt128,
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
    /// ASCII text padded with trailing NUL to 2 bytes.
    Ascii16,
    /// ASCII text padded with trailing NUL to 3 bytes.
    Ascii24,
    /// ASCII text padded with trailing NUL to 4 bytes.
    Ascii32,
    /// ASCII text padded with trailing NUL to 8 bytes.
    Ascii64,
    /// ASCII text padded with trailing NUL to 12 bytes.
    Ascii96,
    /// ASCII text padded with trailing NUL to 16 bytes.
    Ascii128,
    /// ISO 3166-1 alpha-2: a country code, two ASCII bytes.
    Country,
    /// ISO 4217: a currency code, three ASCII bytes.
    Currency,
    /// ISO 10383: a market identifier code, four ASCII bytes.
    Mic,
    /// ISO 10962: a classification of financial instruments, six ASCII bytes.
    Cfi,
    /// One 128-bit universally unique identifier.
    Guid,
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
    pub const ALL: [Self; 58] = [
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
        Self::Int128,
        Self::UInt128,
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
        Self::Ascii16,
        Self::Ascii24,
        Self::Ascii32,
        Self::Ascii64,
        Self::Ascii96,
        Self::Ascii128,
        Self::Country,
        Self::Currency,
        Self::Mic,
        Self::Cfi,
        Self::Guid,
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
            Self::Int128 => "int128",
            Self::UInt128 => "uint128",
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
            Self::Ascii16 => "ascii16",
            Self::Ascii24 => "ascii24",
            Self::Ascii32 => "ascii32",
            Self::Ascii64 => "ascii64",
            Self::Ascii96 => "ascii96",
            Self::Ascii128 => "ascii128",
            Self::Country => "country",
            Self::Currency => "currency",
            Self::Mic => "mic",
            Self::Cfi => "cfi",
            Self::Guid => "guid",
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

    /// Return this identifier's discriminant as one byte.
    ///
    /// The number is the variant's position in the declaration order
    /// [`Self::ALL`] lists, and it is a wire contract:
    /// [`crate::Scalar::write_bytes`] writes it as the tag of every value, so
    /// inserting a variant anywhere but the end changes stored digests. The
    /// test pinning every value is what makes that a failure rather than a
    /// surprise.
    ///
    /// ```
    /// use yggdryl::DataTypeId;
    ///
    /// assert_eq!(DataTypeId::Null.as_u8(), 0);
    /// assert_eq!(DataTypeId::Int128.as_u8(), 10);
    /// ```
    pub const fn as_u8(self) -> u8 {
        self as u8
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
            | Self::UInt64
            | Self::Int128
            | Self::UInt128 => DataTypeKind::Integer,
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
            Self::Utf8
            | Self::LargeUtf8
            | Self::Utf8View
            | Self::Ascii16
            | Self::Ascii24
            | Self::Ascii32
            | Self::Ascii64
            | Self::Ascii96
            | Self::Ascii128
            // A registered code is fixed-width ASCII text with an identity,
            // so it belongs to the family every text behaviour is uniform
            // over: comparison, casting to a variable layout, merging.
            | Self::Country
            | Self::Currency
            | Self::Mic
            | Self::Cfi => DataTypeKind::String,
            Self::Guid => DataTypeKind::Guid,
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
        matches!(
            self,
            Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64 | Self::Int128
        )
    }

    /// Return whether the variant is an unsigned integer.
    pub const fn is_unsigned_integer(self) -> bool {
        matches!(
            self,
            Self::UInt8 | Self::UInt16 | Self::UInt32 | Self::UInt64 | Self::UInt128
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
            Self::Int32
            | Self::UInt32
            | Self::Float32
            | Self::Date32
            | Self::Decimal32
            | Self::Ascii32 => Some(4),
            Self::Int64
            | Self::UInt64
            | Self::Float64
            | Self::Date64
            | Self::Duration64
            | Self::Timestamp
            | Self::Decimal64
            | Self::Ascii64 => Some(8),
            Self::Duration32 | Self::Mic => Some(4),
            Self::Ascii16 | Self::Country => Some(2),
            Self::Ascii24 | Self::Currency => Some(3),
            Self::Cfi => Some(6),
            Self::Ascii96 => Some(12),
            Self::Int128 | Self::UInt128 | Self::Decimal128 | Self::Ascii128 | Self::Guid => {
                Some(16)
            }
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
    fn ascii_widths_and_codes_are_parameter_free_text() {
        assert_eq!(DataTypeId::ALL.len(), 58);
        for id in [
            DataTypeId::Ascii16,
            DataTypeId::Ascii24,
            DataTypeId::Ascii32,
            DataTypeId::Ascii64,
            DataTypeId::Ascii96,
            DataTypeId::Ascii128,
            DataTypeId::Country,
            DataTypeId::Currency,
            DataTypeId::Mic,
            DataTypeId::Cfi,
        ] {
            assert_eq!(id.kind(), DataTypeKind::String);
            assert!(id.is_string());
            assert!(!id.is_parameterized());
        }
        assert_eq!(
            DataTypeId::from_str("ASCII128").unwrap(),
            DataTypeId::Ascii128
        );
        assert_eq!(
            DataTypeId::from_str("Currency").unwrap(),
            DataTypeId::Currency
        );
        // Each code stores the width its standard fixes, and `cfi` takes six
        // bytes, which is a width no ASCII variant has.
        assert_eq!(DataTypeId::Country.fixed_byte_width(), Some(2));
        assert_eq!(DataTypeId::Currency.fixed_byte_width(), Some(3));
        assert_eq!(DataTypeId::Mic.fixed_byte_width(), Some(4));
        assert_eq!(DataTypeId::Cfi.fixed_byte_width(), Some(6));
    }

    #[test]
    fn the_two_arrow_less_integer_widths_are_named_here_and_nowhere_in_datatype() {
        // Arrow has no 128-bit integer layout, so these two identifiers name a
        // width `Scalar` stores and `DataType` cannot. Every integer predicate
        // still has to place them, and `DataType::id` still has to be able to
        // produce every *other* identifier.
        for id in [DataTypeId::Int128, DataTypeId::UInt128] {
            assert!(id.is_integer());
            assert_eq!(id.kind(), DataTypeKind::Integer);
            assert_eq!(id.fixed_byte_width(), Some(16));
            assert!(!id.is_parameterized());
        }
        assert!(DataTypeId::Int128.is_signed_integer());
        assert!(DataTypeId::UInt128.is_unsigned_integer());
        assert_eq!(DataTypeId::from_str("int128").unwrap(), DataTypeId::Int128);
        assert_eq!(
            DataTypeId::from_str("UINT128").unwrap(),
            DataTypeId::UInt128
        );
        // The datatype grammar does not accept them, because no Arrow layout
        // holds one.
        assert!(crate::DataType::from_str("int128").is_err());
    }

    #[test]
    fn discriminants_are_the_declaration_order_and_are_pinned() {
        // The byte `Scalar::write_bytes` writes as a value's tag. Inserting a
        // variant anywhere but the end moves every later number and changes
        // stored digests, which is what this pins.
        for (index, id) in DataTypeId::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(id.as_u8()), index, "{id}");
        }
        assert_eq!(DataTypeId::Null.as_u8(), 0);
        assert_eq!(DataTypeId::Boolean.as_u8(), 1);
        assert_eq!(DataTypeId::UInt64.as_u8(), 9);
        assert_eq!(DataTypeId::Int128.as_u8(), 10);
        assert_eq!(DataTypeId::UInt128.as_u8(), 11);
        assert_eq!(DataTypeId::Float64.as_u8(), 14);
        assert_eq!(DataTypeId::Timestamp.as_u8(), 15);
        assert_eq!(DataTypeId::Date64.as_u8(), 17);
        assert_eq!(DataTypeId::Time64.as_u8(), 19);
        assert_eq!(DataTypeId::Duration64.as_u8(), 21);
        assert_eq!(DataTypeId::Binary.as_u8(), 23);
        assert_eq!(DataTypeId::Utf8.as_u8(), 27);
        assert_eq!(DataTypeId::Country.as_u8(), 36);
        assert_eq!(DataTypeId::Cfi.as_u8(), 39);
        assert_eq!(DataTypeId::Guid.as_u8(), 40);
        assert_eq!(DataTypeId::List.as_u8(), 41);
        assert_eq!(DataTypeId::Struct.as_u8(), 46);
        assert_eq!(DataTypeId::Dictionary.as_u8(), 48);
        assert_eq!(DataTypeId::Decimal256.as_u8(), 52);
        assert_eq!(DataTypeId::Map.as_u8(), 53);
        assert_eq!(DataTypeId::Geometry.as_u8(), 56);
        assert_eq!(DataTypeId::Geography.as_u8(), 57);
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
        assert_eq!(DataTypeId::Ascii32.fixed_byte_width(), Some(4));
        assert_eq!(DataTypeId::Ascii16.fixed_byte_width(), Some(2));
        assert_eq!(DataTypeId::Ascii24.fixed_byte_width(), Some(3));
        assert_eq!(DataTypeId::Ascii96.fixed_byte_width(), Some(12));
        assert_eq!(DataTypeId::Ascii64.fixed_byte_width(), Some(8));
        assert_eq!(DataTypeId::Ascii128.fixed_byte_width(), Some(16));
        assert_eq!(DataTypeId::Utf8.fixed_byte_width(), None);
        assert_eq!(DataTypeId::Struct.fixed_byte_width(), None);
    }
}
