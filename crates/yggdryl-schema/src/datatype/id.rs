//! The [`DataTypeId`] — the `u8` discriminant every [`DataType`](super::DataType)
//! carries — and the [`TypeCategory`] it falls under.

/// The broad family a type belongs to. Every [`DataType`](super::DataType) is exactly
/// one of these three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeCategory {
    /// A fixed/variable-width scalar (null, boolean, integers, floats, strings, bytes).
    Primitive,
    /// A richer logical meaning over a physical storage (decimal, temporal, JSON/BSON).
    Logical,
    /// A container of other fields/types (list, struct, map, union, …).
    Nested,
}

impl TypeCategory {
    /// The lowercase name (`"primitive"` / `"logical"` / `"nested"`).
    pub fn name(self) -> &'static str {
        match self {
            TypeCategory::Primitive => "primitive",
            TypeCategory::Logical => "logical",
            TypeCategory::Nested => "nested",
        }
    }
}

/// The stable `u8` id of every concrete type — the single registry the
/// [`DataType`](super::DataType) variants map onto. Parameters (a decimal's
/// precision/scale, a list's element, …) live on the `DataType`, not here; the id is
/// just the discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DataTypeId {
    // ---- primitive ----
    /// The null type.
    Null = 0,
    /// `true` / `false`.
    Boolean = 1,
    /// Signed 8-bit integer.
    Int8 = 2,
    /// Signed 16-bit integer.
    Int16 = 3,
    /// Signed 32-bit integer.
    Int32 = 4,
    /// Signed 64-bit integer.
    Int64 = 5,
    /// Unsigned 8-bit integer.
    UInt8 = 6,
    /// Unsigned 16-bit integer.
    UInt16 = 7,
    /// Unsigned 32-bit integer.
    UInt32 = 8,
    /// Unsigned 64-bit integer.
    UInt64 = 9,
    /// Half-precision (16-bit) float.
    Float16 = 10,
    /// Single-precision (32-bit) float.
    Float32 = 11,
    /// Double-precision (64-bit) float.
    Float64 = 12,
    /// A UTF-8 string.
    Utf8 = 13,
    /// Opaque bytes.
    Binary = 14,

    // ---- logical ----
    /// A `(precision, scale)` decimal.
    Decimal = 15,
    /// A calendar date.
    Date = 16,
    /// A time of day.
    Time = 17,
    /// A timestamp (optionally zoned).
    Timestamp = 18,
    /// An elapsed duration.
    Duration = 19,
    /// A calendar interval.
    Interval = 20,
    /// JSON text (string-backed).
    Json = 21,
    /// A BSON document (binary-backed).
    Bson = 22,

    // ---- nested ----
    /// A list of one element type.
    List = 23,
    /// A composite of named, typed fields.
    Struct = 24,
    /// A map from a key type to a value type.
    Map = 25,
    /// A union of typed alternatives.
    Union = 26,
    /// Dictionary (index → value) encoding.
    Dictionary = 27,
    /// Run-end encoding.
    RunEndEncoded = 28,
}

impl DataTypeId {
    /// The raw `u8` discriminant.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// The [`TypeCategory`] this id falls under.
    pub fn category(self) -> TypeCategory {
        use DataTypeId::*;
        match self {
            Null | Boolean | Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64
            | Float16 | Float32 | Float64 | Utf8 | Binary => TypeCategory::Primitive,
            Decimal | Date | Time | Timestamp | Duration | Interval | Json | Bson => {
                TypeCategory::Logical
            }
            List | Struct | Map | Union | Dictionary | RunEndEncoded => TypeCategory::Nested,
        }
    }

    /// The canonical lowercase name (`"int32"`, `"decimal"`, `"list"`, …).
    pub fn name(self) -> &'static str {
        use DataTypeId::*;
        match self {
            Null => "null",
            Boolean => "bool",
            Int8 => "int8",
            Int16 => "int16",
            Int32 => "int32",
            Int64 => "int64",
            UInt8 => "uint8",
            UInt16 => "uint16",
            UInt32 => "uint32",
            UInt64 => "uint64",
            Float16 => "float16",
            Float32 => "float32",
            Float64 => "float64",
            Utf8 => "utf8",
            Binary => "binary",
            Decimal => "decimal",
            Date => "date",
            Time => "time",
            Timestamp => "timestamp",
            Duration => "duration",
            Interval => "interval",
            Json => "json",
            Bson => "bson",
            List => "list",
            Struct => "struct",
            Map => "map",
            Union => "union",
            Dictionary => "dictionary",
            RunEndEncoded => "run_end_encoded",
        }
    }
}
