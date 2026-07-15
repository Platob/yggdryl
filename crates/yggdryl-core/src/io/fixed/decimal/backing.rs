//! The two **shared decimal traits** the whole `decimal` family is generic over:
//! [`DecimalCoeff`] (the coefficient integer + its checked arithmetic, one impl per width) and
//! [`DecimalBacking`] (a zero-sized marker tying a width's coefficient to its
//! [`DataTypeId`](crate::io::DataTypeId), name, max precision, and — under feature `arrow` — its
//! zero-copy Arrow `Decimal*Type`).
//!
//! A scaled decimal is `coefficient × 10^-scale`. The coefficient is a fixed-width **two's
//! complement** integer — `i32`/`i64`/`i128` for `d32`/`d64`/`d128`, and Arrow's 256-bit
//! [`i256`](arrow_buffer::i256) for `d256` (kept an implementation detail — never in a public
//! signature). Overflow is always *checked*: every op returns `None` rather than wrapping, so the
//! value type can surface a guided [`DecimalError`](super::DecimalError).

use crate::io::DataTypeId;

/// The **coefficient integer** of a decimal width — a fixed-width two's-complement integer with
/// the checked arithmetic a scaled decimal needs, plus a canonical little-endian codec. Implemented
/// for `i32`, `i64`, `i128` (`d32`/`d64`/`d128`) and Arrow's [`i256`](arrow_buffer::i256) (`d256`).
///
/// Every arithmetic method is **checked** — `None` on overflow, never a wrap — so the decimal
/// layer can turn an overflow into a guided error instead of a silent wrong answer.
pub trait DecimalCoeff:
    Copy
    + Default
    + Ord
    + core::hash::Hash
    + core::fmt::Debug
    + core::fmt::Display
    + Send
    + Sync
    + 'static
{
    /// The coefficient width in bytes (`4`/`8`/`16`/`32`).
    const WIDTH: usize;
    /// The additive identity (`0`).
    const ZERO: Self;

    /// The coefficient equal to `value`, or `None` if it does not fit this width.
    fn from_i128(value: i128) -> Option<Self>;
    /// This coefficient as an `i128`, or `None` if it does not fit (`d256` only, for large values).
    fn to_i128(self) -> Option<i128>;

    /// `self + rhs`, or `None` on overflow.
    fn checked_add(self, rhs: Self) -> Option<Self>;
    /// `self - rhs`, or `None` on overflow.
    fn checked_sub(self, rhs: Self) -> Option<Self>;
    /// `self * rhs`, or `None` on overflow.
    fn checked_mul(self, rhs: Self) -> Option<Self>;
    /// Truncating `self / rhs`, or `None` on overflow or divide-by-zero.
    fn checked_div(self, rhs: Self) -> Option<Self>;
    /// `self % rhs` (truncating), or `None` on overflow or divide-by-zero.
    fn checked_rem(self, rhs: Self) -> Option<Self>;
    /// `-self`, or `None` on overflow (the two's-complement minimum negates out of range).
    fn checked_neg(self) -> Option<Self>;
    /// `10^exp` as this coefficient, or `None` on overflow — the scale-alignment multiplier.
    fn checked_pow10(exp: u32) -> Option<Self>;

    /// Whether the coefficient is strictly negative.
    fn is_negative(self) -> bool;
    /// This coefficient as an `f64` (lossy for magnitudes beyond `f64`'s 53-bit mantissa).
    fn to_f64(self) -> f64;
    /// Parses a plain decimal **integer** string (optional leading `-`), or `None` if malformed
    /// or out of range.
    fn parse_int(text: &str) -> Option<Self>;

    /// Writes this coefficient's little-endian bytes into the first [`WIDTH`](DecimalCoeff::WIDTH)
    /// bytes of `out`.
    fn write_le(self, out: &mut [u8]);
    /// Reads a coefficient from the first [`WIDTH`](DecimalCoeff::WIDTH) little-endian bytes.
    fn read_le(bytes: &[u8]) -> Self;

    /// The number of significant decimal digits in the magnitude (`0` → `0`, `1230` → `4`) — the
    /// value's *precision*. Allocation-free: it divides by ten (truncating toward zero, so it is
    /// total even at the two's-complement minimum) rather than formatting.
    fn digit_count(self) -> u32 {
        let ten = Self::from_i128(10).expect("10 fits every decimal coefficient width");
        let mut n = self;
        let mut count = 0;
        while n != Self::ZERO {
            n = n.checked_div(ten).expect("division by ten never overflows");
            count += 1;
        }
        count
    }

    /// The signum as an `i8` (`-1` / `0` / `1`).
    fn signum(self) -> i8 {
        if self.is_negative() {
            -1
        } else if self == Self::ZERO {
            0
        } else {
            1
        }
    }
}

/// Implements [`DecimalCoeff`] for a Rust primitive integer via its inherent checked ops.
macro_rules! prim_coeff {
    ($t:ty) => {
        impl DecimalCoeff for $t {
            const WIDTH: usize = ::core::mem::size_of::<$t>();
            const ZERO: Self = 0;

            fn from_i128(value: i128) -> Option<Self> {
                <$t>::try_from(value).ok()
            }
            fn to_i128(self) -> Option<i128> {
                Some(self as i128)
            }
            fn checked_add(self, rhs: Self) -> Option<Self> {
                <$t>::checked_add(self, rhs)
            }
            fn checked_sub(self, rhs: Self) -> Option<Self> {
                <$t>::checked_sub(self, rhs)
            }
            fn checked_mul(self, rhs: Self) -> Option<Self> {
                <$t>::checked_mul(self, rhs)
            }
            fn checked_div(self, rhs: Self) -> Option<Self> {
                <$t>::checked_div(self, rhs)
            }
            fn checked_rem(self, rhs: Self) -> Option<Self> {
                <$t>::checked_rem(self, rhs)
            }
            fn checked_neg(self) -> Option<Self> {
                <$t>::checked_neg(self)
            }
            fn checked_pow10(exp: u32) -> Option<Self> {
                (10 as $t).checked_pow(exp)
            }
            fn is_negative(self) -> bool {
                self < 0
            }
            fn to_f64(self) -> f64 {
                self as f64
            }
            fn parse_int(text: &str) -> Option<Self> {
                text.parse::<$t>().ok()
            }
            fn write_le(self, out: &mut [u8]) {
                out[..Self::WIDTH].copy_from_slice(&self.to_le_bytes());
            }
            fn read_le(bytes: &[u8]) -> Self {
                let mut array = [0u8; ::core::mem::size_of::<$t>()];
                array.copy_from_slice(&bytes[..Self::WIDTH]);
                <$t>::from_le_bytes(array)
            }
        }
    };
}

prim_coeff!(i32);
prim_coeff!(i64);
prim_coeff!(i128);

impl DecimalCoeff for arrow_buffer::i256 {
    const WIDTH: usize = 32;
    const ZERO: Self = arrow_buffer::i256::ZERO;

    fn from_i128(value: i128) -> Option<Self> {
        Some(arrow_buffer::i256::from_i128(value))
    }
    fn to_i128(self) -> Option<i128> {
        arrow_buffer::i256::to_i128(self)
    }
    fn checked_add(self, rhs: Self) -> Option<Self> {
        arrow_buffer::i256::checked_add(self, rhs)
    }
    fn checked_sub(self, rhs: Self) -> Option<Self> {
        arrow_buffer::i256::checked_sub(self, rhs)
    }
    fn checked_mul(self, rhs: Self) -> Option<Self> {
        arrow_buffer::i256::checked_mul(self, rhs)
    }
    fn checked_div(self, rhs: Self) -> Option<Self> {
        arrow_buffer::i256::checked_div(self, rhs)
    }
    fn checked_rem(self, rhs: Self) -> Option<Self> {
        arrow_buffer::i256::checked_rem(self, rhs)
    }
    fn checked_neg(self) -> Option<Self> {
        arrow_buffer::i256::checked_neg(self)
    }
    fn checked_pow10(exp: u32) -> Option<Self> {
        arrow_buffer::i256::from_i128(10).checked_pow(exp)
    }
    fn is_negative(self) -> bool {
        arrow_buffer::i256::is_negative(self)
    }
    fn to_f64(self) -> f64 {
        // Exact-basis reconstruction from the (low, high) 128-bit halves — no `ToPrimitive`
        // dependency and no allocation. `2^128` is exactly representable in `f64`.
        let (low, high) = self.to_parts();
        (high as f64) * 340_282_366_920_938_463_463_374_607_431_768_211_456.0 + (low as f64)
    }
    fn parse_int(text: &str) -> Option<Self> {
        arrow_buffer::i256::from_string(text)
    }
    fn write_le(self, out: &mut [u8]) {
        out[..32].copy_from_slice(&self.to_le_bytes());
    }
    fn read_le(bytes: &[u8]) -> Self {
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes[..32]);
        arrow_buffer::i256::from_le_bytes(array)
    }
}

/// A **decimal width** — the zero-sized marker (`Dec32`/`Dec64`/`Dec128`/`Dec256`) that ties a
/// [`DecimalCoeff`] to the rest of the family: its [`DataTypeId`], canonical name, maximum
/// precision, and (under feature `arrow`) its zero-copy Arrow `Decimal*Type`. The value type
/// [`Decimal<B>`](super::Decimal) and the columnar descriptors are all generic over `B: DecimalBacking`.
pub trait DecimalBacking: Copy + Default + Send + Sync + 'static {
    /// The coefficient integer type for this width.
    type Coeff: DecimalCoeff;

    /// The stable, lower-case type name (`"d32"` … `"d256"`).
    const NAME: &'static str;
    /// The coefficient width in bytes (`4`/`8`/`16`/`32`).
    const WIDTH: usize;
    /// The [`DataTypeId`] — `D32` … `D256`.
    const TYPE_ID: DataTypeId;
    /// The maximum precision (significant digits) a value of this width can hold — `9`/`18`/`38`/`76`.
    const MAX_PRECISION: u8;

    /// The matching Arrow `Decimal*Type` (feature `arrow`), whose `Native` is
    /// [`Coeff`](DecimalBacking::Coeff) — so a values buffer of coefficients *is* its
    /// `PrimitiveArray`'s values buffer, giving the zero-copy Arrow round-trip. The
    /// [`DecimalType`](arrow_array::types::DecimalType) bound unlocks
    /// `PrimitiveArray::with_precision_and_scale` / `.precision()` / `.scale()`.
    #[cfg(feature = "arrow")]
    type Arrow: arrow_array::ArrowPrimitiveType<Native = Self::Coeff>
        + arrow_array::types::DecimalType;
}
