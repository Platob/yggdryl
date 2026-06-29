//! The [`DataTypeId`] discriminant: a `u8` tag for every Arrow data type, grouped
//! into the physical / logical / nested categories.

/// A `u8` discriminant identifying a data type and its category.
///
/// Variants are laid out in category blocks — physical (storage) types below
/// [`LOGICAL_BASE`](DataTypeId::LOGICAL_BASE), logical (reinterpreted) types up to
/// [`NESTED_BASE`](DataTypeId::NESTED_BASE), and nested (child-bearing) types
/// above — so [`is_physical`](DataTypeId::is_physical) and friends are a single
/// range check. The set mirrors Apache Arrow's taxonomy; extend it within the
/// matching block as concrete types land.
///
/// ```
/// use yggdryl_schema::DataTypeId;
///
/// assert!(DataTypeId::Int32.is_physical());
/// assert!(DataTypeId::Timestamp.is_logical());
/// assert!(DataTypeId::Struct.is_nested());
/// ```
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataTypeId {
    // ---- physical (storage) types: 0x00–0x3F ----
    /// The null type — no values.
    Null = 0x00,
    /// A boolean.
    Boolean = 0x01,
    /// A signed 8-bit integer.
    Int8 = 0x02,
    /// A signed 16-bit integer.
    Int16 = 0x03,
    /// A signed 32-bit integer.
    Int32 = 0x04,
    /// A signed 64-bit integer.
    Int64 = 0x05,
    /// An unsigned 8-bit integer.
    UInt8 = 0x06,
    /// An unsigned 16-bit integer.
    UInt16 = 0x07,
    /// An unsigned 32-bit integer.
    UInt32 = 0x08,
    /// An unsigned 64-bit integer.
    UInt64 = 0x09,
    /// A half-precision (16-bit) float.
    Float16 = 0x0A,
    /// A single-precision (32-bit) float.
    Float32 = 0x0B,
    /// A double-precision (64-bit) float.
    Float64 = 0x0C,
    /// Variable-length bytes (32-bit offsets).
    Binary = 0x0D,
    /// Variable-length bytes (64-bit offsets).
    LargeBinary = 0x0E,
    /// Fixed-width bytes.
    FixedSizeBinary = 0x0F,
    /// Variable-length UTF-8 string (32-bit offsets).
    Utf8 = 0x10,
    /// Variable-length UTF-8 string (64-bit offsets).
    LargeUtf8 = 0x11,
    /// View-backed variable-length bytes.
    BinaryView = 0x12,
    /// View-backed variable-length bytes (64-bit sizing).
    LargeBinaryView = 0x13,

    // ---- logical (reinterpreted) types: 0x40–0x7F ----
    /// A 128-bit fixed-point decimal.
    Decimal128 = 0x40,
    /// A 256-bit fixed-point decimal.
    Decimal256 = 0x41,
    /// A date as days since the epoch.
    Date32 = 0x42,
    /// A date as milliseconds since the epoch.
    Date64 = 0x43,
    /// A time of day (32-bit).
    Time32 = 0x44,
    /// A time of day (64-bit).
    Time64 = 0x45,
    /// A timestamp with optional time zone.
    Timestamp = 0x46,
    /// An elapsed duration.
    Duration = 0x47,
    /// A calendar interval.
    Interval = 0x48,
    /// A dictionary-encoded value.
    Dictionary = 0x49,

    // ---- nested (child-bearing) types: 0x80–0xBF ----
    /// A list of a single child type (32-bit offsets).
    List = 0x80,
    /// A list of a single child type (64-bit offsets).
    LargeList = 0x81,
    /// A fixed-length list of a single child type.
    FixedSizeList = 0x82,
    /// A struct of named child fields.
    Struct = 0x83,
    /// A map of key/value child fields.
    Map = 0x84,
    /// A union of several child types.
    Union = 0x85,
}

impl DataTypeId {
    /// The first discriminant of the logical-type block.
    pub const LOGICAL_BASE: u8 = 0x40;
    /// The first discriminant of the nested-type block.
    pub const NESTED_BASE: u8 = 0x80;

    /// Whether this is a physical (storage) type.
    pub fn is_physical(self) -> bool {
        (self as u8) < Self::LOGICAL_BASE
    }

    /// Whether this is a logical (reinterpreted) type.
    pub fn is_logical(self) -> bool {
        (Self::LOGICAL_BASE..Self::NESTED_BASE).contains(&(self as u8))
    }

    /// Whether this is a nested (child-bearing) type.
    pub fn is_nested(self) -> bool {
        (self as u8) >= Self::NESTED_BASE
    }
}
