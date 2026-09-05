//! The shared logical datatype enum and its cross-family value contract.

use std::cmp::Ordering;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataTypeId, DataTypeKind, Error, Field, Result, TimeUnit, UnionMode};

use super::decimal::validate_decimal;
use super::geospatial::GeospatialParameters;
use super::nested::{
    DictionaryType, Fields, MapType, RunEndEncodedType, UnionFields, cmp_fields,
    validate_dictionary_key, validate_fields, validate_map_entries, validate_run_ends,
    validate_union_fields,
};
use super::temporal::{validate_duration_unit, validate_time32_unit, validate_time64_unit};
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
    /// Variable-width binary data with 32-bit offsets.
    Binary,
    /// Fixed-width binary data.
    FixedSizeBinary(i32),
    /// Variable-width binary data with 64-bit offsets.
    LargeBinary,
    /// Binary view layout.
    BinaryView,
    /// UTF-8 with 32-bit offsets.
    Utf8,
    /// UTF-8 with 64-bit offsets.
    LargeUtf8,
    /// UTF-8 view layout.
    Utf8View,
    /// Variable-width ASCII text.
    Ascii,
    /// ASCII text padded with trailing NUL to a fixed byte width.
    FixedAscii(i32),
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
    List(Arc<Field>),
    /// Variable list-view with 32-bit offsets.
    ListView(Arc<Field>),
    /// Fixed-length list.
    FixedSizeList(Arc<Field>, i32),
    /// Variable list with 64-bit offsets.
    LargeList(Arc<Field>),
    /// Variable list-view with 64-bit offsets.
    LargeListView(Arc<Field>),
    /// Ordered struct fields.
    Struct(Fields),
    /// Tagged union fields and layout mode.
    Union(UnionFields, UnionMode),
    /// Dictionary key and value types.
    Dictionary(Arc<DictionaryType>),
    /// Exact decimal backed by 32 bits.
    Decimal32 { precision: u8, scale: i8 },
    /// Exact decimal backed by 64 bits.
    Decimal64 { precision: u8, scale: i8 },
    /// Exact decimal backed by 128 bits.
    Decimal128 { precision: u8, scale: i8 },
    /// Exact decimal backed by 256 bits.
    Decimal256 { precision: u8, scale: i8 },
    /// Arrow map entries and key-order flag.
    Map(Arc<MapType>),
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
}

impl DataType {
    /// The canonical value this datatype holds, from any value it accepts.
    ///
    /// This is the crate's one value contract in one call: the value is
    /// checked against the datatype and rewritten into the exact
    /// representation it declares - an integer narrowed to its width, a
    /// decimal restated at its scale, a temporal restated at its unit, an
    /// ASCII value trimmed of the padding its storage adds. A value that
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
    /// // The padded spelling storage holds becomes the exact code leaf, trimmed.
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
        crate::stable_hash_display(self)
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
            Self::Binary => DataTypeId::Binary,
            Self::FixedSizeBinary(_) => DataTypeId::FixedSizeBinary,
            Self::LargeBinary => DataTypeId::LargeBinary,
            Self::BinaryView => DataTypeId::BinaryView,
            Self::Utf8 => DataTypeId::Utf8,
            Self::LargeUtf8 => DataTypeId::LargeUtf8,
            Self::Utf8View => DataTypeId::Utf8View,
            Self::Ascii => DataTypeId::Ascii,
            Self::FixedAscii(_) => DataTypeId::FixedAscii,
            Self::Country => DataTypeId::Country,
            Self::Currency => DataTypeId::Currency,
            Self::Mic => DataTypeId::Mic,
            Self::Cfi => DataTypeId::Cfi,
            Self::Guid => DataTypeId::Guid,
            Self::List(_) => DataTypeId::List,
            Self::ListView(_) => DataTypeId::ListView,
            Self::FixedSizeList(..) => DataTypeId::FixedSizeList,
            Self::LargeList(_) => DataTypeId::LargeList,
            Self::LargeListView(_) => DataTypeId::LargeListView,
            Self::Struct(_) => DataTypeId::Struct,
            Self::Union(..) => DataTypeId::Union,
            Self::Dictionary(_) => DataTypeId::Dictionary,
            Self::Decimal32 { .. } => DataTypeId::Decimal32,
            Self::Decimal64 { .. } => DataTypeId::Decimal64,
            Self::Decimal128 { .. } => DataTypeId::Decimal128,
            Self::Decimal256 { .. } => DataTypeId::Decimal256,
            Self::Map(_) => DataTypeId::Map,
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
            Self::Dictionary(dictionary) => dictionary.value.is_nested(),
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
    /// let tags = DataType::list(DataType::Utf8.named_field("item", true))
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
            Self::FixedSizeBinary(width) => {
                validate_non_negative("FixedSizeBinary", "width", *width)
            }
            Self::List(field)
            | Self::ListView(field)
            | Self::LargeList(field)
            | Self::LargeListView(field) => field.validate(),
            Self::FixedSizeList(field, length) => {
                validate_non_negative("FixedSizeList", "length", *length)?;
                field.validate()
            }
            Self::Struct(fields) => validate_fields(fields.as_fields(), "Struct"),
            Self::Union(fields, _) => validate_union_fields(fields),
            Self::Dictionary(dictionary) => {
                validate_dictionary_key(&dictionary.key)?;
                dictionary.key.validate()?;
                dictionary.value.validate()
            }
            Self::Decimal32 { precision, scale } => {
                validate_decimal("Decimal32", *precision, *scale, 9)
            }
            Self::Decimal64 { precision, scale } => {
                validate_decimal("Decimal64", *precision, *scale, 18)
            }
            Self::Decimal128 { precision, scale } => {
                validate_decimal("Decimal128", *precision, *scale, 38)
            }
            Self::Decimal256 { precision, scale } => {
                validate_decimal("Decimal256", *precision, *scale, 76)
            }
            Self::Map(map) => {
                validate_map_entries(&map.entries)?;
                map.entries.validate()
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
            (D::FixedSizeBinary(left), D::FixedSizeBinary(right))
            | (D::FixedAscii(left), D::FixedAscii(right)) => left.cmp(right),
            (D::List(left), D::List(right))
            | (D::ListView(left), D::ListView(right))
            | (D::LargeList(left), D::LargeList(right))
            | (D::LargeListView(left), D::LargeListView(right)) => cmp_fields(left, right),
            (
                D::FixedSizeList(left_field, left_size),
                D::FixedSizeList(right_field, right_size),
            ) => cmp_fields(left_field, right_field).then_with(|| left_size.cmp(right_size)),
            (D::Struct(left), D::Struct(right)) => left.cmp(right),
            (D::Union(left_fields, left_mode), D::Union(right_fields, right_mode)) => left_mode
                .cmp(right_mode)
                .then_with(|| left_fields.cmp(right_fields)),
            (D::Dictionary(left), D::Dictionary(right)) => left.cmp(right),
            (
                D::Decimal32 {
                    precision: left_precision,
                    scale: left_scale,
                },
                D::Decimal32 {
                    precision: right_precision,
                    scale: right_scale,
                },
            )
            | (
                D::Decimal64 {
                    precision: left_precision,
                    scale: left_scale,
                },
                D::Decimal64 {
                    precision: right_precision,
                    scale: right_scale,
                },
            )
            | (
                D::Decimal128 {
                    precision: left_precision,
                    scale: left_scale,
                },
                D::Decimal128 {
                    precision: right_precision,
                    scale: right_scale,
                },
            )
            | (
                D::Decimal256 {
                    precision: left_precision,
                    scale: left_scale,
                },
                D::Decimal256 {
                    precision: right_precision,
                    scale: right_scale,
                },
            ) => (left_precision, left_scale).cmp(&(right_precision, right_scale)),
            (D::Map(left), D::Map(right)) => left.cmp(right),
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
        DataType::Binary => 21,
        DataType::FixedSizeBinary(_) => 22,
        DataType::LargeBinary => 23,
        DataType::BinaryView => 24,
        DataType::Utf8 => 25,
        DataType::LargeUtf8 => 26,
        DataType::Utf8View => 27,
        DataType::Ascii => 28,
        DataType::FixedAscii(_) => 29,
        DataType::Country => 30,
        DataType::Currency => 31,
        DataType::Mic => 32,
        DataType::Cfi => 33,
        DataType::Guid => 34,
        DataType::List(_) => 35,
        DataType::ListView(_) => 36,
        DataType::FixedSizeList(..) => 37,
        DataType::LargeList(_) => 38,
        DataType::LargeListView(_) => 39,
        DataType::Struct(_) => 40,
        DataType::Union(..) => 41,
        DataType::Dictionary(_) => 42,
        DataType::Decimal32 { .. } => 43,
        DataType::Decimal64 { .. } => 44,
        DataType::Decimal128 { .. } => 45,
        DataType::Decimal256 { .. } => 46,
        DataType::Map(_) => 47,
        DataType::RunEndEncoded(_) => 48,
        DataType::Variant => 49,
        DataType::Geometry(_) => 50,
        DataType::Geography(_) => 51,
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
