//! Primitive-category checks and constructors: the scalar types (null, boolean,
//! integers, floats, strings, binary).

use super::DataType;
use crate::Charset;

/// The default integer width used by [`integer`](DataType::integer).
pub(crate) const DEFAULT_INT_BITS: u16 = 64;

/// The default float width used by [`floating`](DataType::floating).
pub(crate) const DEFAULT_FLOAT_BITS: u16 = 64;

impl DataType {
    // ---- constructors ----

    /// An integer of `bits` width (commonly 8/16/32/64, but any width is allowed),
    /// signed or unsigned.
    pub fn int(bits: u16, signed: bool) -> DataType {
        DataType::Int { bits, signed }
    }

    /// A generic signed integer at the default width (`int64`) — the no-argument
    /// constructor; pass an explicit width to [`int`](DataType::int).
    pub fn integer() -> DataType {
        DataType::int(DEFAULT_INT_BITS, true)
    }

    /// A floating-point number of `bits` width (commonly 16/32/64, but any width is
    /// allowed for custom encodings).
    pub fn float(bits: u16) -> DataType {
        DataType::Float { bits }
    }

    /// A float at the default width (`float64`) — the no-argument constructor.
    pub fn floating() -> DataType {
        DataType::float(DEFAULT_FLOAT_BITS)
    }

    /// A variable-length UTF-8 string (32-bit offsets, no view).
    pub fn varchar() -> DataType {
        DataType::Varchar {
            charset: Charset::Utf8,
            large: false,
            view: false,
            size: None,
        }
    }

    /// A string with the given charset, large/view flags and optional fixed `size`
    /// (`None` = variable-length).
    pub fn varchar_with(charset: Charset, large: bool, view: bool, size: Option<i32>) -> DataType {
        DataType::Varchar {
            charset,
            large,
            view,
            size,
        }
    }

    /// A fixed-length UTF-8 string of `size` characters (SQL `char(n)`).
    pub fn fixed_size_varchar(size: i32) -> DataType {
        DataType::varchar_with(Charset::Utf8, false, false, Some(size))
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
