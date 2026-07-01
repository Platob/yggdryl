//! The [`DataTypeId`] discriminant.

/// A `u8` discriminant identifying a data type. Every [`DataType`](crate::DataType)
/// reports one from [`type_id`](crate::DataType::type_id). The set grows as concrete
/// types land; today only [`Binary`](DataTypeId::Binary) has a real implementation.
///
/// ```
/// use yggdryl_schema::DataTypeId;
///
/// assert_eq!(DataTypeId::Binary as u8, 5);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum DataTypeId {
    /// The empty type.
    Null = 0,
    /// A boolean.
    Boolean = 1,
    /// A 32-bit signed integer.
    Int32 = 2,
    /// A 64-bit signed integer.
    Int64 = 3,
    /// A 64-bit IEEE float.
    Float64 = 4,
    /// Variable-length bytes.
    Binary = 5,
    /// Variable-length UTF-8 text.
    Utf8 = 6,
    /// A variable-length list of a single child type.
    List = 7,
    /// A composite of named child fields.
    Struct = 8,
}
