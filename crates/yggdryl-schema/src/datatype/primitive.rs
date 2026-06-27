//! Primitive-category checks and constructors: the scalar types (null, boolean,
//! integers, floats, strings, binary).

use super::DataType;
use crate::Charset;

impl DataType {
    // ---- constructors ----

    /// An integer of `bits` width (8/16/32/64), signed or unsigned.
    pub fn int(bits: u16, signed: bool) -> DataType {
        DataType::Int { bits, signed }
    }

    /// A floating-point number of `bits` width (16/32/64).
    pub fn float(bits: u16) -> DataType {
        DataType::Float { bits }
    }

    /// A UTF-8 string (32-bit offsets, no view).
    pub fn varchar() -> DataType {
        DataType::Varchar {
            charset: Charset::Utf8,
            large: false,
            view: false,
        }
    }

    /// A string with the given charset and large/view flags.
    pub fn varchar_with(charset: Charset, large: bool, view: bool) -> DataType {
        DataType::Varchar {
            charset,
            large,
            view,
        }
    }

    /// Variable-length opaque bytes (32-bit offsets).
    pub fn binary() -> DataType {
        DataType::Binary {
            large: false,
            view: false,
            size: None,
        }
    }

    /// Fixed-width opaque bytes of `size` bytes.
    pub fn fixed_size_binary(size: i32) -> DataType {
        DataType::Binary {
            large: false,
            view: false,
            size: Some(size),
        }
    }

    // ---- checks ----

    /// Whether this is a [primitive](super::TypeCategory::Primitive) scalar.
    pub fn is_primitive(&self) -> bool {
        use DataType::*;
        matches!(
            self,
            Null | Boolean | Int { .. } | Float { .. } | Varchar { .. } | Binary { .. }
        )
    }

    /// Whether this is the [`Null`](DataType::Null) type.
    pub fn is_null(&self) -> bool {
        matches!(self, DataType::Null)
    }

    /// Whether this is the [`Boolean`](DataType::Boolean) type.
    pub fn is_boolean(&self) -> bool {
        matches!(self, DataType::Boolean)
    }

    /// Whether this is any integer.
    pub fn is_integer(&self) -> bool {
        matches!(self, DataType::Int { .. })
    }

    /// Whether this is a signed integer.
    pub fn is_signed_integer(&self) -> bool {
        matches!(self, DataType::Int { signed: true, .. })
    }

    /// Whether this is an unsigned integer.
    pub fn is_unsigned_integer(&self) -> bool {
        matches!(self, DataType::Int { signed: false, .. })
    }

    /// Whether this is a floating-point type.
    pub fn is_floating(&self) -> bool {
        matches!(self, DataType::Float { .. })
    }

    /// Whether this is a number — an integer or a float (decimals are
    /// [logical](DataType::is_decimal), not counted here).
    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_floating()
    }

    /// Whether this is a binary (byte) type.
    pub fn is_binary(&self) -> bool {
        matches!(self, DataType::Binary { .. })
    }

    /// Whether this is a string ([`Varchar`](DataType::Varchar)) type.
    pub fn is_string(&self) -> bool {
        matches!(self, DataType::Varchar { .. })
    }

    /// The [`Charset`] of a string type, or `None`.
    pub fn charset(&self) -> Option<Charset> {
        match self {
            DataType::Varchar { charset, .. } => Some(*charset),
            _ => None,
        }
    }
}
