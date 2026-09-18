use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::DataTypeKind;
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
#[repr(u8)]
pub enum DataTypeId {
    /// Null values.
    Null = 0,
    /// Boolean values.
    Boolean = 1,
    /// Signed 8-bit integers.
    Int8 = 2,
    /// Signed 16-bit integers.
    Int16 = 3,
    /// Signed 32-bit integers.
    Int32 = 4,
    /// Signed 64-bit integers.
    Int64 = 5,
    /// Unsigned 8-bit integers.
    UInt8 = 6,
    /// Unsigned 16-bit integers.
    UInt16 = 7,
    /// Unsigned 32-bit integers.
    UInt32 = 8,
    /// Unsigned 64-bit integers.
    UInt64 = 9,
    /// Signed 128-bit integers.
    ///
    /// Arrow has no 128-bit integer layout, so no [`crate::DataType`] answers
    /// this identifier. It names the width [`crate::types::integer::Int128`] stores and
    /// the canonical identity a negative integer of any width carries into
    /// [`crate::Scalar::write_bytes`].
    Int128 = 10,
    /// Unsigned 128-bit integers.
    ///
    /// The unsigned half of the pair [`Self::Int128`] documents.
    UInt128 = 11,
    /// IEEE 16-bit floating point.
    Float16 = 12,
    /// IEEE 32-bit floating point.
    Float32 = 13,
    /// IEEE 64-bit floating point.
    Float64 = 14,
    /// A 64-bit datetime with a resolution and explicit timezone marker.
    DateTime64 = 15,
    /// Days since the Unix epoch.
    Date32 = 16,
    /// Milliseconds since the Unix epoch representing whole days.
    Date64 = 17,
    /// 32-bit time of day.
    Time32 = 18,
    /// 64-bit time of day.
    Time64 = 19,
    /// 32-bit elapsed time.
    Duration32 = 20,
    /// 64-bit elapsed time.
    Duration64 = 21,
    /// Calendar interval.
    Interval = 22,
    /// Bytes with 32-bit offsets, under any bound.
    Binary = 23,
    /// Bytes of one fixed width.
    FixedSizeBinary = 24,
    /// Bytes with 64-bit offsets.
    LargeBinary = 25,
    /// Bytes in the view layout.
    BinaryView = 26,
    /// A string with 32-bit offsets, in any charset, with any bound.
    ///
    /// The five string layouts sit where the five text identifiers they
    /// replaced sat, so every later number - and every digest tag - stays
    /// what it was; this one is the number UTF-8 text always fed.
    String = 27,
    /// A string of one fixed, padded byte width.
    FixedString = 28,
    /// A string in the view layout.
    StringView = 29,
    /// A string with 64-bit offsets.
    LargeString = 30,
    /// A string in the view layout, declared large.
    LargeStringView = 31,
    /// ISO 3166-1 alpha-2: a country code, two ASCII bytes.
    Country = 32,
    /// ISO 4217: a currency code, three ASCII bytes.
    Currency = 33,
    /// ISO 10383: a market identifier code, four ASCII bytes.
    Mic = 34,
    /// ISO 10962: a classification of financial instruments, six ASCII bytes.
    Cfi = 35,
    /// One 128-bit universally unique identifier.
    Uuid = 36,
    /// Variable list with 32-bit offsets.
    List = 37,
    /// Variable list-view with 32-bit offsets.
    ListView = 38,
    /// Fixed-length list.
    FixedSizeList = 39,
    /// Variable list with 64-bit offsets.
    LargeList = 40,
    /// Variable list-view with 64-bit offsets.
    LargeListView = 41,
    /// Ordered struct fields.
    Struct = 42,
    /// Tagged union fields.
    Union = 43,
    /// Dictionary-encoded values.
    Dictionary = 44,
    /// Exact decimal backed by 32 bits.
    Decimal32 = 45,
    /// Exact decimal backed by 64 bits.
    Decimal64 = 46,
    /// Exact decimal backed by 128 bits.
    Decimal128 = 47,
    /// Exact decimal backed by 256 bits.
    Decimal256 = 48,
    /// Arrow map entries.
    Map = 49,
    /// Run-end encoded values.
    RunEndEncoded = 50,
    /// Self-describing semi-structured values.
    Variant = 51,
    /// Geospatial features on a planar coordinate system.
    Geometry = 52,
    /// Geospatial features on the surface of a sphere or spheroid.
    Geography = 53,
    /// A canonical, numerically ordered software or protocol version.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Version = 54,
    /// FIX's side of a trade, four ASCII bytes.
    Side = 55,
    /// What state one thing is in.
    State = 56,
    /// How long an order stands.
    TimeInForce = 57,
    /// A validated, canonical location.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Url = 59,
    /// ISO 6166: a securities identification number, twelve ASCII bytes.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Isin = 60,
    /// A canonical time zone name, a fixed offset, or the zone-free marker.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Timezone = 61,
    /// A validated, canonical MIME type.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    MimeType = 62,
    /// A MIME type with its charset and content codings.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    MediaType = 63,
    /// CUSIP: a North American securities identifier, nine ASCII bytes.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Cusip = 64,
    /// SEDOL: a London Stock Exchange securities identifier, seven ASCII
    /// bytes.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Sedol = 65,
    Bloomberg = 66,
    /// Arrow map entries whose keys are ordered within each row.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    SortedMap = 67,
    /// Exactly two children: a structure of a first and a second field.
    ///
    /// Appended because [`Self::as_u8`] is a wire contract.
    Struct2 = 68,
}

impl DataTypeId {
    /// Every identifier in canonical declaration order.
    pub const ALL: [Self; 68] = [
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
        Self::DateTime64,
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
        Self::String,
        Self::FixedString,
        Self::StringView,
        Self::LargeString,
        Self::LargeStringView,
        Self::Country,
        Self::Currency,
        Self::Mic,
        Self::Cfi,
        Self::Uuid,
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
        Self::Version,
        Self::Side,
        Self::State,
        Self::TimeInForce,
        Self::Url,
        Self::Isin,
        Self::Timezone,
        Self::MimeType,
        Self::MediaType,
        Self::Cusip,
        Self::Sedol,
        Self::Bloomberg,
        Self::SortedMap,
        Self::Struct2,
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
            Self::DateTime64 => "datetime64",
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
            Self::Country => "country",
            Self::Currency => "currency",
            Self::Mic => "mic",
            Self::Cfi => "cfi",
            Self::Isin => "isin",
            Self::Cusip => "cusip",
            Self::Sedol => "sedol",
            Self::Bloomberg => "bloomberg",
            Self::Side => "side",
            Self::State => "state",
            Self::TimeInForce => "timeinforce",
            Self::Uuid => "uuid",
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
            Self::SortedMap => "sorted_map",
            Self::Struct2 => "struct2",
            Self::RunEndEncoded => "run_end_encoded",
            Self::Variant => "variant",
            Self::Geometry => "geometry",
            Self::Geography => "geography",
            Self::Version => "version",
            Self::Url => "url",
            Self::Timezone => "timezone",
            Self::MimeType => "mimetype",
            Self::MediaType => "mediatype",
            Self::String => "string",
            Self::FixedString => "fixed_string",
            Self::StringView => "string_view",
            Self::LargeString => "large_string",
            Self::LargeStringView => "large_string_view",
        }
    }

    /// Return this identifier's discriminant as one byte.
    ///
    /// Every variant states its discriminant, and it is a wire contract:
    /// [`crate::Scalar::write_bytes`] writes it as the tag of every value, so
    /// a number is never reused and never moves. A retired variant leaves its
    /// number unused - 58 was `msgdirection`, since retired - so
    /// the byte is no longer the variant's position in [`Self::ALL`]; the
    /// test pinning every value is what makes a moved number a failure rather
    /// than a surprise.
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
            Self::DateTime64
            | Self::Date32
            | Self::Date64
            | Self::Time32
            | Self::Time64
            | Self::Duration32
            | Self::Duration64
            | Self::Interval => DataTypeKind::Temporal,
            Self::Binary | Self::FixedSizeBinary | Self::LargeBinary | Self::BinaryView => {
                DataTypeKind::Bytes
            }
            Self::String
            | Self::FixedString
            | Self::StringView
            | Self::LargeString
            | Self::LargeStringView
            | Self::Version
            | Self::Url
            | Self::Timezone
            | Self::MimeType
            | Self::MediaType => DataTypeKind::Text,
            // A registered code is fixed-width ASCII text with an identity;
            // the family is the identity, and every text behaviour - comparison,
            // casting to a variable layout, merging - is uniform over it too.
            Self::Country
            | Self::Currency
            | Self::Mic
            | Self::Cfi
            | Self::Isin
            | Self::Cusip
            | Self::Sedol
            | Self::Bloomberg
            | Self::Side
            | Self::State
            | Self::TimeInForce => DataTypeKind::Code,
            Self::Uuid => DataTypeKind::Uuid,
            Self::List
            | Self::ListView
            | Self::FixedSizeList
            | Self::LargeList
            | Self::LargeListView
            | Self::Struct
            | Self::Union
            | Self::Map
            | Self::SortedMap
            | Self::Struct2
            | Self::Dictionary
            | Self::RunEndEncoded
            | Self::Variant => DataTypeKind::Nested,
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
            Self::DateTime64
                | Self::Time32
                | Self::Time64
                | Self::Duration32
                | Self::Duration64
                | Self::Interval
                | Self::Binary
                | Self::FixedSizeBinary
                | Self::LargeBinary
                | Self::BinaryView
                | Self::String
                | Self::FixedString
                | Self::StringView
                | Self::LargeString
                | Self::LargeStringView
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
                | Self::SortedMap
                | Self::Struct2
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

    /// Return whether the variant is a date, time, datetime, duration, or interval.
    pub const fn is_temporal(self) -> bool {
        matches!(self.kind(), DataTypeKind::Temporal)
    }

    /// Return whether the variant stores opaque bytes.
    pub const fn is_binary(self) -> bool {
        matches!(self.kind(), DataTypeKind::Bytes)
    }

    /// Return whether the variant stores text: a string, a code, a version
    /// or a URL.
    pub const fn is_string(self) -> bool {
        matches!(self.kind(), DataTypeKind::Text | DataTypeKind::Code)
    }

    /// Return whether the variant always holds child fields.
    ///
    /// Wrapper variants report `false`; their nesting depends on the value
    /// type they encode, which only [`crate::DataType`] knows.
    pub const fn is_nested(self) -> bool {
        matches!(
            self,
            Self::List
                | Self::ListView
                | Self::FixedSizeList
                | Self::LargeList
                | Self::LargeListView
                | Self::Struct
                | Self::Union
                | Self::Map
                | Self::SortedMap
                | Self::Struct2
                | Self::Variant
        )
    }

    /// Return whether the variant transparently encodes another value type.
    pub const fn is_wrapper(self) -> bool {
        matches!(self, Self::Dictionary | Self::RunEndEncoded)
    }

    /// Return the fixed byte width of one value, when the variant has one.
    ///
    /// Variable-width, view, parameterized, and nested layouts return `None`.
    /// A registered code is variable-width text, so its standard's width is
    /// [`Self::code_width`] - a maximum - rather than a width answered here.
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
            | Self::DateTime64
            | Self::Decimal64 => Some(8),
            Self::Duration32 => Some(4),
            Self::Int128 | Self::UInt128 | Self::Decimal128 | Self::Uuid => Some(16),
            Self::Decimal256 => Some(32),
            _ => None,
        }
    }

    /// Return the most bytes one registered code's value may be.
    ///
    /// The number each standard fixes: two for a country, three for a
    /// currency, four for a market identifier or a side, six for a
    /// classification, seven for a SEDOL, eight for a time in force, nine
    /// for a CUSIP, ten for a state, twelve for an ISIN, and thirty-two for
    /// a Bloomberg identifier - the one whose width is only a bound, because
    /// a ticker, a market and a yellow key have no fixed length between
    /// them. It is a
    /// maximum, not a layout - a code stores as the text it is - and it is
    /// what the value rule holds a cell to and what
    /// [`crate::DataType::ascii_packed`] pads into.
    ///
    /// Every other variant returns `None`.
    pub const fn code_width(self) -> Option<usize> {
        match self {
            Self::Country => Some(2),
            Self::Currency => Some(3),
            Self::Mic => Some(4),
            Self::Cfi => Some(6),
            Self::Sedol => Some(7),
            Self::Side | Self::TimeInForce => Some(8),
            Self::Cusip => Some(9),
            Self::State => Some(10),
            Self::Isin => Some(12),
            Self::Bloomberg => Some(32),
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
