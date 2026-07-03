//! The [`DataTypeId`] enum: the identifier of every data type in the model.

/// The identifier of a data type — one variant per Apache Arrow type, independent of
/// any parameters (precision, time unit, time zone, child fields, …).
///
/// It *classifies* a type without describing it: the concrete
/// [`RawDataType`](super::RawDataType) carries the parameters and the exact Arrow C
/// Data Interface format string, while a `DataTypeId` is the cheap `Copy` tag used to
/// switch on or group types. [`ALL`](DataTypeId::ALL) lists every id.
///
/// `#[non_exhaustive]`: new ids are appended as the model grows, so external `match`es
/// must include a wildcard arm.
///
/// ```
/// use yggdryl_dtype::DataTypeId;
///
/// assert_eq!(DataTypeId::Int64.name(), "int64");
/// assert_eq!(DataTypeId::Int64.arrow_format(), Some("l"));
/// assert!(DataTypeId::Int64.is_primitive());
/// assert!(DataTypeId::Struct.is_nested());
///
/// // Parameterized and logical types have no id-level format string.
/// assert_eq!(DataTypeId::Decimal128.arrow_format(), None);
/// assert_eq!(DataTypeId::Timestamp.arrow_format(), None);
///
/// // Every id is enumerable.
/// assert!(DataTypeId::ALL.contains(&DataTypeId::Utf8));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataTypeId {
    /// The null type: every value is null.
    Null,
    /// A boolean, bit-packed.
    Boolean,
    /// A signed 8-bit integer.
    Int8,
    /// A signed 16-bit integer.
    Int16,
    /// A signed 32-bit integer.
    Int32,
    /// A signed 64-bit integer.
    Int64,
    /// An unsigned 8-bit integer.
    UInt8,
    /// An unsigned 16-bit integer.
    UInt16,
    /// An unsigned 32-bit integer.
    UInt32,
    /// An unsigned 64-bit integer.
    UInt64,
    /// A 16-bit (half-precision) float.
    Float16,
    /// A 32-bit (single-precision) float.
    Float32,
    /// A 64-bit (double-precision) float.
    Float64,
    /// Variable-length bytes (32-bit offsets).
    Binary,
    /// Variable-length bytes (64-bit offsets).
    LargeBinary,
    /// A view of variable-length bytes.
    BinaryView,
    /// Fixed-width bytes (`w:N`).
    FixedSizeBinary,
    /// A variable-length UTF-8 string (32-bit offsets).
    Utf8,
    /// A variable-length UTF-8 string (64-bit offsets).
    LargeUtf8,
    /// A view of a variable-length UTF-8 string.
    Utf8View,
    /// Days since the UNIX epoch, stored as `int32`.
    Date32,
    /// Milliseconds since the UNIX epoch, stored as `int64`.
    Date64,
    /// Time of day, stored as `int32` (seconds or milliseconds).
    Time32,
    /// Time of day, stored as `int64` (microseconds or nanoseconds).
    Time64,
    /// A timestamp with a unit and optional time zone.
    Timestamp,
    /// An elapsed time with a unit.
    Duration,
    /// A calendar interval.
    Interval,
    /// A 128-bit fixed-point decimal (`d:precision,scale`).
    Decimal128,
    /// A 256-bit fixed-point decimal (`d:precision,scale,256`).
    Decimal256,
    /// A variable-length list (32-bit offsets).
    List,
    /// A variable-length list (64-bit offsets).
    LargeList,
    /// A view of a variable-length list (32-bit offsets).
    ListView,
    /// A view of a variable-length list (64-bit offsets).
    LargeListView,
    /// A fixed-length list (`w:N`).
    FixedSizeList,
    /// An ordered set of named child fields.
    Struct,
    /// A union of several child types (sparse or dense).
    Union,
    /// A map of keys to values.
    Map,
    /// A dictionary-encoded type (indices into a value dictionary).
    Dictionary,
    /// A run-end-encoded type.
    RunEndEncoded,
}

impl DataTypeId {
    /// Every data type id, in declaration order. Kept in sync with the variants.
    pub const ALL: &'static [DataTypeId] = &[
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
        Self::Binary,
        Self::LargeBinary,
        Self::BinaryView,
        Self::FixedSizeBinary,
        Self::Utf8,
        Self::LargeUtf8,
        Self::Utf8View,
        Self::Date32,
        Self::Date64,
        Self::Time32,
        Self::Time64,
        Self::Timestamp,
        Self::Duration,
        Self::Interval,
        Self::Decimal128,
        Self::Decimal256,
        Self::List,
        Self::LargeList,
        Self::ListView,
        Self::LargeListView,
        Self::FixedSizeList,
        Self::Struct,
        Self::Union,
        Self::Map,
        Self::Dictionary,
        Self::RunEndEncoded,
    ];

    /// The stable, lowercase name of this id, e.g. `"int64"`, `"large_utf8"`.
    pub fn name(&self) -> &'static str {
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
            Self::Binary => "binary",
            Self::LargeBinary => "large_binary",
            Self::BinaryView => "binary_view",
            Self::FixedSizeBinary => "fixed_size_binary",
            Self::Utf8 => "utf8",
            Self::LargeUtf8 => "large_utf8",
            Self::Utf8View => "utf8_view",
            Self::Date32 => "date32",
            Self::Date64 => "date64",
            Self::Time32 => "time32",
            Self::Time64 => "time64",
            Self::Timestamp => "timestamp",
            Self::Duration => "duration",
            Self::Interval => "interval",
            Self::Decimal128 => "decimal128",
            Self::Decimal256 => "decimal256",
            Self::List => "list",
            Self::LargeList => "large_list",
            Self::ListView => "list_view",
            Self::LargeListView => "large_list_view",
            Self::FixedSizeList => "fixed_size_list",
            Self::Struct => "struct",
            Self::Union => "union",
            Self::Map => "map",
            Self::Dictionary => "dictionary",
            Self::RunEndEncoded => "run_end_encoded",
        }
    }

    /// The Arrow C Data Interface format string for the parameterless types, or `None`
    /// for a type whose format depends on parameters or a logical unit (decimals,
    /// temporals, fixed-size and union/dictionary types) — there the concrete
    /// [`RawDataType`](super::RawDataType) builds the exact string.
    pub fn arrow_format(&self) -> Option<&'static str> {
        let format = match self {
            Self::Null => "n",
            Self::Boolean => "b",
            Self::Int8 => "c",
            Self::Int16 => "s",
            Self::Int32 => "i",
            Self::Int64 => "l",
            Self::UInt8 => "C",
            Self::UInt16 => "S",
            Self::UInt32 => "I",
            Self::UInt64 => "L",
            Self::Float16 => "e",
            Self::Float32 => "f",
            Self::Float64 => "g",
            Self::Binary => "z",
            Self::LargeBinary => "Z",
            Self::BinaryView => "vz",
            Self::Utf8 => "u",
            Self::LargeUtf8 => "U",
            Self::Utf8View => "vu",
            Self::List => "+l",
            Self::LargeList => "+L",
            Self::ListView => "+vl",
            Self::LargeListView => "+vL",
            Self::Struct => "+s",
            Self::Map => "+m",
            Self::RunEndEncoded => "+r",
            Self::FixedSizeBinary
            | Self::Date32
            | Self::Date64
            | Self::Time32
            | Self::Time64
            | Self::Timestamp
            | Self::Duration
            | Self::Interval
            | Self::Decimal128
            | Self::Decimal256
            | Self::FixedSizeList
            | Self::Union
            | Self::Dictionary => return None,
        };
        Some(format)
    }

    /// Whether this is a fixed-width numeric or boolean primitive.
    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            Self::Boolean
                | Self::Int8
                | Self::Int16
                | Self::Int32
                | Self::Int64
                | Self::UInt8
                | Self::UInt16
                | Self::UInt32
                | Self::UInt64
                | Self::Float16
                | Self::Float32
                | Self::Float64
        )
    }

    /// Whether this type is composed of child fields (lists, struct, union, map).
    pub fn is_nested(&self) -> bool {
        matches!(
            self,
            Self::List
                | Self::LargeList
                | Self::ListView
                | Self::LargeListView
                | Self::FixedSizeList
                | Self::Struct
                | Self::Union
                | Self::Map
        )
    }
}
