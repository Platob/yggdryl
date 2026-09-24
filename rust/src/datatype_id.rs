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
/// The number each variant states is one byte laid out by family: every
/// [`DataTypeKind`] owns a [range](DataTypeKind::range) that starts at the
/// family's own number - [`DataTypeKind::id`], a placeholder no leaf takes,
/// but for `null`, whose one leaf states it - and its leaves follow in that
/// range, so the byte says its family and a family has room for the leaves
/// it does not have yet. The byte is what
/// [the variant encoding](crate::Scalar::into_value_bytes) and
/// [`crate::Scalar::write_bytes`] write as a value's tag, and
/// [`Self::from_u8`] reads it back.
///
/// Use [`DataTypeKind`] through [`Self::kind`] when only the family matters.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
#[repr(u8)]
pub enum DataTypeId {
    // Null: 0x00..0x07
    /// Null values.
    Null = 0x00,
    // Boolean: 0x08..0x0f
    /// Boolean values.
    Boolean = 0x09,
    // Integer: 0x10..0x1f
    /// Signed 8-bit integers.
    Int8 = 0x11,
    /// Signed 16-bit integers.
    Int16 = 0x12,
    /// Signed 32-bit integers.
    Int32 = 0x13,
    /// Signed 64-bit integers.
    Int64 = 0x14,
    /// Signed 128-bit integers.
    ///
    /// Arrow has no 128-bit integer layout, so no [`crate::DataType`] answers
    /// this identifier. It names the width [`crate::integer::Int128`] stores and
    /// the canonical identity a negative integer of any width carries into
    /// [`crate::Scalar::write_bytes`].
    Int128 = 0x15,
    /// Unsigned 8-bit integers.
    UInt8 = 0x19,
    /// Unsigned 16-bit integers.
    UInt16 = 0x1a,
    /// Unsigned 32-bit integers.
    UInt32 = 0x1b,
    /// Unsigned 64-bit integers.
    UInt64 = 0x1c,
    /// Unsigned 128-bit integers.
    ///
    /// The unsigned half of the pair [`Self::Int128`] documents.
    UInt128 = 0x1d,
    // Floating: 0x20..0x27
    /// IEEE 16-bit floating point.
    Float16 = 0x21,
    /// IEEE 32-bit floating point.
    Float32 = 0x22,
    /// IEEE 64-bit floating point.
    Float64 = 0x23,
    // Decimal: 0x28..0x2f
    /// Exact decimal backed by 32 bits.
    Decimal32 = 0x29,
    /// Exact decimal backed by 64 bits.
    Decimal64 = 0x2a,
    /// Exact decimal backed by 128 bits.
    Decimal128 = 0x2b,
    /// Exact decimal backed by 256 bits.
    Decimal256 = 0x2c,
    // Temporal: 0x30..0x3f
    /// A 64-bit datetime with a resolution and explicit timezone marker.
    DateTime64 = 0x31,
    /// Days since the Unix epoch.
    Date32 = 0x32,
    /// Milliseconds since the Unix epoch representing whole days.
    Date64 = 0x33,
    /// 32-bit time of day.
    Time32 = 0x34,
    /// 64-bit time of day.
    Time64 = 0x35,
    /// 32-bit elapsed time.
    Duration32 = 0x36,
    /// 64-bit elapsed time.
    Duration64 = 0x37,
    /// Calendar interval.
    Interval = 0x38,
    // Bytes: 0x40..0x4f
    /// Bytes with 32-bit offsets, under any bound.
    Binary = 0x41,
    /// Bytes with 64-bit offsets.
    LargeBinary = 0x42,
    /// Bytes in the view layout.
    BinaryView = 0x43,
    /// The viewed byte layout over 64-bit offsets.
    LargeBinaryView = 0x44,
    /// Bytes of one fixed width.
    FixedBinary = 0x45,
    /// Bytes under a declared maximum.
    SizedBinary = 0x46,
    // Text: 0x50..0x6f
    /// Any length of UTF-8 with 32-bit offsets.
    Utf8String = 0x51,
    /// UTF-8 with 64-bit offsets.
    LargeUtf8String = 0x52,
    /// UTF-8 in the view layout.
    Utf8StringView = 0x53,
    /// UTF-8 in the view layout, declared large.
    LargeUtf8StringView = 0x54,
    /// UTF-8 of one fixed, padded byte width.
    FixedUtf8String = 0x55,
    /// UTF-8 under a declared maximum.
    SizedUtf8String = 0x56,
    /// Any length of US-ASCII with 32-bit offsets.
    AsciiString = 0x57,
    /// US-ASCII with 64-bit offsets.
    LargeAsciiString = 0x58,
    /// US-ASCII in the view layout.
    AsciiStringView = 0x59,
    /// US-ASCII in the view layout, declared large.
    LargeAsciiStringView = 0x5a,
    /// US-ASCII of one fixed, padded byte width.
    FixedAsciiString = 0x5b,
    /// US-ASCII under a declared maximum.
    SizedAsciiString = 0x5c,
    /// Any length of windows-1252 with 32-bit offsets.
    Cp1252String = 0x5d,
    /// Windows-1252 with 64-bit offsets.
    LargeCp1252String = 0x5e,
    /// Windows-1252 in the view layout.
    Cp1252StringView = 0x5f,
    /// Windows-1252 in the view layout, declared large.
    LargeCp1252StringView = 0x60,
    /// Windows-1252 of one fixed, padded byte width.
    FixedCp1252String = 0x61,
    /// Windows-1252 under a declared maximum.
    SizedCp1252String = 0x62,
    /// A canonical, numerically ordered software or protocol version.
    Version = 0x63,
    /// A validated, canonical location.
    Url = 0x64,
    /// A validated, canonical resource name.
    Urn = 0x65,
    /// A canonical time zone name, a fixed offset, or the zone-free marker.
    Timezone = 0x66,
    /// A validated, canonical MIME type.
    MimeType = 0x67,
    /// A MIME type with its charset and content codings.
    MediaType = 0x68,
    // Code: 0x70..0x7f
    /// ISO 3166-1 alpha-2: a country code, two ASCII bytes.
    Country = 0x71,
    /// ISO 4217: a currency code, three ASCII bytes.
    Ccy = 0x72,
    /// ISO 10383: a market identifier code, four ASCII bytes.
    MicCode = 0x73,
    /// ISO 10962: a classification of financial instruments, six ASCII bytes.
    CfiCode = 0x74,
    /// FIX's side of a trade, four ASCII bytes.
    Side = 0x75,
    /// What state one thing is in.
    State = 0x76,
    /// How long an order stands.
    TimeInForce = 0x77,
    /// ISO 6166: a securities identification number, twelve ASCII bytes.
    IsinCode = 0x78,
    /// CUSIP: a North American securities identifier, nine ASCII bytes.
    CusipCode = 0x79,
    /// SEDOL: a London Stock Exchange securities identifier, seven ASCII
    /// bytes.
    SedolCode = 0x7a,
    /// A Bloomberg identifier: ticker, market and yellow key, up to thirty-two
    /// ASCII bytes.
    BloombergCode = 0x7b,
    /// ANSI X9.145 Financial Instrument Global Identifier, twelve ASCII bytes.
    FIGICode = 0x7c,
    // Uuid: 0x80..0x8f
    /// One 128-bit universally unique identifier.
    Uuid = 0x81,
    // Nested: 0x90..0xaf
    /// A serie of items behind 32-bit offsets.
    Serie = 0x91,
    /// A serie of items behind 64-bit offsets.
    LargeSerie = 0x92,
    /// A serie of items behind 32-bit offsets and sizes.
    SerieView = 0x93,
    /// A serie of items behind 64-bit offsets and sizes.
    LargeSerieView = 0x94,
    /// A serie of exactly one length of items.
    FixedSizeSerie = 0x95,
    /// Ordered struct fields.
    Struct = 0x96,
    /// Arrow map entries.
    Map = 0x97,
    /// Arrow map entries whose keys are ordered within each row.
    SortedMap = 0x98,
    /// Tagged union fields.
    Union = 0x99,
    /// Dictionary-encoded values.
    Dictionary = 0x9a,
    /// Run-end encoded values.
    RunEndEncoded = 0x9b,
    /// Self-describing semi-structured values.
    Variant = 0x9c,
    // Geospatial: 0xb0..0xbf
    /// Geospatial features on a planar coordinate system.
    Geometry = 0xb1,
    /// Geospatial features on the surface of a sphere or spheroid.
    Geography = 0xb2,
}

impl DataTypeId {
    /// Every identifier in canonical declaration order.
    pub const ALL: [Self; 84] = [
        Self::Null,
        Self::Boolean,
        Self::Int8,
        Self::Int16,
        Self::Int32,
        Self::Int64,
        Self::Int128,
        Self::UInt8,
        Self::UInt16,
        Self::UInt32,
        Self::UInt64,
        Self::UInt128,
        Self::Float16,
        Self::Float32,
        Self::Float64,
        Self::Decimal32,
        Self::Decimal64,
        Self::Decimal128,
        Self::Decimal256,
        Self::DateTime64,
        Self::Date32,
        Self::Date64,
        Self::Time32,
        Self::Time64,
        Self::Duration32,
        Self::Duration64,
        Self::Interval,
        Self::Binary,
        Self::LargeBinary,
        Self::BinaryView,
        Self::LargeBinaryView,
        Self::FixedBinary,
        Self::SizedBinary,
        Self::Utf8String,
        Self::LargeUtf8String,
        Self::Utf8StringView,
        Self::LargeUtf8StringView,
        Self::FixedUtf8String,
        Self::SizedUtf8String,
        Self::AsciiString,
        Self::LargeAsciiString,
        Self::AsciiStringView,
        Self::LargeAsciiStringView,
        Self::FixedAsciiString,
        Self::SizedAsciiString,
        Self::Cp1252String,
        Self::LargeCp1252String,
        Self::Cp1252StringView,
        Self::LargeCp1252StringView,
        Self::FixedCp1252String,
        Self::SizedCp1252String,
        Self::Version,
        Self::Url,
        Self::Urn,
        Self::Timezone,
        Self::MimeType,
        Self::MediaType,
        Self::Country,
        Self::Ccy,
        Self::MicCode,
        Self::CfiCode,
        Self::Side,
        Self::State,
        Self::TimeInForce,
        Self::IsinCode,
        Self::CusipCode,
        Self::SedolCode,
        Self::BloombergCode,
        Self::FIGICode,
        Self::Uuid,
        Self::Serie,
        Self::LargeSerie,
        Self::SerieView,
        Self::LargeSerieView,
        Self::FixedSizeSerie,
        Self::Struct,
        Self::Map,
        Self::SortedMap,
        Self::Union,
        Self::Dictionary,
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
            Self::DateTime64 => "datetime64",
            Self::Date32 => "date32",
            Self::Date64 => "date64",
            Self::Time32 => "time32",
            Self::Time64 => "time64",
            Self::Duration32 => "duration32",
            Self::Duration64 => "duration64",
            Self::Interval => "interval",
            Self::Binary => "binary",
            Self::FixedBinary => "fixed_binary",
            Self::LargeBinary => "large_binary",
            Self::BinaryView => "binary_view",
            Self::Country => "country",
            Self::Ccy => "ccy",
            Self::MicCode => "mic",
            Self::CfiCode => "cfi",
            Self::IsinCode => "isin",
            Self::CusipCode => "cusip",
            Self::SedolCode => "sedol",
            Self::BloombergCode => "bloomberg",
            Self::FIGICode => "figi",
            Self::Side => "side",
            Self::State => "state",
            Self::TimeInForce => "timeinforce",
            Self::Uuid => "uuid",
            Self::LargeBinaryView => "large_binary_view",
            Self::SizedBinary => "sized_binary",
            Self::Serie => "serie",
            Self::SerieView => "serie_view",
            Self::FixedSizeSerie => "fixed_size_serie",
            Self::LargeSerie => "large_serie",
            Self::LargeSerieView => "large_serie_view",
            Self::Struct => "struct",
            Self::Union => "union",
            Self::Dictionary => "dictionary",
            Self::Decimal32 => "decimal32",
            Self::Decimal64 => "decimal64",
            Self::Decimal128 => "decimal128",
            Self::Decimal256 => "decimal256",
            Self::Map => "map",
            Self::SortedMap => "sorted_map",
            Self::RunEndEncoded => "run_end_encoded",
            Self::Variant => "variant",
            Self::Geometry => "geometry",
            Self::Geography => "geography",
            Self::Version => "version",
            Self::Url => "url",
            Self::Urn => "urn",
            Self::Timezone => "timezone",
            Self::MimeType => "mimetype",
            Self::MediaType => "mediatype",
            Self::Utf8String => "utf8",
            Self::FixedUtf8String => "fixed_utf8",
            Self::Utf8StringView => "utf8_view",
            Self::LargeUtf8String => "large_utf8",
            Self::LargeUtf8StringView => "large_utf8_view",
            Self::SizedUtf8String => "sized_utf8",
            Self::AsciiString => "ascii",
            Self::LargeAsciiString => "large_ascii",
            Self::AsciiStringView => "ascii_view",
            Self::LargeAsciiStringView => "large_ascii_view",
            Self::FixedAsciiString => "fixed_ascii",
            Self::SizedAsciiString => "sized_ascii",
            Self::Cp1252String => "cp1252",
            Self::LargeCp1252String => "large_cp1252",
            Self::Cp1252StringView => "cp1252_view",
            Self::LargeCp1252StringView => "large_cp1252_view",
            Self::FixedCp1252String => "fixed_cp1252",
            Self::SizedCp1252String => "sized_cp1252",
        }
    }

    /// Return this identifier's discriminant as one byte.
    ///
    /// Every variant states its number, and the number is a wire contract:
    /// [the variant encoding](crate::Scalar::into_value_bytes) and
    /// [`crate::Scalar::write_bytes`] write it as the tag of every value, so
    /// a number is never reused and never moves. The numbers are laid out
    /// by family - the family's own number first, [`DataTypeKind::id`],
    /// then its leaves - and a byte no variant states is a placeholder in
    /// its family's range, which is how a leaf added later lands beside its
    /// family rather than at the end; the test pinning every value is what
    /// makes a moved number a failure rather than a surprise.
    ///
    /// ```
    /// use yggdryl::{DataTypeId, DataTypeKind};
    ///
    /// assert_eq!(DataTypeId::Null.as_u8(), 0x00);
    /// assert_eq!(DataTypeId::Int32.as_u8(), 0x13);
    /// assert_eq!(DataTypeKind::Integer.id(), 0x10);
    /// assert_eq!(DataTypeId::from_u8(0x13), Some(DataTypeId::Int32));
    /// assert_eq!(DataTypeId::from_u8(0x10), None, "a family's own number names no leaf");
    /// ```
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// The identifier one byte names, or nothing for a byte no variant
    /// states: a family's own number, a placeholder in a family's range, or
    /// a byte past every family.
    pub const fn from_u8(byte: u8) -> Option<Self> {
        FROM_U8[byte as usize]
    }

    /// Return the coarse family this identifier belongs to: the family whose
    /// [range](DataTypeKind::range) its byte is in.
    ///
    /// ```
    /// use yggdryl::{DataTypeId, DataTypeKind};
    ///
    /// assert_eq!(DataTypeId::UInt128.kind(), DataTypeKind::Integer);
    /// assert_eq!(DataTypeId::Url.kind(), DataTypeKind::Text);
    /// assert!(DataTypeId::Url.kind().contains(DataTypeId::Url));
    /// ```
    pub const fn kind(self) -> DataTypeKind {
        match DataTypeKind::of_u8(self.as_u8()) {
            Some(kind) => kind,
            None => panic!("every identifier sits in a family's range"),
        }
    }

    /// The temporal family this identifier is a leaf of - a date, a time of
    /// day, a datetime, a duration or a calendar interval - and `None`
    /// outside [`DataTypeKind::Temporal`]. The widths of one family share it.
    pub(crate) const fn temporal_kind(self) -> Option<crate::TemporalKind> {
        use crate::TemporalKind as T;
        match self {
            Self::Date32 | Self::Date64 => Some(T::Date),
            Self::Time32 | Self::Time64 => Some(T::Time),
            Self::DateTime64 => Some(T::DateTime),
            Self::Duration32 | Self::Duration64 => Some(T::Duration),
            Self::Interval => Some(T::Interval),
            _ => None,
        }
    }

    /// The name of the temporal family this identifier is a leaf of -
    /// `date`, `time`, `datetime`, `duration` or `interval` - and `None`
    /// outside [`DataTypeKind::Temporal`].
    ///
    /// ```
    /// use yggdryl::DataTypeId;
    ///
    /// assert_eq!(DataTypeId::Date64.temporal_family(), Some("date"));
    /// assert_eq!(DataTypeId::Duration32.temporal_family(), Some("duration"));
    /// assert_eq!(DataTypeId::Int64.temporal_family(), None);
    /// ```
    pub const fn temporal_family(self) -> Option<&'static str> {
        match self.temporal_kind() {
            Some(kind) => Some(kind.as_str()),
            None => None,
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
                | Self::FixedBinary
                | Self::SizedBinary
                | Self::FixedUtf8String
                | Self::SizedUtf8String
                | Self::FixedAsciiString
                | Self::SizedAsciiString
                | Self::FixedCp1252String
                | Self::SizedCp1252String
                | Self::Serie
                | Self::SerieView
                | Self::FixedSizeSerie
                | Self::LargeSerie
                | Self::LargeSerieView
                | Self::Struct
                | Self::Union
                | Self::Dictionary
                | Self::Decimal32
                | Self::Decimal64
                | Self::Decimal128
                | Self::Decimal256
                | Self::Map
                | Self::SortedMap
                | Self::RunEndEncoded
                | Self::Geometry
                | Self::Geography
        )
    }

    /// Return whether the variant is a signed or unsigned integer.
    pub const fn is_integer(self) -> bool {
        DataTypeKind::Integer.contains(self)
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
            Self::Serie
                | Self::SerieView
                | Self::FixedSizeSerie
                | Self::LargeSerie
                | Self::LargeSerieView
                | Self::Struct
                | Self::Union
                | Self::Map
                | Self::SortedMap
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
            Self::Ccy => Some(3),
            Self::MicCode => Some(4),
            Self::CfiCode => Some(6),
            Self::SedolCode => Some(7),
            Self::Side | Self::TimeInForce => Some(8),
            Self::CusipCode => Some(9),
            Self::State => Some(10),
            Self::IsinCode | Self::FIGICode => Some(12),
            Self::BloombergCode => Some(32),
            _ => None,
        }
    }
}

// Every identifier sits in one family's range, so `kind` never reaches its
// panic: a new identifier placed past every range fails to compile.
const _: () = {
    let mut index = 0;
    while index < DataTypeId::ALL.len() {
        assert!(
            DataTypeKind::of_u8(DataTypeId::ALL[index].as_u8()).is_some(),
            "an identifier sits past every family's range"
        );
        index += 1;
    }
};

/// Every byte's identifier, built once from [`DataTypeId::ALL`].
const FROM_U8: [Option<DataTypeId>; 256] = {
    let mut table = [None; 256];
    let mut index = 0;
    while index < DataTypeId::ALL.len() {
        let id = DataTypeId::ALL[index];
        table[id.as_u8() as usize] = Some(id);
        index += 1;
    }
    table
};

impl DataTypeId {
    /// The names the serie family was spelled with before it took its own,
    /// each still read as the identifier it names.
    ///
    /// Written by nothing - [`Self::as_str`] spells every identifier - and
    /// read by every door that reads a datatype's name: [`FromStr`], the type
    /// grammar, the serde tags and both bindings, so a schema, a document or
    /// a pickle written before the rename still reads.
    ///
    /// ```
    /// use std::str::FromStr;
    ///
    /// use yggdryl::DataTypeId;
    ///
    /// assert_eq!(DataTypeId::from_str("large_list")?, DataTypeId::LargeSerie);
    /// assert_eq!(DataTypeId::LargeSerie.as_str(), "large_serie");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    pub const LEGACY_NAMES: [(&'static str, Self); 5] = [
        ("list", Self::Serie),
        ("list_view", Self::SerieView),
        ("fixed_size_list", Self::FixedSizeSerie),
        ("large_list", Self::LargeSerie),
        ("large_list_view", Self::LargeSerieView),
    ];

    /// The identifier one of [`Self::LEGACY_NAMES`] names, ignoring case, or
    /// nothing for any other word.
    pub fn from_legacy_name(value: &str) -> Option<Self> {
        Self::LEGACY_NAMES
            .into_iter()
            .find(|(name, _)| value.eq_ignore_ascii_case(name))
            .map(|(_, id)| id)
    }
}

impl FromStr for DataTypeId {
    type Err = Error;

    /// An identifier's name, or one of its [legacy names](Self::LEGACY_NAMES),
    /// ignoring case.
    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|id| value.eq_ignore_ascii_case(id.as_str()))
            .or_else(|| Self::from_legacy_name(value))
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
