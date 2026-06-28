//! The [`PrimitiveType`] — the fixed/variable-width scalar types (null, boolean,
//! integers, floats, string, bytes).

use super::{DataTypeId, IntegerType};

/// A primitive (scalar) type. Every variant is parameter-less; its width and
/// signedness are intrinsic. The fixed-width integers are grouped under the
/// [`IntegerType`] family rather than spelled out as separate variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    /// The null type.
    Null,
    /// `true` / `false`.
    Boolean,
    /// A fixed-width signed or unsigned [`integer`](IntegerType).
    Integer(IntegerType),
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
            Integer(int) => int.type_id(),
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

    /// The [`IntegerType`] if this is an integer, else `None`.
    pub fn as_integer(self) -> Option<IntegerType> {
        match self {
            PrimitiveType::Integer(int) => Some(int),
            _ => None,
        }
    }

    /// Whether this is any integer (signed or unsigned).
    pub fn is_integer(self) -> bool {
        matches!(self, PrimitiveType::Integer(_))
    }

    /// Whether this is a floating-point type.
    pub fn is_float(self) -> bool {
        matches!(
            self,
            PrimitiveType::Float16 | PrimitiveType::Float32 | PrimitiveType::Float64
        )
    }
}
