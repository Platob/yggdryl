//! The [`DataTypeId`] discriminant.

/// A `u8` discriminant identifying a data type. Every [`DataType`](crate::DataType)
/// reports one from [`type_id`](crate::DataType::type_id). The set grows as concrete
/// types land; today the signed and unsigned integer types are the ones with real
/// implementations.
///
/// ```
/// use yggdryl_schema::DataTypeId;
///
/// assert_eq!(DataTypeId::Int32 as u8, 4);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum DataTypeId {
    /// The empty type.
    Null = 0,
    /// A boolean.
    Boolean = 1,
    /// An 8-bit signed integer.
    Int8 = 2,
    /// A 16-bit signed integer.
    Int16 = 3,
    /// A 32-bit signed integer.
    Int32 = 4,
    /// A 64-bit signed integer.
    Int64 = 5,
    /// A 128-bit signed integer.
    Int128 = 6,
    /// A 256-bit signed integer.
    Int256 = 7,
    /// An 8-bit unsigned integer.
    UInt8 = 8,
    /// A 16-bit unsigned integer.
    UInt16 = 9,
    /// A 32-bit unsigned integer.
    UInt32 = 10,
    /// A 64-bit unsigned integer.
    UInt64 = 11,
    /// A 128-bit unsigned integer.
    UInt128 = 12,
    /// A 256-bit unsigned integer.
    UInt256 = 13,
    /// Variable-length UTF-8 text.
    Utf8 = 14,
    /// A variable-length list of a single child type.
    List = 15,
    /// A composite of named child fields.
    Struct = 16,
}

impl DataTypeId {
    /// The type's canonical name (e.g. `"int32"`, `"struct"`).
    pub const fn name(self) -> &'static str {
        match self {
            DataTypeId::Null => "null",
            DataTypeId::Boolean => "bool",
            DataTypeId::Int8 => "int8",
            DataTypeId::Int16 => "int16",
            DataTypeId::Int32 => "int32",
            DataTypeId::Int64 => "int64",
            DataTypeId::Int128 => "int128",
            DataTypeId::Int256 => "int256",
            DataTypeId::UInt8 => "uint8",
            DataTypeId::UInt16 => "uint16",
            DataTypeId::UInt32 => "uint32",
            DataTypeId::UInt64 => "uint64",
            DataTypeId::UInt128 => "uint128",
            DataTypeId::UInt256 => "uint256",
            DataTypeId::Utf8 => "utf8",
            DataTypeId::List => "list",
            DataTypeId::Struct => "struct",
        }
    }
}
