//! Primitive-category checks and constructors: the scalar types (null, boolean,
//! integers, floats, strings, binary).

use super::fixed::{
    FixedKind, Float16, Float32, Float64, Int16, Int32, Int64, Int8, UInt16, UInt32, UInt64, UInt8,
};
use super::DataType;
#[allow(unused_imports)]
use crate::log_event;
use crate::Charset;

/// The default integer width used by [`integer`](DataType::integer).
pub(crate) const DEFAULT_INT_BITS: u16 = 64;

/// The default float width used by [`floating`](DataType::floating).
pub(crate) const DEFAULT_FLOAT_BITS: u16 = 64;

impl DataType {
    // ---- constructors ----

    /// The fixed-width integer for `(bits, signed)` — the convenience builder over the
    /// concrete [`Int8`](DataType::Int8) … [`UInt64`](DataType::UInt64) variants. The
    /// type system carries only the standard widths (8/16/32/64); a non-standard width
    /// rounds **up** to the next supported one (saturating at 64) with a `warn` log.
    pub fn int(bits: u16, signed: bool) -> DataType {
        match (bits, signed) {
            (8, true) => Int8::new().into(),
            (16, true) => Int16::new().into(),
            (32, true) => Int32::new().into(),
            (64, true) => Int64::new().into(),
            (8, false) => UInt8::new().into(),
            (16, false) => UInt16::new().into(),
            (32, false) => UInt32::new().into(),
            (64, false) => UInt64::new().into(),
            _ => {
                let standard = match bits {
                    0..=8 => 8,
                    9..=16 => 16,
                    17..=32 => 32,
                    _ => 64,
                };
                log_event!(
                    warn,
                    "DataType::int: non-standard width {bits} mapped to {}int{standard}",
                    if signed { "" } else { "u" }
                );
                DataType::int(standard, signed)
            }
        }
    }

    /// A signed 8-bit integer ([`int8`](DataType::Int8)).
    pub fn int8() -> DataType {
        Int8::new().into()
    }

    /// A signed 16-bit integer ([`int16`](DataType::Int16)).
    pub fn int16() -> DataType {
        Int16::new().into()
    }

    /// A signed 32-bit integer ([`int32`](DataType::Int32)).
    pub fn int32() -> DataType {
        Int32::new().into()
    }

    /// A signed 64-bit integer ([`int64`](DataType::Int64)).
    pub fn int64() -> DataType {
        Int64::new().into()
    }

    /// An unsigned 8-bit integer ([`uint8`](DataType::UInt8)).
    pub fn uint8() -> DataType {
        UInt8::new().into()
    }

    /// An unsigned 16-bit integer ([`uint16`](DataType::UInt16)).
    pub fn uint16() -> DataType {
        UInt16::new().into()
    }

    /// An unsigned 32-bit integer ([`uint32`](DataType::UInt32)).
    pub fn uint32() -> DataType {
        UInt32::new().into()
    }

    /// An unsigned 64-bit integer ([`uint64`](DataType::UInt64)).
    pub fn uint64() -> DataType {
        UInt64::new().into()
    }

    /// A signed integer at the default width (`int64`) — the no-argument constructor;
    /// pass an explicit width to [`int`](DataType::int).
    pub fn integer() -> DataType {
        DataType::int(DEFAULT_INT_BITS, true)
    }

    /// The fixed-width float for `bits` — the convenience builder over the concrete
    /// [`Float16`](DataType::Float16) / [`Float32`](DataType::Float32) /
    /// [`Float64`](DataType::Float64) variants. The type system carries only the IEEE
    /// widths (16/32/64); a non-standard width rounds **up** to the next supported one
    /// (saturating at 64) with a `warn` log.
    pub fn float(bits: u16) -> DataType {
        match bits {
            16 => Float16::new().into(),
            32 => Float32::new().into(),
            64 => Float64::new().into(),
            _ => {
                let standard = match bits {
                    0..=16 => 16,
                    17..=32 => 32,
                    _ => 64,
                };
                log_event!(
                    warn,
                    "DataType::float: non-standard width {bits} mapped to float{standard}"
                );
                DataType::float(standard)
            }
        }
    }

    /// A half-precision (16-bit) float ([`float16`](DataType::Float16)).
    pub fn float16() -> DataType {
        Float16::new().into()
    }

    /// A single-precision (32-bit) float ([`float32`](DataType::Float32)).
    pub fn float32() -> DataType {
        Float32::new().into()
    }

    /// A double-precision (64-bit) float ([`float64`](DataType::Float64)).
    pub fn float64() -> DataType {
        Float64::new().into()
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
        // The fixed-width numerics are primitive; so are null / boolean / strings / bytes.
        self.fixed().is_some()
            || matches!(
                self,
                DataType::Null
                    | DataType::Boolean
                    | DataType::Varchar { .. }
                    | DataType::Binary { .. }
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
        matches!(
            self.fixed().map(|t| t.kind),
            Some(FixedKind::SignedInt | FixedKind::UnsignedInt)
        )
    }

    /// Whether this is a signed integer.
    pub fn is_signed_integer(&self) -> bool {
        self.fixed().map(|t| t.kind) == Some(FixedKind::SignedInt)
    }

    /// Whether this is an unsigned integer.
    pub fn is_unsigned_integer(&self) -> bool {
        self.fixed().map(|t| t.kind) == Some(FixedKind::UnsignedInt)
    }

    /// Whether this is a floating-point type.
    pub fn is_floating(&self) -> bool {
        self.fixed().map(|t| t.kind) == Some(FixedKind::Float)
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
