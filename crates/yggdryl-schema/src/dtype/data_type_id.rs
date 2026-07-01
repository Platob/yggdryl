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
