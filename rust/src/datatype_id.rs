use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DataTypeId(u8);

#[allow(non_upper_case_globals)]
impl DataTypeId {
    // Null: 0x00..0x07
    /// Null values.
    pub const Null: Self = Self(0x00);
    // Boolean: 0x08..0x0f
    /// Boolean values.
    pub const Boolean: Self = Self(0x09);
    // Integer: 0x10..0x1f
    /// Signed 8-bit integers.
    pub const Int8: Self = Self(0x11);
    /// Signed 16-bit integers.
    pub const Int16: Self = Self(0x12);
    /// Signed 32-bit integers.
    pub const Int32: Self = Self(0x13);
    /// Signed 64-bit integers.
    pub const Int64: Self = Self(0x14);
    /// Signed 128-bit integers.
    ///
    /// Arrow has no 128-bit integer layout, so no [`crate::DataType`] answers
    /// this identifier. It names the width [`crate::integer::Int128`] stores and
    /// the canonical identity a negative integer of any width carries into
    /// [`crate::Scalar::write_bytes`].
    pub const Int128: Self = Self(0x15);
    /// Unsigned 8-bit integers.
    pub const UInt8: Self = Self(0x19);
    /// Unsigned 16-bit integers.
    pub const UInt16: Self = Self(0x1a);
    /// Unsigned 32-bit integers.
    pub const UInt32: Self = Self(0x1b);
    /// Unsigned 64-bit integers.
    pub const UInt64: Self = Self(0x1c);
    /// Unsigned 128-bit integers.
    ///
    /// The unsigned half of the pair [`Self::Int128`] documents.
    pub const UInt128: Self = Self(0x1d);
    // Floating: 0x20..0x27
    /// IEEE 16-bit floating point.
    pub const Float16: Self = Self(0x21);
    /// IEEE 32-bit floating point.
    pub const Float32: Self = Self(0x22);
    /// IEEE 64-bit floating point.
    pub const Float64: Self = Self(0x23);
    // Decimal: 0x28..0x2f
    /// Exact decimal backed by 32 bits.
    pub const Decimal32: Self = Self(0x29);
    /// Exact decimal backed by 64 bits.
    pub const Decimal64: Self = Self(0x2a);
    /// Exact decimal backed by 128 bits.
    pub const Decimal128: Self = Self(0x2b);
    /// Exact decimal backed by 256 bits.
    pub const Decimal256: Self = Self(0x2c);
    /// The fixed `decimal128(38, 18)` leaf: eighteen fractional digits, always.
    pub const Decimal: Self = Self(0x2d);
    /// The fixed `decimal256(76, 18)` leaf: the wide twin of `decimal`.
    pub const BigDecimal: Self = Self(0x2e);
    // Temporal: 0x30..0x3f
    /// A 64-bit datetime with a resolution and explicit timezone marker.
    pub const DateTime64: Self = Self(0x31);
    /// Days since the Unix epoch.
    pub const Date32: Self = Self(0x32);
    /// Milliseconds since the Unix epoch representing whole days.
    pub const Date64: Self = Self(0x33);
    /// 32-bit time of day.
    pub const Time32: Self = Self(0x34);
    /// 64-bit time of day.
    pub const Time64: Self = Self(0x35);
    /// 32-bit elapsed time.
    pub const Duration32: Self = Self(0x36);
    /// 64-bit elapsed time.
    pub const Duration64: Self = Self(0x37);
    /// Calendar interval.
    pub const Interval: Self = Self(0x38);
    // Bytes: 0x40..0x4f
    /// Bytes with 32-bit offsets, under any bound.
    pub const Binary: Self = Self(0x41);
    /// Bytes with 64-bit offsets.
    pub const LargeBinary: Self = Self(0x42);
    /// Bytes in the view layout.
    pub const BinaryView: Self = Self(0x43);
    /// The viewed byte layout over 64-bit offsets.
    pub const LargeBinaryView: Self = Self(0x44);
    /// Bytes of one fixed width.
    pub const FixedBinary: Self = Self(0x45);
    /// Bytes under a declared maximum.
    pub const SizedBinary: Self = Self(0x46);
    // Text: 0x50..0x69
    /// Any length of UTF-8 with 32-bit offsets.
    pub const Utf8String: Self = Self(0x51);
    /// UTF-8 with 64-bit offsets.
    pub const LargeUtf8String: Self = Self(0x52);
    /// UTF-8 in the view layout.
    pub const Utf8StringView: Self = Self(0x53);
    /// UTF-8 in the view layout, declared large.
    pub const LargeUtf8StringView: Self = Self(0x54);
    /// UTF-8 of one fixed, padded byte width.
    pub const FixedUtf8String: Self = Self(0x55);
    /// UTF-8 under a declared maximum.
    pub const SizedUtf8String: Self = Self(0x56);
    /// Any length of US-ASCII with 32-bit offsets.
    pub const AsciiString: Self = Self(0x57);
    /// US-ASCII with 64-bit offsets.
    pub const LargeAsciiString: Self = Self(0x58);
    /// US-ASCII in the view layout.
    pub const AsciiStringView: Self = Self(0x59);
    /// US-ASCII in the view layout, declared large.
    pub const LargeAsciiStringView: Self = Self(0x5a);
    /// US-ASCII of one fixed, padded byte width.
    pub const FixedAsciiString: Self = Self(0x5b);
    /// US-ASCII under a declared maximum.
    pub const SizedAsciiString: Self = Self(0x5c);
    /// Any length of windows-1252 with 32-bit offsets.
    pub const Cp1252String: Self = Self(0x5d);
    /// Windows-1252 with 64-bit offsets.
    pub const LargeCp1252String: Self = Self(0x5e);
    /// Windows-1252 in the view layout.
    pub const Cp1252StringView: Self = Self(0x5f);
    /// Windows-1252 in the view layout, declared large.
    pub const LargeCp1252StringView: Self = Self(0x60);
    /// Windows-1252 of one fixed, padded byte width.
    pub const FixedCp1252String: Self = Self(0x61);
    /// Windows-1252 under a declared maximum.
    pub const SizedCp1252String: Self = Self(0x62);
    /// A canonical software or protocol version - a sixteen-bit major and
    /// minor and an optional text patch - ordered by its numbers, not its text.
    pub const Version: Self = Self(0x63);
    /// A validated, canonical location.
    pub const Url: Self = Self(0x64);
    /// A validated, canonical resource name.
    pub const Urn: Self = Self(0x65);
    /// A canonical time zone name, a fixed offset, or the zone-free marker.
    pub const Timezone: Self = Self(0x66);
    /// A validated, canonical MIME type.
    pub const MimeType: Self = Self(0x67);
    /// A MIME type with its charset and content codings.
    pub const MediaType: Self = Self(0x68);
    // Code: 0x6a..0x7f, the registered codes - the core's own, flat variants
    // of every root enum; 0x70 is spare and 0x75..=0x77 retired.
    /// ISO 17442: a legal entity identifier, twenty ASCII bytes closed by
    /// two ISO 7064 MOD 97-10 check digits.
    pub const Lei: Self = Self(0x6b);
    /// ISO 9362: a business identifier code, eight or eleven ASCII bytes.
    pub const Bic: Self = Self(0x6c);
    /// ISO 20275: an entity legal form code, four ASCII bytes.
    pub const Elf: Self = Self(0x6d);
    /// ISO 24165: a digital token identifier, nine ASCII bytes closed by an
    /// ISO 7064 MOD 31,30 check character.
    pub const Dti: Self = Self(0x6e);
    /// ISO 18774: a financial instrument short name, an issuer and an
    /// instrument description, at most thirty-five ASCII bytes.
    pub const Fisn: Self = Self(0x6f);
    /// ISO 3166-1 alpha-2: a country code, two ASCII bytes.
    pub const Country: Self = Self(0x71);
    /// A currency code: ISO 4217's three letters or a digital-asset ticker,
    /// at most eight ASCII bytes.
    pub const Ccy: Self = Self(0x72);
    /// ISO 10383: a market identifier code, four ASCII bytes.
    pub const Mic: Self = Self(0x73);
    /// ISO 10962: a classification of financial instruments, six ASCII bytes.
    pub const Cfi: Self = Self(0x74);
    /// ISO 6166: a securities identification number, twelve ASCII bytes.
    pub const Isin: Self = Self(0x78);
    /// CUSIP: a North American securities identifier, nine ASCII bytes.
    pub const Cusip: Self = Self(0x79);
    /// SEDOL: a London Stock Exchange securities identifier, seven ASCII
    /// bytes.
    pub const Sedol: Self = Self(0x7a);
    /// A Bloomberg identifier: ticker, market and yellow key, up to thirty-two
    /// ASCII bytes.
    pub const Bbg: Self = Self(0x7b);
    /// ANSI X9.145 Financial Instrument Global Identifier, twelve ASCII bytes.
    pub const Figi: Self = Self(0x7c);
    /// The unit a quantity is stated in - FIX's `UnitOfMeasure(996)` - up to
    /// thirty-two ASCII bytes.
    pub const Unit: Self = Self(0x7d);
    /// A Refinitiv Identification Code: a ticker and an exchange mnemonic,
    /// up to thirty-two ASCII bytes.
    pub const Ric: Self = Self(0x7e);
    /// ISO 4217 currency pair: `CCY/CCY`, seven ASCII bytes.
    pub const Forex: Self = Self(0x7f);
    // Uuid: 0x80..0x8f
    /// One 128-bit universally unique identifier.
    pub const Uuid: Self = Self(0x81);
    // Nested: 0x90..0xaf
    /// A serie of items behind 32-bit offsets.
    pub const Serie: Self = Self(0x91);
    /// A serie of items behind 64-bit offsets.
    pub const LargeSerie: Self = Self(0x92);
    /// A serie of items behind 32-bit offsets and sizes.
    pub const SerieView: Self = Self(0x93);
    /// A serie of items behind 64-bit offsets and sizes.
    pub const LargeSerieView: Self = Self(0x94);
    /// A serie of exactly one length of items.
    pub const FixedSizeSerie: Self = Self(0x95);
    /// Ordered struct fields.
    pub const Struct: Self = Self(0x96);
    /// Arrow map entries.
    pub const Map: Self = Self(0x97);
    /// Arrow map entries whose keys are ordered within each row.
    pub const SortedMap: Self = Self(0x98);
    /// Tagged union fields.
    pub const Union: Self = Self(0x99);
    /// Dictionary-encoded values.
    pub const Dictionary: Self = Self(0x9a);
    /// Run-end encoded values.
    pub const RunEndEncoded: Self = Self(0x9b);
    /// Self-describing semi-structured values.
    pub const Variant: Self = Self(0x9c);
    // Geospatial: 0xb0..0xbf
    /// Geospatial features on a planar coordinate system.
    pub const Geometry: Self = Self(0xb1);
    /// Geospatial features on the surface of a sphere or spheroid.
    pub const Geography: Self = Self(0xb2);
    // Enum: 0xc0..0xcf
    /// What state one thing is in: a lifecycle-sorted enum, stored as the
    /// `uint16` code of its member.
    pub const State: Self = Self(0xc1);

    /// Every identifier of the core's own datatypes, in canonical
    /// declaration order - the seventeen registered codes among them, after
    /// the text family - what [`Self::all`] opens with before the
    /// registered kinds are spliced in after the core's own enum.
    pub const ALL: [Self; 92] = [
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
        Self::Decimal,
        Self::BigDecimal,
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
        Self::Lei,
        Self::Bic,
        Self::Elf,
        Self::Dti,
        Self::Fisn,
        Self::Country,
        Self::Ccy,
        Self::Mic,
        Self::Cfi,
        Self::Isin,
        Self::Cusip,
        Self::Sedol,
        Self::Bbg,
        Self::Figi,
        Self::Unit,
        Self::Ric,
        Self::Forex,
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
        Self::State,
    ];

    /// The identifier a registered kind states: `byte` in the Enum family's
    /// range, which is what a claim may take.
    ///
    /// Refused at compile time outside that range, so a kind's `const ID`
    /// is never a byte another family owns; whether the byte is free is the
    /// register's question, asked at the claim.
    ///
    /// ```
    /// use yggdryl::{DataTypeId, DataTypeKind};
    ///
    /// let id = DataTypeId::market(0xc7);
    /// assert_eq!(id.as_u8(), 0xc7);
    /// assert_eq!(id.kind(), DataTypeKind::Enum);
    /// ```
    ///
    /// # Panics
    ///
    /// For a byte outside `0xc0..=0xcf`: at compile time where a kind's
    /// `const ID` states it, at run time where a caller computes one.
    #[must_use]
    pub const fn market(byte: u8) -> Self {
        assert!(
            byte >= DataTypeKind::Enum.id() && byte <= DataTypeKind::Enum.last(),
            "a registered kind's byte sits in the enum family's range"
        );
        Self(byte)
    }

    /// Every identifier a value can carry, in the canonical order: the
    /// core's own ([`Self::ALL`]) with every registered kind spliced in
    /// after the core's own enum, in byte order. What a binding lists its
    /// datatype names from.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let claimed = crate::market::kinds();
        let mut all = Vec::with_capacity(Self::ALL.len() + claimed.len());
        for id in Self::ALL {
            all.push(id);
            if id.kind() == DataTypeKind::Enum {
                all.extend(claimed.iter().map(|kind| kind.id));
            }
        }
        all
    }

    /// The canonical lowercase name of a core identifier, `None` for a
    /// byte the core does not own: a registered kind's name is the kind's,
    /// which [`Self::as_str`] reads off the register.
    #[must_use]
    pub const fn core_name(self) -> Option<&'static str> {
        match self {
            Self::Null => Some("null"),
            Self::Boolean => Some("boolean"),
            Self::Int8 => Some("int8"),
            Self::Int16 => Some("int16"),
            Self::Int32 => Some("int32"),
            Self::Int64 => Some("int64"),
            Self::Int128 => Some("int128"),
            Self::UInt8 => Some("uint8"),
            Self::UInt16 => Some("uint16"),
            Self::UInt32 => Some("uint32"),
            Self::UInt64 => Some("uint64"),
            Self::UInt128 => Some("uint128"),
            Self::Float16 => Some("float16"),
            Self::Float32 => Some("float32"),
            Self::Float64 => Some("float64"),
            Self::Decimal32 => Some("decimal32"),
            Self::Decimal64 => Some("decimal64"),
            Self::Decimal128 => Some("decimal128"),
            Self::Decimal256 => Some("decimal256"),
            Self::Decimal => Some("decimal"),
            Self::BigDecimal => Some("bigdecimal"),
            Self::DateTime64 => Some("datetime64"),
            Self::Date32 => Some("date32"),
            Self::Date64 => Some("date64"),
            Self::Time32 => Some("time32"),
            Self::Time64 => Some("time64"),
            Self::Duration32 => Some("duration32"),
            Self::Duration64 => Some("duration64"),
            Self::Interval => Some("interval"),
            Self::Binary => Some("binary"),
            Self::LargeBinary => Some("large_binary"),
            Self::BinaryView => Some("binary_view"),
            Self::LargeBinaryView => Some("large_binary_view"),
            Self::FixedBinary => Some("fixed_binary"),
            Self::SizedBinary => Some("sized_binary"),
            Self::Utf8String => Some("utf8"),
            Self::LargeUtf8String => Some("large_utf8"),
            Self::Utf8StringView => Some("utf8_view"),
            Self::LargeUtf8StringView => Some("large_utf8_view"),
            Self::FixedUtf8String => Some("fixed_utf8"),
            Self::SizedUtf8String => Some("sized_utf8"),
            Self::AsciiString => Some("ascii"),
            Self::LargeAsciiString => Some("large_ascii"),
            Self::AsciiStringView => Some("ascii_view"),
            Self::LargeAsciiStringView => Some("large_ascii_view"),
            Self::FixedAsciiString => Some("fixed_ascii"),
            Self::SizedAsciiString => Some("sized_ascii"),
            Self::Cp1252String => Some("cp1252"),
            Self::LargeCp1252String => Some("large_cp1252"),
            Self::Cp1252StringView => Some("cp1252_view"),
            Self::LargeCp1252StringView => Some("large_cp1252_view"),
            Self::FixedCp1252String => Some("fixed_cp1252"),
            Self::SizedCp1252String => Some("sized_cp1252"),
            Self::Version => Some("version"),
            Self::Url => Some("url"),
            Self::Urn => Some("urn"),
            Self::Timezone => Some("timezone"),
            Self::MimeType => Some("mimetype"),
            Self::MediaType => Some("mediatype"),
            Self::Lei => Some("lei"),
            Self::Bic => Some("bic"),
            Self::Elf => Some("elf"),
            Self::Dti => Some("dti"),
            Self::Fisn => Some("fisn"),
            Self::Country => Some("country"),
            Self::Ccy => Some("ccy"),
            Self::Mic => Some("mic"),
            Self::Cfi => Some("cfi"),
            Self::Isin => Some("isin"),
            Self::Cusip => Some("cusip"),
            Self::Sedol => Some("sedol"),
            Self::Bbg => Some("bbg"),
            Self::Figi => Some("figi"),
            Self::Unit => Some("unit"),
            Self::Ric => Some("ric"),
            Self::Forex => Some("forex"),
            Self::Uuid => Some("uuid"),
            Self::Serie => Some("serie"),
            Self::LargeSerie => Some("large_serie"),
            Self::SerieView => Some("serie_view"),
            Self::LargeSerieView => Some("large_serie_view"),
            Self::FixedSizeSerie => Some("fixed_size_serie"),
            Self::Struct => Some("struct"),
            Self::Map => Some("map"),
            Self::SortedMap => Some("sorted_map"),
            Self::Union => Some("union"),
            Self::Dictionary => Some("dictionary"),
            Self::RunEndEncoded => Some("run_end_encoded"),
            Self::Variant => Some("variant"),
            Self::Geometry => Some("geometry"),
            Self::Geography => Some("geography"),
            Self::State => Some("state"),
            _ => None,
        }
    }

    /// Whether this is one of the core's own identifiers.
    #[must_use]
    pub const fn is_core(self) -> bool {
        self.core_name().is_some()
    }

    /// The canonical name of a core identifier, in a `const` context: what
    /// a core datatype's marker and family name read at compile time.
    ///
    /// # Panics
    ///
    /// At compile time for a byte the core does not own: a registered
    /// kind's name is read at run time through [`Self::as_str`].
    #[must_use]
    pub(crate) const fn core_str(self) -> &'static str {
        match self.core_name() {
            Some(name) => name,
            None => panic!("a registered kind's name is read at run time"),
        }
    }

    /// Whether this identifier is a registered kind's: a byte the core does
    /// not own that a claim holds.
    #[must_use]
    pub fn is_registered(self) -> bool {
        !self.is_core() && crate::market::kind_of(self).is_some()
    }

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

    /// Return the canonical lowercase name without allocating: a core
    /// identifier's own, a registered kind's, and `unregistered` for a byte
    /// no claim holds, which no door hands out.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self.core_name() {
            Some(name) => name,
            None => crate::market::kind_of(self).map_or("unregistered", |kind| kind.name),
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
        self.0
    }

    /// The identifier one byte names, or nothing for a byte no variant
    /// states: a family's own number, a placeholder in a family's range, or
    /// a byte past every family.
    pub fn from_u8(byte: u8) -> Option<Self> {
        match FROM_U8[byte as usize] {
            Some(id) => Some(id),
            None => crate::market::kind_of(Self(byte)).map(|kind| kind.id),
        }
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

    /// The Arrow extension name a datatype of this identifier rides under:
    /// the one owner of which datatype states itself to Arrow by a name, and
    /// `None` for one Arrow states alone - plain UTF-8 in its three layouts,
    /// the four byte layouts and a fixed width, every number, temporal and
    /// nested shape.
    ///
    /// ```
    /// use yggdryl::DataTypeId;
    ///
    /// assert_eq!(DataTypeId::Ccy.arrow_extension_name(), Some("yggdryl.ccy"));
    /// assert_eq!(DataTypeId::Uuid.arrow_extension_name(), Some("arrow.uuid"));
    /// assert_eq!(DataTypeId::FixedAsciiString.arrow_extension_name(), Some("yggdryl.string"));
    /// assert_eq!(DataTypeId::Utf8String.arrow_extension_name(), None);
    /// ```
    pub fn arrow_extension_name(self) -> Option<&'static str> {
        if self.is_core() {
            return self.core_arrow_extension_name();
        }
        crate::market::kind_of(self).map(|kind| kind.extension_name)
    }

    /// The Arrow extension name of a core identifier, in a `const`
    /// context; `None` for one Arrow states alone and for every byte the
    /// core does not own, whose name is the kind's.
    #[must_use]
    pub(crate) const fn core_arrow_extension_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Uuid => crate::UUID_EXTENSION_NAME,
            Self::Variant => crate::VARIANT_EXTENSION_NAME,
            Self::Geometry | Self::Geography => crate::GEOARROW_WKB_EXTENSION_NAME,
            // A maximum and the second view width are what Arrow cannot
            // state about bytes; the other four layouts are its own.
            Self::SizedBinary | Self::LargeBinaryView => crate::BYTES_EXTENSION_NAME,
            // Plain UTF-8 is Arrow's own; every other leaf states a charset,
            // a bound or the second view width.
            Self::LargeUtf8StringView
            | Self::FixedUtf8String
            | Self::SizedUtf8String
            | Self::AsciiString
            | Self::LargeAsciiString
            | Self::AsciiStringView
            | Self::LargeAsciiStringView
            | Self::FixedAsciiString
            | Self::SizedAsciiString
            | Self::Cp1252String
            | Self::LargeCp1252String
            | Self::Cp1252StringView
            | Self::LargeCp1252StringView
            | Self::FixedCp1252String
            | Self::SizedCp1252String => crate::STRING_EXTENSION_NAME,
            Self::Decimal => crate::DECIMAL_EXTENSION_NAME,
            Self::BigDecimal => crate::BIGDECIMAL_EXTENSION_NAME,
            Self::Version => crate::VERSION_EXTENSION_NAME,
            Self::Url => crate::URL_EXTENSION_NAME,
            Self::Urn => crate::URN_EXTENSION_NAME,
            Self::Timezone => crate::TIMEZONE_EXTENSION_NAME,
            Self::MimeType => crate::MIMETYPE_EXTENSION_NAME,
            Self::MediaType => crate::MEDIATYPE_EXTENSION_NAME,
            Self::Lei => crate::LEI_EXTENSION_NAME,
            Self::Bic => crate::BIC_EXTENSION_NAME,
            Self::Elf => crate::ELF_EXTENSION_NAME,
            Self::Dti => crate::DTI_EXTENSION_NAME,
            Self::Fisn => crate::FISN_EXTENSION_NAME,
            Self::Country => crate::COUNTRY_EXTENSION_NAME,
            Self::Ccy => crate::CCY_EXTENSION_NAME,
            Self::Mic => crate::MIC_EXTENSION_NAME,
            Self::Cfi => crate::CFI_EXTENSION_NAME,
            Self::Isin => crate::ISIN_EXTENSION_NAME,
            Self::Cusip => crate::CUSIP_EXTENSION_NAME,
            Self::Sedol => crate::SEDOL_EXTENSION_NAME,
            Self::Bbg => crate::BBG_EXTENSION_NAME,
            Self::Figi => crate::FIGI_EXTENSION_NAME,
            Self::Unit => crate::UNIT_EXTENSION_NAME,
            Self::Ric => crate::RIC_EXTENSION_NAME,
            Self::Forex => crate::FOREX_EXTENSION_NAME,
            Self::State => crate::State::EXTENSION_NAME,
            _ => return None,
        })
    }

    /// Every distinct Arrow extension name [`Self::arrow_extension_name`]
    /// answers, in the order of [`Self::ALL`]: what a binding registers its
    /// runtime's extension types from.
    ///
    /// ```
    /// use yggdryl::DataTypeId;
    ///
    /// let names: Vec<&str> = DataTypeId::arrow_extension_names().collect();
    /// assert!(names.contains(&"yggdryl.state"));
    /// assert_eq!(names.iter().filter(|name| **name == "yggdryl.string").count(), 1);
    /// ```
    pub fn arrow_extension_names() -> impl Iterator<Item = &'static str> {
        // One name's identifiers are neighbours in `ALL` once the ones naming
        // none are passed, so a repeat is always the name just answered.
        let mut last = None;
        Self::all()
            .into_iter()
            .filter_map(Self::arrow_extension_name)
            .filter(move |name| last.replace(*name) != Some(*name))
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
            Self::Int128 | Self::UInt128 | Self::Decimal128 | Self::Decimal | Self::Uuid => {
                Some(16)
            }
            Self::Decimal256 | Self::BigDecimal => Some(32),
            _ => None,
        }
    }

    /// Return the most bytes one registered code's value may be.
    ///
    /// The number each standard fixes: two for a country, four for a market
    /// identifier or an entity legal form, six for a classification, seven
    /// for a SEDOL or a currency pair, nine for a CUSIP or a digital token
    /// identifier, eleven for a business identifier code, twelve for an ISIN
    /// or a FIGI, twenty for a legal entity identifier, thirty-two for a
    /// Bloomberg identifier - a bound, because a ticker, a market and a
    /// yellow key have no fixed length between them - and for a unit and a
    /// RIC, which take the same bound, and thirty-five for a financial
    /// instrument short name, a bound for the same reason. A currency is a
    /// bound too: eight, ISO 4217's three letters or a digital-asset ticker
    /// past them. It is a maximum, not a layout - a code stores as the text
    /// it is - and it is what the value rule holds a
    /// cell to and what [`crate::DataType::ascii_packed`] pads into. Each
    /// number is its code file's own constant, read here.
    ///
    /// Every other variant returns `None`.
    pub const fn code_width(self) -> Option<usize> {
        match self {
            Self::Country => Some(crate::COUNTRY_WIDTH),
            Self::Ccy => Some(crate::CCY_WIDTH),
            Self::Mic => Some(crate::MIC_WIDTH),
            Self::Cfi => Some(crate::CFI_WIDTH),
            Self::Sedol => Some(crate::SEDOL_WIDTH),
            Self::Forex => Some(crate::FOREX_WIDTH),
            Self::Lei => Some(crate::LEI_WIDTH),
            Self::Bic => Some(crate::BIC_WIDTH),
            Self::Elf => Some(crate::ELF_WIDTH),
            Self::Dti => Some(crate::DTI_WIDTH),
            Self::Fisn => Some(crate::FISN_WIDTH),
            Self::Cusip => Some(crate::CUSIP_WIDTH),
            Self::Isin => Some(crate::ISIN_WIDTH),
            Self::Figi => Some(crate::FIGI_WIDTH),
            Self::Bbg => Some(crate::BBG_WIDTH),
            Self::Unit => Some(crate::UNIT_WIDTH),
            Self::Ric => Some(crate::RIC_WIDTH),
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
            .or_else(|| {
                crate::market::kinds()
                    .into_iter()
                    .find(|kind| value.eq_ignore_ascii_case(kind.name))
                    .map(|kind| kind.id)
            })
            .ok_or_else(|| crate::market::unregistered(format_args!("{value}")))
    }
}

impl From<DataTypeId> for DataTypeKind {
    fn from(value: DataTypeId) -> Self {
        value.kind()
    }
}

impl fmt::Debug for DataTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
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
