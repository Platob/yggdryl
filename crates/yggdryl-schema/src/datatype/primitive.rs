//! The [`PrimitiveType`] — the fixed/variable-width scalar types (null, boolean,
//! integers, floats, string, bytes).

use super::DataTypeId;

/// A primitive (scalar) type. Every variant is parameter-less; its width and
/// signedness are intrinsic to the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    /// The null type.
    Null,
    /// `true` / `false`.
    Boolean,
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
    /// Half-precision (16-bit) float.
    Float16,
    /// Single-precision (32-bit) float.
    Float32,
    /// Double-precision (64-bit) float.
    Float64,
    /// A UTF-8 string.
    Utf8,
    /// Opaque bytes.
    Binary,
}

impl PrimitiveType {
    /// The [`DataTypeId`] of this type.
    pub fn type_id(self) -> DataTypeId {
        use PrimitiveType::*;
        match self {
            Null => DataTypeId::Null,
            Boolean => DataTypeId::Boolean,
            Int8 => DataTypeId::Int8,
            Int16 => DataTypeId::Int16,
            Int32 => DataTypeId::Int32,
            Int64 => DataTypeId::Int64,
            UInt8 => DataTypeId::UInt8,
            UInt16 => DataTypeId::UInt16,
            UInt32 => DataTypeId::UInt32,
            UInt64 => DataTypeId::UInt64,
            Float16 => DataTypeId::Float16,
            Float32 => DataTypeId::Float32,
            Float64 => DataTypeId::Float64,
            Utf8 => DataTypeId::Utf8,
            Binary => DataTypeId::Binary,
        }
    }

    /// The canonical name (`"int32"`, `"utf8"`, …).
    pub fn name(self) -> &'static str {
        self.type_id().name()
    }

    /// Whether this is any integer (signed or unsigned).
    pub fn is_integer(self) -> bool {
        use PrimitiveType::*;
        matches!(
            self,
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64
        )
    }

    /// Whether this is a floating-point type.
    pub fn is_float(self) -> bool {
        matches!(
            self,
            PrimitiveType::Float16 | PrimitiveType::Float32 | PrimitiveType::Float64
        )
    }
}
