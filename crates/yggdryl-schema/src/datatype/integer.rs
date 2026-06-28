//! The [`IntegerType`] — the fixed-width signed and unsigned integers that the
//! [`PrimitiveType`](super::PrimitiveType) groups as its integer family.

use super::DataTypeId;

/// A fixed-width integer type. Each variant's width and signedness are intrinsic to
/// the variant, not parameters; the [`PrimitiveType`](super::PrimitiveType) wraps these
/// as a single `Integer(IntegerType)` family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegerType {
    /// Signed 8-bit integer.
    Int8,
    /// Signed 16-bit integer.
    Int16,
    /// Signed 32-bit integer.
    Int32,
    /// Signed 64-bit integer.
    Int64,
    /// Unsigned 8-bit integer.
    UInt8,
    /// Unsigned 16-bit integer.
    UInt16,
    /// Unsigned 32-bit integer.
    UInt32,
    /// Unsigned 64-bit integer.
    UInt64,
}

impl IntegerType {
    /// The [`DataTypeId`] of this integer.
    ///
    /// ```
    /// use yggdryl_schema::{DataTypeId, IntegerType};
    /// assert_eq!(IntegerType::Int32.type_id(), DataTypeId::Int32);
    /// ```
    pub fn type_id(self) -> DataTypeId {
        use IntegerType::*;
        match self {
            Int8 => DataTypeId::Int8,
            Int16 => DataTypeId::Int16,
            Int32 => DataTypeId::Int32,
            Int64 => DataTypeId::Int64,
            UInt8 => DataTypeId::UInt8,
            UInt16 => DataTypeId::UInt16,
            UInt32 => DataTypeId::UInt32,
            UInt64 => DataTypeId::UInt64,
        }
    }

    /// The canonical name (`"int32"`, `"uint8"`, …).
    pub fn name(self) -> &'static str {
        self.type_id().name()
    }

    /// Whether this is a signed integer (`Int8`…`Int64`); `false` for the `UInt*`.
    pub fn is_signed(self) -> bool {
        use IntegerType::*;
        matches!(self, Int8 | Int16 | Int32 | Int64)
    }
}
