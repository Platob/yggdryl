//! Primitive-category checks and constructors: the scalar types (null, boolean,
//! integers, floats, strings, binary).

use super::DataType;
#[allow(unused_imports)]
use crate::log_event;
use crate::Charset;

/// The default integer width used by [`integer`](DataType::integer) and the
/// byte-decode fallback.
pub(crate) const DEFAULT_INT_BITS: u16 = 64;

/// The default float width used by [`floating`](DataType::floating) and the
/// byte-decode fallback.
pub(crate) const DEFAULT_FLOAT_BITS: u16 = 64;

/// The largest byte-aligned integer width that fits a `u16` (65528 bits = 8191
/// bytes) — the clamp ceiling for [`int_from_bytes`](DataType::int_from_bytes), so an
/// inferred width is always a whole number of bytes (`!7` clears the low 3 bits).
const MAX_BYTE_ALIGNED_BITS: u16 = !7;

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

    /// An integer type wide enough to hold a value of `bytes.len()` bytes — the
    /// width is inferred from the buffer length (1 → `int8`, 2 → `int16`, 4 →
    /// `int32`, 8 → `int64`, 16 → `int128`; any length maps to `bytes.len() * 8`
    /// bits). Works on an owned array or a borrowed view. An empty buffer falls back
    /// to the default width; a buffer wider than `u16` caps at the largest
    /// byte-aligned width (both defaults are logged at `warn`).
    pub fn int_from_bytes(bytes: &[u8], signed: bool) -> DataType {
        if bytes.is_empty() {
            log_event!(
                warn,
                "int_from_bytes: empty buffer, defaulting to int{DEFAULT_INT_BITS}"
            );
            return DataType::int(DEFAULT_INT_BITS, signed);
        }
        // Do the width math in u64 so the `* 8` cannot overflow before the clamp.
        let wanted = (bytes.len() as u64).saturating_mul(8);
        let capped = wanted.min(MAX_BYTE_ALIGNED_BITS as u64) as u16;
        if (capped as u64) < wanted {
            log_event!(
                warn,
                "int_from_bytes: {} bytes exceeds the max integer width, capping at {capped} bits",
                bytes.len()
            );
        }
        DataType::int(capped, signed)
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

    /// A float type wide enough to hold a `bytes.len()`-byte encoding (2 → `float16`,
    /// 4 → `float32`, 8 → `float64`; any length maps to `bytes.len() * 8` bits). An
    /// empty buffer falls back to the default width; an oversized buffer caps at the
    /// largest byte-aligned width (both logged at `warn`).
    pub fn float_from_bytes(bytes: &[u8]) -> DataType {
        if bytes.is_empty() {
            log_event!(
                warn,
                "float_from_bytes: empty buffer, defaulting to float{DEFAULT_FLOAT_BITS}"
            );
            return DataType::float(DEFAULT_FLOAT_BITS);
        }
        let wanted = (bytes.len() as u64).saturating_mul(8);
        let capped = wanted.min(MAX_BYTE_ALIGNED_BITS as u64) as u16;
        if (capped as u64) < wanted {
            log_event!(
                warn,
                "float_from_bytes: {} bytes exceeds the max float width, capping at {capped} bits",
                bytes.len()
            );
        }
        DataType::float(capped)
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
