//! The shared logical datatype enum and its cross-family value contract.

use std::cmp::Ordering;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::types::decimal::DecimalType;
use crate::types::uuid::UuidType;
use crate::types::enums::EnumType;
use crate::types::structure::StructureType;
use crate::types::mapping::MappingType;
use crate::types::sequence::SequenceType;
use crate::types::runend::RunEndEncodedType;
use crate::types::union::UnionFields;
use crate::{DataTypeId, DataTypeKind, Error, Field, Result, Scalar, TimeUnit, UnionMode};

use super::decimal::validate_decimal;
use super::geospatial::GeospatialParameters;
use crate::types::structure::cmp_fields;
use crate::types::enums::validate_dictionary_key;
use crate::types::structure::validate_fields;
use crate::types::mapping::validate_map_entries;
use crate::types::runend::validate_run_ends;
use crate::types::union::validate_union_fields;
use super::temporal::{validate_duration_unit, validate_time32_unit, validate_time64_unit};
use std::ops::Index;
use crate::types::typed::define_field_types;
/// An allocation-conscious logical datatype with complete Arrow 59.2 parity.
///
/// Scalar variants are inline. Nested children use `Arc`, so cloning a
/// datatype never allocates. Cache state belongs to [`Field`], not this value.
///
/// Parameterized variants remain public for ergonomic pattern matching and
/// Arrow parity. Caller-created values can therefore bypass constructors and
/// temporarily contain invalid parameters. Prefer validated constructors such
/// as [`Self::time`], [`Self::decimal`], and [`Self::map`]. Arrow
/// projection, structural serialization, and [`Self::validate`] reject every
/// invalid state before it crosses an interoperability boundary.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum DataType {
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
    /// A 64-bit datetime count with an explicit timezone marker.
    DateTime64 {
        /// The count's temporal resolution.
        unit: TimeUnit,
        /// An IANA zone, fixed offset, or [`crate::Timezone::NAIVE`].
        timezone: crate::Timezone,
    },
    /// Days since the Unix epoch.
    Date32,
    /// Milliseconds since the Unix epoch representing whole days.
    Date64,
    /// 32-bit time of day; seconds and milliseconds are valid.
    Time32(TimeUnit),
    /// 64-bit time of day; microseconds and nanoseconds are valid.
    Time64(TimeUnit),
    /// 32-bit elapsed-time count.
    Duration32(TimeUnit),
    /// 64-bit elapsed-time count.
    Duration64(TimeUnit),
    /// Calendar interval.
    Interval(TimeUnit),
    /// Bytes: one layout, one optional byte bound.
    ///
    /// Every byte column the crate has, `binary`, `varbinary(16)` and
    /// `fixed_binary(16)` alike; [`Self::bytes`] builds one and the
    /// sugar beside it - [`Self::binary`], [`Self::large_binary`],
    /// [`Self::binary_view`], [`Self::fixed_size_binary`] - names the common
    /// ones.
    Bytes(super::bytes::BytesType),
    /// A string: one layout, one charset, one optional byte bound.
    ///
    /// Every string the crate has, `utf8` and `ascii(n)` and
    /// `fixed_string(windows-1252,8)` alike; [`Self::string`] builds one and
    /// the sugar beside it - [`Self::utf8`], [`Self::large_utf8`],
    /// [`Self::utf8_view`], [`Self::ascii`], [`Self::fixed_utf8`],
    /// [`Self::fixed_ascii`] - names the common ones. The parameters ride
    /// inline: a layout, a charset and one bound are two bytes and a number,
    /// which is cheaper to carry than to point at.
    String(super::string::StringType),
    /// ISO 3166-1 alpha-2: a country code, two ASCII bytes.
    Country,
    /// ISO 4217: a currency code, three ASCII bytes.
    Currency,
    /// ISO 10383: a market identifier code, four ASCII bytes.
    Mic,
    /// ISO 10962: a classification of financial instruments, six ASCII bytes.
    Cfi,
    /// ISO 6166: a securities identification number, twelve ASCII bytes
    /// closed by a check digit.
    Isin,
    /// FIX's side of a trade, four ASCII bytes.
    Side,
    /// What state one thing is in, eight ASCII bytes.
    ///
    /// A rank character then a name, so the stored bytes sort from the first
    /// state to the terminal ones wherever they are sorted. One vocabulary
    /// over FIX's `OrdStatus` and `ExecType` and an ordinary scheduler's
    /// words, because they describe the same shape.
    State,
    /// How long an order stands, eight ASCII bytes.
    TimeInForce,
    /// Which way a captured line moved, four ASCII bytes: `SENT` or `RECV`.
    /// One 128-bit universally unique identifier: the whole uuid family.
    ///
    /// The leaf - whether the column admits every RFC 9562 version or only
    /// the one it names - is [`UuidType`]'s business, not this enum's. Every
    /// leaf is the same sixteen bytes, so a reader walking storage never asks
    /// which one it is.
    Uuid(UuidType),
    /// A canonical, numerically ordered software or protocol version.
    Version,
    /// A validated, canonical location, stored as its canonical text.
    Url,
    /// Many of one thing: one variant for the whole sequence family.
    ///
    /// The leaf - which offset width, whether it is a view, whether the
    /// length is fixed - is [`SequenceType`]'s business, not this enum's.
    /// Every leaf holds one item field, so a reader walking children asks
    /// [`SequenceType::item`] and never branches on the layout.
    Sequence(SequenceType),
    /// Named children, or the two-child pair: the whole structure family.
    ///
    /// The leaf - a struct's ordered children, or a mapping's key-value
    /// pair - is [`StructureType`]'s business, not this enum's.
    Structure(StructureType),
    /// Tagged union fields and layout mode.
    Union(UnionFields, UnionMode),
    /// A value stored as a code that stands for it: the whole enum family.
    ///
    /// The leaf - dictionary encoding today - is [`EnumType`]'s business,
    /// not this enum's.
    Enum(EnumType),
    /// An exact number at a fixed scale: the whole decimal family.
    ///
    /// The leaf - which backing integer holds the coefficient - is
    /// [`DecimalType`]'s business, not this enum's. Every leaf carries one
    /// precision and one scale, so a reader asks
    /// [`DecimalType::precision`] and [`DecimalType::scale`] and never
    /// branches on the width.
    Decimal(DecimalType),
    /// Keys to values: one variant for the whole mapping family.
    ///
    /// The leaf - Arrow's map today - is [`MappingType`]'s business, not this
    /// enum's. A second key-to-value layout joins the family there and no
    /// call site here learns a new variant.
    Mapping(MappingType),
    /// Run-end encoding child fields.
    RunEndEncoded(Arc<RunEndEncodedType>),
    /// Self-describing semi-structured values.
    ///
    /// A variant value is a [`crate::Scalar`] - a tree that declares its own
    /// types per value - so the type takes no parameters: shredding is a
    /// physical layout, not part of the logical type. Bare `variant` is this
    /// type; `variant(...)` with members stays the dense-union input sugar,
    /// and the parenthesis is what disambiguates.
    Variant,
    /// Planar geospatial features, carried as Well-Known Binary.
    Geometry(Arc<GeospatialParameters>),
    /// Geospatial features on a sphere or spheroid, carried as WKB.
    Geography(Arc<GeospatialParameters>),
    // Appended rather than grouped with the text datatypes: `Hash` is derived
    // here and a derived discriminant is what a stored digest of a schema is
    // over, so a variant inserted in the middle would move every one after it.
    /// A canonical time zone name, a fixed offset, or the zone-free marker,
    /// stored as its canonical text.
    Timezone,
    /// A validated, canonical MIME type, stored as its canonical text.
    MimeType,
    /// A MIME type with its charset and content codings, stored as the
    /// canonical text that spells all three.
    MediaType,
    // Appended after the text datatypes rather than beside `Isin` for the
    // same reason: the derived discriminant is what a stored digest of a
    // schema is over.
    /// CUSIP: a North American securities identifier, nine ASCII bytes
    /// closed by a check digit.
    Cusip,
    /// SEDOL: a London Stock Exchange securities identifier, seven ASCII
    /// bytes closed by a check digit.
    Sedol,
    Bloomberg,
}

impl DataType {
    /// The canonical value this datatype holds, from any value it accepts.
    ///
    /// This is the crate's one value contract in one call: the value is
    /// checked against the datatype and rewritten into the exact
    /// representation it declares - an integer narrowed to its width, a
    /// decimal restated at its scale, a temporal restated at its unit, an
    /// ASCII value trimmed of the padding a fixed slot adds. A value that
    /// already matches comes back untouched, so a correctly built value costs
    /// one walk and no allocation, and nothing downstream checks it again.
    ///
    /// A null is the null of this datatype: nullability belongs to the
    /// [`crate::Field`] holding the column, never to the value in it, so
    /// [`crate::Field::scalar`] is where a column refuses one.
    ///
    /// ```
    /// use yggdryl::{DataType, DataTypeId, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // Every spelling a code is written in becomes the exact code leaf.
    /// let currency = DataType::Currency.scalar("USD\0")?;
    /// assert_eq!(currency.id(), DataTypeId::Currency);
    /// assert_eq!(currency.as_str(), Some("USD"));
    /// // A decimal is restated at the scale the column declares.
    /// let decimal = DataType::decimal64(18, 8)?.scalar(Scalar::d128(10_125, 2))?;
    /// assert_eq!(decimal.id(), DataTypeId::Decimal64);
    /// assert_eq!(decimal, Scalar::d128(10_125_000_000, 8));
    /// // An integer narrows to the width it is declared at.
    /// assert_eq!(DataType::Int32.scalar(7_i64)?, Scalar::from(7_i32));
    /// assert_eq!(DataType::Int32.scalar(Scalar::Null)?, Scalar::Null);
    ///
    /// assert!(DataType::Currency.scalar("EURO").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the value's path when it is not a value this
    /// datatype accepts.
    pub fn scalar(&self, value: impl Into<crate::Scalar>) -> Result<crate::Scalar> {
        crate::types::dtype_scalar(self, value.into())
    }

    /// Returns a deterministic cross-process hash of the canonical display.
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(self)
    }

    /// Returns the parameter-free identifier of this variant.
    ///
    /// The identifier compares and hashes without touching nested state, so it
    /// is the cheap way to branch on a variant or name it for a binding.
    pub const fn id(&self) -> DataTypeId {
        match self {
            Self::Null => DataTypeId::Null,
            Self::Boolean => DataTypeId::Boolean,
            Self::Int8 => DataTypeId::Int8,
            Self::Int16 => DataTypeId::Int16,
            Self::Int32 => DataTypeId::Int32,
            Self::Int64 => DataTypeId::Int64,
            Self::UInt8 => DataTypeId::UInt8,
            Self::UInt16 => DataTypeId::UInt16,
            Self::UInt32 => DataTypeId::UInt32,
            Self::UInt64 => DataTypeId::UInt64,
            Self::Float16 => DataTypeId::Float16,
            Self::Float32 => DataTypeId::Float32,
            Self::Float64 => DataTypeId::Float64,
            Self::DateTime64 { .. } => DataTypeId::DateTime64,
            Self::Date32 => DataTypeId::Date32,
            Self::Date64 => DataTypeId::Date64,
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
            Self::Duration32(_) => DataTypeId::Duration32,
            Self::Duration64(_) => DataTypeId::Duration64,
            Self::Interval(_) => DataTypeId::Interval,
            Self::Bytes(parameters) => parameters.id(),
            Self::String(parameters) => parameters.layout().id(),
            Self::Country => DataTypeId::Country,
            Self::Currency => DataTypeId::Currency,
            Self::Mic => DataTypeId::Mic,
            Self::Cfi => DataTypeId::Cfi,
            Self::Isin => DataTypeId::Isin,
            Self::Cusip => DataTypeId::Cusip,
            Self::Sedol => DataTypeId::Sedol,
            Self::Bloomberg => DataTypeId::Bloomberg,
            Self::Side => DataTypeId::Side,
            Self::State => DataTypeId::State,
            Self::TimeInForce => DataTypeId::TimeInForce,
            Self::Uuid(family) => family.id(),
            Self::Version => DataTypeId::Version,
            Self::Url => DataTypeId::Url,
            Self::Timezone => DataTypeId::Timezone,
            Self::MimeType => DataTypeId::MimeType,
            Self::MediaType => DataTypeId::MediaType,
            Self::Sequence(SequenceType::List(_)) => DataTypeId::List,
            Self::Sequence(SequenceType::ListView(_)) => DataTypeId::ListView,
            Self::Sequence(SequenceType::FixedSizeList(..)) => DataTypeId::FixedSizeList,
            Self::Sequence(SequenceType::LargeList(_)) => DataTypeId::LargeList,
            Self::Sequence(SequenceType::LargeListView(_)) => DataTypeId::LargeListView,
            Self::Structure(_) => DataTypeId::Struct,
            Self::Union(..) => DataTypeId::Union,
            Self::Enum(EnumType::Dictionary(_)) => DataTypeId::Dictionary,
            Self::Decimal(DecimalType::Decimal32 { .. }) => DataTypeId::Decimal32,
            Self::Decimal(DecimalType::Decimal64 { .. }) => DataTypeId::Decimal64,
            Self::Decimal(DecimalType::Decimal128 { .. }) => DataTypeId::Decimal128,
            Self::Decimal(DecimalType::Decimal256 { .. }) => DataTypeId::Decimal256,
            Self::Mapping(MappingType::Map(_)) => DataTypeId::Map,
            Self::Mapping(MappingType::SortedMap(_)) => DataTypeId::SortedMap,
            Self::RunEndEncoded(_) => DataTypeId::RunEndEncoded,
            Self::Variant => DataTypeId::Variant,
            Self::Geometry(_) => DataTypeId::Geometry,
            Self::Geography(_) => DataTypeId::Geography,
        }
    }

    /// Returns the coarse family this datatype belongs to.
    pub const fn kind(&self) -> DataTypeKind {
        self.id().kind()
    }

    /// Returns a stable, parameter-independent variant name.
    pub const fn name(&self) -> &'static str {
        self.id().as_str()
    }

    /// Returns whether this type contains child fields or a nested value.
    ///
    /// Unlike [`DataTypeId::is_nested`], this resolves wrapper variants: a
    /// dictionary or run-end encoding is nested exactly when the value type it
    /// encodes is nested.
    pub fn is_nested(&self) -> bool {
        match self {
            Self::Enum(EnumType::Dictionary(dictionary)) => dictionary.value.is_nested(),
            Self::RunEndEncoded(run_end) => run_end.values.dtype().is_nested(),
            other => other.id().is_nested(),
        }
    }

    /// Builds a [`Field`] of this datatype.
    ///
    /// This reads in the order a schema is usually described - name, type,
    /// nullability - and lets a nested type be built inline without repeating
    /// it:
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let id = DataType::Int64.named_field("id", false);
    /// let tags = DataType::list(DataType::utf8().named_field("item", true))
    ///     .named_field("tags", true);
    ///
    /// assert_eq!(id.name(), "id");
    /// assert!(!id.is_nullable());
    /// assert!(tags.dtype().is_nested());
    /// # Ok(())
    /// # }
    /// ```
    pub fn named_field(self, name: impl Into<SmolStr>, nullable: bool) -> Field {
        Field::new(name, self, nullable)
    }

    /// Builds a nullable [`Field`] of this datatype.
    /// Convert one value into this datatype, exactly.
    ///
    /// This is the one scalar conversion of the crate - the same one a
    /// `cast(...)` in an expression runs and a literal is coerced with - so
    /// a value converts one way wherever it is asked to. Text is read the
    /// way the datatype reads it, a number widens or narrows when it fits,
    /// and anything that would lose a digit, a character or a second is
    /// refused rather than rounded.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::Int32.cast_scalar(&Scalar::from("42"))?, Scalar::from(42_i32));
    /// assert_eq!(DataType::utf8().cast_scalar(&Scalar::from(42_i64))?, Scalar::from("42"));
    /// assert!(DataType::Int8.cast_scalar(&Scalar::from(1_000_i64)).is_err());
    /// assert!(DataType::Int8.try_cast_scalar(&Scalar::from(1_000_i64)).is_null());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be held by this datatype
    /// without loss.
    pub fn cast_scalar(&self, value: &Scalar) -> Result<Scalar> {
        crate::expression::convert_scalar(self, value, true)
    }

    /// Convert one value into this datatype, or answer null when it cannot
    /// be: the best-effort reading of a cast.
    #[must_use]
    pub fn try_cast_scalar(&self, value: &Scalar) -> Scalar {
        crate::expression::convert_scalar(self, value, false).unwrap_or(Scalar::Null)
    }

    pub fn nullable_field(self, name: impl Into<SmolStr>) -> Field {
        self.named_field(name, true)
    }

    /// Builds a non-null [`Field`] of this datatype.
    pub fn required_field(self, name: impl Into<SmolStr>) -> Field {
        self.named_field(name, false)
    }

    /// Validates all parameters and nested children without projecting Arrow.
    ///
    /// This walk performs no allocation for a valid value. Arrow conversion
    /// repeats the checks while materializing foreign state so directly built
    /// enum variants cannot bypass an interop boundary.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::DateTime64 { unit, .. } if !unit.is_arrow_time() => {
                Err(invalid(self.name(), "unit must be a temporal resolution"))
            }
            Self::Time32(unit) => validate_time32_unit(*unit),
            Self::Time64(unit) => validate_time64_unit(*unit),
            Self::Duration32(unit) => validate_duration_unit("Duration32", *unit),
            Self::Duration64(unit) => validate_duration_unit("Duration64", *unit),
            Self::Interval(unit) if !unit.is_interval() => {
                Err(invalid("Interval", "unit must be an interval layout"))
            }
            // The variant is public, so a caller can build a fixed layout
            // without the width that makes it fixed. This is where it stops.
            Self::Bytes(parameters) => parameters.validate(),
            // The variant is public, so a caller can build a fixed string
            // the constructor would have refused for want of a width. This
            // is where it stops, before it reaches a boundary.
            Self::String(parameters) => parameters.validate(),
            Self::Sequence(SequenceType::List(field))
            | Self::Sequence(SequenceType::ListView(field))
            | Self::Sequence(SequenceType::LargeList(field))
            | Self::Sequence(SequenceType::LargeListView(field)) => field.validate(),
            Self::Sequence(SequenceType::FixedSizeList(field, length)) => {
                validate_non_negative("FixedSizeList", "length", *length)?;
                field.validate()
            }
            Self::Structure(fields) => validate_fields(fields.as_fields(), "Struct"),
            Self::Union(fields, _) => validate_union_fields(fields),
            Self::Enum(EnumType::Dictionary(dictionary)) => {
                validate_dictionary_key(&dictionary.key)?;
                dictionary.key.validate()?;
                dictionary.value.validate()
            }
            Self::Decimal(DecimalType::Decimal32 { precision, scale }) => {
                validate_decimal("Decimal32", *precision, *scale, 9)
            }
            Self::Decimal(DecimalType::Decimal64 { precision, scale }) => {
                validate_decimal("Decimal64", *precision, *scale, 18)
            }
            Self::Decimal(DecimalType::Decimal128 { precision, scale }) => {
                validate_decimal("Decimal128", *precision, *scale, 38)
            }
            Self::Decimal(DecimalType::Decimal256 { precision, scale }) => {
                validate_decimal("Decimal256", *precision, *scale, 76)
            }
            Self::Mapping(mapping) => {
                validate_map_entries(mapping.entries())?;
                mapping.entries().validate()
            }
            Self::RunEndEncoded(encoded) => {
                validate_run_ends(&encoded.run_ends)?;
                encoded.run_ends.validate()?;
                encoded.values.validate()
            }
            _ => Ok(()),
        }
    }
}

impl Ord for DataType {
    fn cmp(&self, other: &Self) -> Ordering {
        let rank = dtype_rank(self).cmp(&dtype_rank(other));
        if rank != Ordering::Equal {
            return rank;
        }

        use DataType as D;
        match (self, other) {
            (
                D::DateTime64 {
                    unit: left_unit,
                    timezone: left_zone,
                },
                D::DateTime64 {
                    unit: right_unit,
                    timezone: right_zone,
                },
            ) => (left_unit, left_zone).cmp(&(right_unit, right_zone)),
            (D::Time32(left), D::Time32(right))
            | (D::Time64(left), D::Time64(right))
            | (D::Duration32(left), D::Duration32(right))
            | (D::Duration64(left), D::Duration64(right)) => left.cmp(right),
            (D::Interval(left), D::Interval(right)) => left.cmp(right),
            (D::Bytes(left), D::Bytes(right)) => left.cmp(right),
            (D::Sequence(SequenceType::List(left)), D::Sequence(SequenceType::List(right)))
            | (D::Sequence(SequenceType::ListView(left)), D::Sequence(SequenceType::ListView(right)))
            | (D::Sequence(SequenceType::LargeList(left)), D::Sequence(SequenceType::LargeList(right)))
            | (D::Sequence(SequenceType::LargeListView(left)), D::Sequence(SequenceType::LargeListView(right))) => cmp_fields(left, right),
            (
                D::Sequence(SequenceType::FixedSizeList(left_field, left_size)),
                D::Sequence(SequenceType::FixedSizeList(right_field, right_size)),
            ) => cmp_fields(left_field, right_field).then_with(|| left_size.cmp(right_size)),
            (D::Structure(left), D::Structure(right)) => left.cmp(right),
            (D::Union(left_fields, left_mode), D::Union(right_fields, right_mode)) => left_mode
                .cmp(right_mode)
                .then_with(|| left_fields.cmp(right_fields)),
            (D::Enum(EnumType::Dictionary(left)), D::Enum(EnumType::Dictionary(right))) => left.cmp(right),
            (
                D::Decimal(DecimalType::Decimal32 {
                    precision: left_precision,
                    scale: left_scale,
                }),
                D::Decimal(DecimalType::Decimal32 {
                    precision: right_precision,
                    scale: right_scale,
                }),
            )
            | (
                D::Decimal(DecimalType::Decimal64 {
                    precision: left_precision,
                    scale: left_scale,
                }),
                D::Decimal(DecimalType::Decimal64 {
                    precision: right_precision,
                    scale: right_scale,
                }),
            )
            | (
                D::Decimal(DecimalType::Decimal128 {
                    precision: left_precision,
                    scale: left_scale,
                }),
                D::Decimal(DecimalType::Decimal128 {
                    precision: right_precision,
                    scale: right_scale,
                }),
            )
            | (
                D::Decimal(DecimalType::Decimal256 {
                    precision: left_precision,
                    scale: left_scale,
                }),
                D::Decimal(DecimalType::Decimal256 {
                    precision: right_precision,
                    scale: right_scale,
                }),
            ) => (left_precision, left_scale).cmp(&(right_precision, right_scale)),
            (D::String(left), D::String(right)) => left.cmp(right),
            (D::Mapping(left), D::Mapping(right)) => left.cmp(right),
            (D::RunEndEncoded(left), D::RunEndEncoded(right)) => left.cmp(right),
            (D::Geometry(left), D::Geometry(right)) | (D::Geography(left), D::Geography(right)) => {
                left.cmp(right)
            }
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for DataType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn dtype_rank(value: &DataType) -> u8 {
    match value {
        DataType::Null => 0,
        DataType::Boolean => 1,
        DataType::Int8 => 2,
        DataType::Int16 => 3,
        DataType::Int32 => 4,
        DataType::Int64 => 5,
        DataType::UInt8 => 6,
        DataType::UInt16 => 7,
        DataType::UInt32 => 8,
        DataType::UInt64 => 9,
        DataType::Float16 => 10,
        DataType::Float32 => 11,
        DataType::Float64 => 12,
        DataType::DateTime64 { .. } => 13,
        DataType::Date32 => 14,
        DataType::Date64 => 15,
        DataType::Time32(_) => 16,
        DataType::Time64(_) => 17,
        DataType::Duration32(_) => 18,
        DataType::Duration64(_) => 19,
        DataType::Interval(_) => 20,
        // The one byte variant takes the first of the four ranks the binary
        // variants it replaced held, so nothing after it moves.
        DataType::Bytes(_) => 21,
        // The one string variant takes the first of the five ranks the text
        // variants it replaced held, so nothing after it moves.
        DataType::String(_) => 25,
        DataType::Country => 30,
        DataType::Currency => 31,
        DataType::Mic => 32,
        DataType::Cfi => 33,
        DataType::Uuid(_) => 34,
        DataType::Version => 35,
        DataType::Sequence(SequenceType::List(_)) => 36,
        DataType::Sequence(SequenceType::ListView(_)) => 37,
        DataType::Sequence(SequenceType::FixedSizeList(..)) => 38,
        DataType::Sequence(SequenceType::LargeList(_)) => 39,
        DataType::Sequence(SequenceType::LargeListView(_)) => 40,
        DataType::Structure(_) => 41,
        DataType::Union(..) => 42,
        DataType::Enum(EnumType::Dictionary(_)) => 43,
        DataType::Decimal(DecimalType::Decimal32 { .. }) => 44,
        DataType::Decimal(DecimalType::Decimal64 { .. }) => 45,
        DataType::Decimal(DecimalType::Decimal128 { .. }) => 46,
        DataType::Decimal(DecimalType::Decimal256 { .. }) => 47,
        DataType::Mapping(_) => 48,
        DataType::RunEndEncoded(_) => 49,
        DataType::Variant => 50,
        DataType::Geometry(_) => 51,
        DataType::Geography(_) => 52,
        // Appended rather than grouped with the other codes so no existing
        // rank moves: this ordering is total, not a wire contract, and a
        // renumbering would change how every unrelated pair sorts.
        DataType::Side => 53,
        // 54 was `msgdirection`, since retired; the rank stays
        // unused so no other pair moves.
        DataType::State => 55,
        DataType::TimeInForce => 56,
        DataType::Url => 57,
        DataType::Isin => 58,
        DataType::Timezone => 59,
        DataType::MimeType => 60,
        DataType::MediaType => 61,
        DataType::Cusip => 62,
        DataType::Sedol => 63,
        DataType::Bloomberg => 64,
    }
}

pub(crate) fn invalid(kind: &'static str, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidDataType {
        kind,
        reason: reason.into(),
    }
}

pub(crate) fn validate_non_negative(
    kind: &'static str,
    parameter: &'static str,
    value: i32,
) -> Result<()> {
    if value < 0 {
        Err(invalid(
            kind,
            format_smolstr!("{parameter} must be non-negative: {value}"),
        ))
    } else {
        Ok(())
    }
}

// ------------------------------------------------------------------------
// Shared child collections and validated nested datatype construction.
// ------------------------------------------------------------------------

impl DataType {
    /// Creates the self-describing semi-structured Variant type.
    ///
    /// It takes no parameters: shredding is physical layout, while each value
    /// is the ordinary [`crate::Scalar`] tree. Parentheses distinguish the
    /// finite [`Self::dense_union`] input form in the grammar.
    #[must_use]
    pub const fn variant() -> Self {
        Self::Variant
    }

}

/// Subscripting a datatype reaches a nested **child**, never metadata.
///
/// The same semantic [`Field`] carries, so a caller walking a schema gets a
/// child from every node in the graph. The string is resolved by
/// [`DataType::get_field_by_path`] - an exact name first, a dotted path after -
/// and that method is the non-panicking form.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
/// assert_eq!(row["id"].dtype(), &DataType::Int64);
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this datatype has no child with that name - including when it is a dotted path.
impl Index<&str> for DataType {
    type Output = Field;

    fn index(&self, path: &str) -> &Self::Output {
        self.get_field_by_path(path)
            .unwrap_or_else(|| panic!("{path:?} is not a child of the datatype {self}"))
    }
}

/// Subscripting a datatype by position reaches that nested child.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let items = DataType::list(DataType::utf8().nullable_field("item"));
/// assert_eq!(items[0].name(), "item");
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this datatype has no child at that position.
impl Index<usize> for DataType {
    type Output = Field;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_field_at(index).unwrap_or_else(|| {
            panic!(
                "the datatype {self} has {} children, so position {index} is out of range",
                self.field_len()
            )
        })
    }
}



// The variant lives with the nested family: it is the self-describing
// sibling of the union whose grammar it shares (`variant` bare, `variant(...)`
// as dense-union sugar), and its Arrow storage is a struct of two binaries.
define_field_types!(VariantType, Variant);








// ------------------------------------------------------------------------
// Arrow projection and import: this enum names the family and nothing else.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::{DataType as ArrowDataType, ffi::FFI_ArrowSchema};
    use smol_str::format_smolstr;

    use super::{DataType, VariantType, invalid};
    use crate::types::boolean::{BooleanType, NullType};
    use crate::types::enums::EnumType;
    use crate::types::geospatial::geospatial_arrow_storage;
    use crate::types::media_type::MediaTypeType;
    use crate::types::mime_type::MimeTypeType;
    use crate::types::runend::RunEndEncodedType;
    use crate::types::sequence::SequenceType;
    use crate::types::structure::StructureType;
    use crate::types::timezone::TimezoneType;
    use crate::types::union::UnionFields;
    use crate::types::url::UrlType;
    use crate::types::uuid::UuidType;
    use crate::types::version::VersionType;
    use crate::types::{bytes, code, decimal, floating, integer, string, temporal};
    use crate::types::mapping::MappingType;
    use crate::{Error, Field, Result};
    use crate::types::DecimalType;

    impl DataType {
        /// Projects this datatype as the Arrow storage its family lays out.
        ///
        /// This enum owns no projection: each arm names the family that does,
        /// and the family answers for every leaf it holds - which is why a new
        /// leaf never reaches this match.
        ///
        /// An extension type projects as its *storage*: an Arrow datatype has
        /// nowhere to carry the `ARROW:extension:*` entries, which is what
        /// [`Self::arrow_extension`] answers and what the field projection and
        /// [`Self::into_arrow_datatype_ffi`] carry.
        ///
        /// # Errors
        ///
        /// Returns an error when the datatype states something Arrow cannot
        /// lay out: a unit a width does not carry, a precision wider than its
        /// backing integer, a negative fixed length.
        pub fn to_arrow_datatype(&self) -> Result<ArrowDataType> {
            use DataType as R;
            Ok(match self {
                R::Null => NullType::arrow_storage(),
                R::Boolean => BooleanType::arrow_storage(),
                R::Int8 | R::Int16 | R::Int32 | R::Int64 | R::UInt8 | R::UInt16 | R::UInt32
                | R::UInt64 => integer::arrow_storage(self)?,
                R::Float16 | R::Float32 | R::Float64 => floating::arrow_storage(self)?,
                R::DateTime64 { .. }
                | R::Date32
                | R::Date64
                | R::Time32(_)
                | R::Time64(_)
                | R::Duration32(_)
                | R::Duration64(_)
                | R::Interval(_) => temporal::arrow_storage(self)?,
                R::Bytes(parameters) => bytes::arrow_storage(*parameters)?,
                R::String(parameters) => string::arrow_storage(*parameters)?,
                R::Country
                | R::Currency
                | R::Mic
                | R::Cfi
                | R::Isin
                | R::Cusip
                | R::Sedol
                | R::Bloomberg
                | R::Side
                | R::State
                | R::TimeInForce => code::code_arrow_storage(self)?,
                R::Version => VersionType::arrow_storage(),
                R::Url => UrlType::arrow_storage(),
                R::Timezone => TimezoneType::arrow_storage(),
                R::MimeType => MimeTypeType::arrow_storage(),
                R::MediaType => MediaTypeType::arrow_storage(),
                R::Uuid(_) => UuidType::arrow_storage(),
                R::Decimal(DecimalType::Decimal32 { .. })
                | R::Decimal(DecimalType::Decimal64 { .. })
                | R::Decimal(DecimalType::Decimal128 { .. })
                | R::Decimal(DecimalType::Decimal256 { .. }) => decimal::arrow_storage(self)?,
                R::Sequence(sequence) => sequence.arrow_storage()?,
                R::Structure(structure) => structure.arrow_storage()?,
                R::Union(fields, mode) => fields.arrow_storage(*mode)?,
                R::Enum(enumeration) => enumeration.arrow_storage()?,
                R::Mapping(mapping) => mapping.arrow_storage()?,
                R::RunEndEncoded(encoded) => encoded.arrow_storage()?,
                R::Variant => VariantType::arrow_storage(),
                R::Geometry(_) | R::Geography(_) => geospatial_arrow_storage(),
            })
        }

        /// Consumes this datatype and returns its Arrow storage.
        ///
        /// The same projection [`Self::to_arrow_datatype`] makes, except that a
        /// family holding uniquely shared children consumes them rather than
        /// cloning a subtree it is about to drop.
        ///
        /// # Errors
        ///
        /// [`Self::to_arrow_datatype`] carries the rule.
        pub fn into_arrow_datatype(self) -> Result<ArrowDataType> {
            use DataType as R;
            match self {
                R::DateTime64 { .. } => temporal::into_arrow_storage(self),
                R::Sequence(sequence) => sequence.into_arrow_storage(),
                R::Structure(structure) => structure.into_arrow_storage(),
                R::Union(fields, mode) => fields.into_arrow_storage(mode),
                R::Enum(enumeration) => enumeration.into_arrow_storage(),
                R::Mapping(mapping) => mapping.into_arrow_storage(),
                R::RunEndEncoded(encoded) => RunEndEncodedType::into_arrow_storage(encoded),
                ref other => other.to_arrow_datatype(),
            }
        }

        /// Imports an Arrow datatype and validates every nested invariant.
        ///
        /// An Arrow datatype carries no metadata, so an extension type arrives
        /// as the storage it is written over: `fixed_binary(3)` and not
        /// `currency`, `binary` and not `geometry`. The identity lives on the
        /// field - [`Field::from_arrow_field`](crate::Field::from_arrow_field)
        /// reads it, and [`Self::into_arrow_datatype_ffi`] projects a node that
        /// carries it - so a schema round trip keeps every first-class datatype
        /// and only this bare pair answers storage.
        ///
        /// # Errors
        ///
        /// Returns an error when the storage states something this crate
        /// refuses, or when the nesting is deeper than the parser's limit.
        pub fn from_arrow_datatype(value: &ArrowDataType) -> Result<Self> {
            Self::from_arrow_datatype_at_depth(value, 0)
        }

        /// Imports Arrow state at an existing datatype nesting depth.
        ///
        /// Field import paths use this entry point to preserve one shared depth
        /// budget across alternating Arrow datatype and field nodes.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_datatype`] carries the rule.
        pub(crate) fn from_arrow_datatype_at_depth(
            value: &ArrowDataType,
            depth: usize,
        ) -> Result<Self> {
            check_arrow_import_depth(depth)?;
            let children = depth + 1;
            use ArrowDataType as A;
            match value {
                A::Null | A::Boolean => crate::types::boolean::from_arrow_storage(value),
                A::Int8 | A::Int16 | A::Int32 | A::Int64 | A::UInt8 | A::UInt16 | A::UInt32
                | A::UInt64 => integer::from_arrow_storage(value),
                A::Float16 | A::Float32 | A::Float64 => floating::from_arrow_storage(value),
                A::Timestamp(..)
                | A::Date32
                | A::Date64
                | A::Time32(_)
                | A::Time64(_)
                | A::Duration(_)
                | A::Interval(_) => temporal::from_arrow_storage(value),
                A::Binary | A::LargeBinary | A::BinaryView | A::FixedSizeBinary(_) => {
                    bytes::from_arrow_storage(value)
                }
                A::Utf8 | A::LargeUtf8 | A::Utf8View => string::from_arrow_storage(value),
                A::Decimal32(..) | A::Decimal64(..) | A::Decimal128(..) | A::Decimal256(..) => {
                    decimal::from_arrow_storage(value)
                }
                A::List(_)
                | A::ListView(_)
                | A::FixedSizeList(..)
                | A::LargeList(_)
                | A::LargeListView(_) => {
                    SequenceType::from_arrow_storage_at_depth(value, children)
                }
                A::Struct(fields) => StructureType::from_arrow_storage_at_depth(fields, children),
                A::Union(fields, mode) => {
                    UnionFields::from_arrow_storage_at_depth(fields, *mode, children)
                }
                A::Dictionary(key, values) => {
                    EnumType::from_arrow_storage_at_depth(key, values, children)
                }
                A::Map(entries, keys_sorted) => {
                    MappingType::from_arrow_storage_at_depth(entries, *keys_sorted, children)
                }
                A::RunEndEncoded(run_ends, values) => {
                    RunEndEncodedType::from_arrow_storage_at_depth(run_ends, values, children)
                }
            }
        }

        /// The same import, consuming the Arrow node rather than cloning it.
        ///
        /// Only the layouts whose children Arrow hands over by value differ
        /// here; everything else reads exactly as the borrowed walk does.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_datatype`] carries the rule.
        pub(crate) fn from_arrow_datatype_owned_at_depth(
            value: ArrowDataType,
            depth: usize,
        ) -> Result<Self> {
            check_arrow_import_depth(depth)?;
            let children = depth + 1;
            use ArrowDataType as A;
            match value {
                A::Timestamp(..) => temporal::from_arrow_storage_owned(value),
                A::List(_)
                | A::ListView(_)
                | A::FixedSizeList(..)
                | A::LargeList(_)
                | A::LargeListView(_) => {
                    SequenceType::from_arrow_storage_owned_at_depth(value, children)
                }
                A::Union(fields, mode) => {
                    UnionFields::from_arrow_storage_at_depth(&fields, mode, children)
                }
                A::Dictionary(key, values) => {
                    EnumType::from_arrow_storage_owned_at_depth(key, values, children)
                }
                A::Map(entries, keys_sorted) => {
                    MappingType::from_arrow_storage_owned_at_depth(entries, keys_sorted, children)
                }
                A::RunEndEncoded(run_ends, values) => {
                    RunEndEncodedType::from_arrow_storage_owned_at_depth(run_ends, values, children)
                }
                ref other => Self::from_arrow_datatype_at_depth(other, depth),
            }
        }

        /// Projects this datatype to an owned Arrow C Data Interface schema.
        ///
        /// This uses the same validated Arrow projection as
        /// [`Self::into_arrow_datatype`] and preserves datatype flags
        /// recursively, including sorted map keys. Arrow's generic
        /// Field-to-C-schema conversion overwrites those flags when adding
        /// field flags, so each nested family writes its own node here.
        ///
        /// A C schema is a field node, so unlike [`Self::into_arrow_datatype`]
        /// this keeps an extension identity - a code, a UUID, a version, a
        /// variant, a geospatial parameter set - including under a dictionary
        /// encoding, where the entries belong to the outer node.
        ///
        /// # Errors
        ///
        /// [`Self::to_arrow_datatype`] carries the rule.
        pub fn into_arrow_datatype_ffi(self) -> Result<FFI_ArrowSchema> {
            use DataType as R;
            let parts = match &self {
                R::Sequence(sequence) => sequence.arrow_ffi_parts()?,
                R::Structure(structure) => structure.arrow_ffi_parts()?,
                R::Union(fields, mode) => fields.arrow_ffi_parts(*mode)?,
                R::Enum(enumeration) => enumeration.arrow_ffi_parts()?,
                R::Mapping(mapping) => mapping.arrow_ffi_parts()?,
                R::RunEndEncoded(encoded) => encoded.arrow_ffi_parts()?,
                // The extension identity of an extension-typed variant is
                // metadata, and a C schema is a field, so the storage
                // projection carries the two `ARROW:extension:*` entries here.
                // `Field::into_arrow_field_ffi` merges the same entries with
                // the field's own metadata.
                //
                // Which datatypes those are is asked of the function that
                // answers it rather than re-listed: a list spelled here drifts
                // behind the families, and a datatype that falls through
                // crosses the C Data Interface as anonymous storage, which is
                // exactly what this arm exists to prevent.
                other => {
                    let storage = other.to_arrow_datatype()?;
                    let schema = FFI_ArrowSchema::try_from(&storage)?;
                    let Some((name, document)) = other.arrow_extension() else {
                        return Ok(schema);
                    };
                    return with_extension(schema, name, &document);
                }
            };
            let schema =
                FFI_ArrowSchema::try_new(&parts.format, parts.children, parts.dictionary)
                    .and_then(|schema| schema.with_flags(parts.flags))?;
            // A dictionary-encoded extension is still that extension, and the C
            // schema is the field that says so: the entries the plain
            // projection writes above ride here for the encoded shape too.
            let Some((name, document)) = self.arrow_extension() else {
                return Ok(schema);
            };
            with_extension(schema, name, &document)
        }

        /// The Arrow extension name and document this datatype rides under,
        /// `None` for every datatype that is Arrow's own.
        ///
        /// A dictionary answers what its values would. Arrow's `Dictionary`
        /// holds a bare datatype for its values rather than a field, so a
        /// dictionary-encoded currency has nowhere but the field itself to
        /// carry its identity - and a dictionary-encoded code is the ordinary
        /// case, not an edge one.
        #[must_use]
        pub fn arrow_extension(&self) -> Option<(&'static str, String)> {
            match self {
                Self::Enum(EnumType::Dictionary(dictionary)) => {
                    dictionary.value().arrow_extension()
                }
                Self::Variant => Some((crate::types::VARIANT_EXTENSION_NAME, String::new())),
                Self::Geometry(geospatial) | Self::Geography(geospatial) => Some((
                    crate::types::GEOARROW_WKB_EXTENSION_NAME,
                    geospatial.geoarrow_json(),
                )),
                // A charset, a length bound, and which of the two view layouts
                // this is: three facts Arrow has nowhere to put, so they ride
                // here when the string declares any of them; plain UTF-8 is
                // Arrow's own.
                Self::String(parameters) if string::needs_extension(*parameters) => Some((
                    crate::types::STRING_EXTENSION_NAME,
                    parameters.extension_json(),
                )),
                // A maximum on a variable layout is the one fact about bytes
                // Arrow has nowhere to put; the four layouts and a fixed width
                // are its own.
                Self::Bytes(parameters) if bytes::needs_extension(*parameters) => Some((
                    crate::types::BYTES_EXTENSION_NAME,
                    parameters.extension_json(),
                )),
                // Every identifier is Arrow's own sixteen bytes; only a
                // version is a fact Arrow has nowhere to put.
                Self::Uuid(UuidType::Uuid) => {
                    Some((crate::types::UUID_EXTENSION_NAME, String::new()))
                }
                Self::Uuid(family) => Some((
                    crate::types::UUID_VERSION_EXTENSION_NAME,
                    family.extension_json(),
                )),
                Self::Version => Some((crate::types::VERSION_EXTENSION_NAME, String::new())),
                Self::Url => Some((crate::types::URL_EXTENSION_NAME, String::new())),
                Self::Timezone => Some((crate::types::TIMEZONE_EXTENSION_NAME, String::new())),
                Self::MimeType => Some((crate::types::MIMETYPE_EXTENSION_NAME, String::new())),
                Self::MediaType => Some((crate::types::MEDIATYPE_EXTENSION_NAME, String::new())),
                // A code carries its own name, so the identity survives Arrow:
                // three bytes under `yggdryl.currency` read back a currency.
                code => crate::types::code_extension_name(code).map(|name| (name, String::new())),
            }
        }

        /// Projects this Struct datatype as an Arrow schema.
        ///
        /// A schema is the columns of a struct, so this is the same projection
        /// a non-null Struct [`Field`] makes, without a name or metadata.
        ///
        /// # Errors
        ///
        /// Returns an error unless this is a bounded Struct datatype.
        pub fn into_arrow_schema(self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
            Field::new("row", self, false).into_arrow_schema()
        }

        /// Materializes [`DataType::default_value`] as an exact one-row array.
        ///
        /// The bounded core default planner selects the value, so
        /// [`DataType::Null`] and transparent logical wrappers with a null-only
        /// canonical default materialize as logical null; every other datatype
        /// materializes its present zero/empty default.
        ///
        /// # Errors
        ///
        /// Returns an error when no physically valid default exists or Arrow
        /// cannot materialize the datatype.
        pub fn default_arrow_array(&self) -> crate::arrow::Result<arrow_array::ArrayRef> {
            crate::arrow::default_dtype_scalar_array(self)
        }

        /// Reports whether an imported datatype can reuse its enclosing Arrow
        /// field.
        ///
        /// Scalar and parameter-only variants import without canonicalization.
        /// Nested fields retain their incoming projection only when their
        /// complete subtree is equivalent, so inspecting each direct child
        /// propagates that result through the tree in one pass without
        /// allocating an Arrow copy.
        pub(crate) fn arrow_import_is_projection_equivalent(&self) -> bool {
            match self {
                Self::Sequence(sequence) => {
                    sequence.item().arrow_import_is_projection_equivalent()
                }
                Self::Structure(fields) => fields
                    .iter()
                    .all(Field::arrow_import_is_projection_equivalent),
                Self::Union(fields, _) => fields
                    .iter()
                    .all(|(_, field)| field.arrow_import_is_projection_equivalent()),
                Self::Enum(EnumType::Dictionary(dictionary)) => {
                    dictionary.key().arrow_import_is_projection_equivalent()
                        && dictionary.value().arrow_import_is_projection_equivalent()
                }
                Self::Mapping(mapping) => mapping.entries().arrow_import_is_projection_equivalent(),
                Self::RunEndEncoded(encoded) => {
                    encoded.run_ends().arrow_import_is_projection_equivalent()
                        && encoded.values().arrow_import_is_projection_equivalent()
                }
                _ => true,
            }
        }
    }

    /// Writes one extension identity onto a finished C schema node.
    fn with_extension(
        schema: FFI_ArrowSchema,
        name: &'static str,
        document: &str,
    ) -> Result<FFI_ArrowSchema> {
        schema
            .with_metadata([
                (arrow_schema::extension::EXTENSION_TYPE_NAME_KEY, name),
                (arrow_schema::extension::EXTENSION_TYPE_METADATA_KEY, document),
            ])
            .map_err(Error::from)
    }

    /// Refuses an Arrow import nested deeper than the parser's own limit.
    ///
    /// # Errors
    ///
    /// Returns an error naming the limit.
    pub(crate) fn check_arrow_import_depth(depth: usize) -> Result<()> {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            Err(invalid(
                "ArrowImport",
                format_smolstr!(
                    "datatype nesting exceeds the limit of {}",
                    DataType::PARSE_RECURSION_LIMIT
                ),
            ))
        } else {
            Ok(())
        }
    }

    impl TryFrom<&DataType> for ArrowDataType {
        type Error = Error;

        fn try_from(value: &DataType) -> Result<Self> {
            value.to_arrow_datatype()
        }
    }

    impl TryFrom<DataType> for ArrowDataType {
        type Error = Error;

        fn try_from(value: DataType) -> Result<Self> {
            value.into_arrow_datatype()
        }
    }

    impl TryFrom<&ArrowDataType> for DataType {
        type Error = Error;

        fn try_from(value: &ArrowDataType) -> Result<Self> {
            Self::from_arrow_datatype_at_depth(value, 0)
        }
    }

    impl TryFrom<ArrowDataType> for DataType {
        type Error = Error;

        fn try_from(value: ArrowDataType) -> Result<Self> {
            Self::from_arrow_datatype_owned_at_depth(value, 0)
        }
    }
}
