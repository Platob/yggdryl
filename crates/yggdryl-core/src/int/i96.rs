//! [`i96`] — a 96-bit signed two's-complement integer.
#![allow(non_camel_case_types)] // `i96` mirrors the primitive integer naming (like `i128`).

use core::fmt;
use core::ops::{Add, Div, Mul, Neg, Rem, Sub};

/// A 96-bit signed two's-complement integer (range `-2^95 ..= 2^95 - 1`).
///
/// The 96-bit width sits between native `i64` and `i128`, matching Arrow's
/// `interval`/`decimal`-adjacent widths. The value is held canonically in an `i128`,
/// so arithmetic reuses the native 128-bit path and then re-wraps to 96 bits; the
/// arithmetic operators (`+`, `-`, `*`, `/`, `%`, unary `-`) **panic on overflow**
/// like the primitive integers, with `checked_*` / `wrapping_*` / `saturating_*` /
/// `overflowing_*` for the other overflow behaviours.
///
/// It round-trips through 12 little-endian bytes
/// ([`to_le_bytes`](i96::to_le_bytes) / [`from_le_bytes`](i96::from_le_bytes)) and has
/// value semantics (`Eq` / `Ord` / `Hash` by value), so it is an
/// [`IoPrimitive`](crate::IoPrimitive): `TypedCursor<i96>` reads and writes it.
///
/// ```
/// use yggdryl_core::i96;
///
/// let a = i96::from_i64(1_000_000_000_000);
/// assert_eq!((a * i96::from_i64(1000)).to_i128(), 1_000_000_000_000_000);
/// assert_eq!(i96::MAX.checked_add(i96::ONE), None); // overflow
/// assert_eq!(i96::from_le_bytes(a.to_le_bytes()), a);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct i96 {
    /// The canonical value, always in `-2^95 ..= 2^95 - 1`.
    repr: i128,
}

impl i96 {
    /// The number of bits (96).
    pub const BITS: u32 = 96;
    /// The value `0`.
    pub const ZERO: Self = Self { repr: 0 };
    /// The value `1`.
    pub const ONE: Self = Self { repr: 1 };
    /// The smallest value, `-2^95`.
    pub const MIN: Self = Self {
        repr: -(1i128 << 95),
    };
    /// The largest value, `2^95 - 1`.
    pub const MAX: Self = Self {
        repr: (1i128 << 95) - 1,
    };

    /// The low-96-bit mask.
    const MASK: u128 = (1u128 << 96) - 1;

    /// Wraps an `i128` into the 96-bit range (two's-complement, modulo `2^96`).
    fn wrap(value: i128) -> Self {
        let low = (value as u128) & Self::MASK;
        let repr = if low & (1u128 << 95) != 0 {
            (low as i128) - (1i128 << 96)
        } else {
            low as i128
        };
        Self { repr }
    }

    /// Whether `value` is representable in 96 signed bits.
    const fn in_range(value: i128) -> bool {
        value >= Self::MIN.repr && value <= Self::MAX.repr
    }

    /// Creates an `i96` from an `i128`, **wrapping** modulo `2^96`.
    pub fn from_i128(value: i128) -> Self {
        Self::wrap(value)
    }

    /// Creates an `i96` from an `i128` if it fits, else `None`.
    pub fn try_from_i128(value: i128) -> Option<Self> {
        Self::in_range(value).then_some(Self { repr: value })
    }

    /// Creates an `i96` from an `i64` (always in range).
    pub fn from_i64(value: i64) -> Self {
        Self {
            repr: value as i128,
        }
    }

    /// The value as an `i128` (exact — the canonical representation).
    pub fn to_i128(self) -> i128 {
        self.repr
    }

    /// The value's 12 little-endian two's-complement bytes.
    pub fn to_le_bytes(self) -> [u8; 12] {
        let full = self.repr.to_le_bytes();
        let mut out = [0u8; 12];
        out.copy_from_slice(&full[..12]);
        out
    }

    /// Reconstructs an `i96` from 12 little-endian two's-complement bytes.
    pub fn from_le_bytes(bytes: [u8; 12]) -> Self {
        let mut full = [0u8; 16];
        full[..12].copy_from_slice(&bytes);
        // Sign-extend from bit 95 (the top bit of byte 11) through bytes 12..16.
        if bytes[11] & 0x80 != 0 {
            full[12..].fill(0xFF);
        }
        Self {
            repr: i128::from_le_bytes(full),
        }
    }

    /// Checked addition — `None` on overflow.
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        let sum = self.repr + rhs.repr; // both in `±2^95`, so the `i128` add cannot overflow
        Self::try_from_i128(sum)
    }

    /// Checked subtraction — `None` on overflow.
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        Self::try_from_i128(self.repr - rhs.repr)
    }

    /// Checked multiplication — `None` on overflow.
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        // If the `i128` product overflows it is far outside 96 bits; otherwise
        // range-check it.
        self.repr
            .checked_mul(rhs.repr)
            .filter(|&p| Self::in_range(p))
            .map(|repr| Self { repr })
    }

    /// Checked division — `None` on division by zero or overflow (`MIN / -1`).
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        self.repr
            .checked_div(rhs.repr)
            .filter(|&q| Self::in_range(q))
            .map(|repr| Self { repr })
    }

    /// Checked remainder — `None` on division by zero.
    pub fn checked_rem(self, rhs: Self) -> Option<Self> {
        self.repr.checked_rem(rhs.repr).map(|repr| Self { repr })
    }

    /// Checked negation — `None` on overflow (`-MIN`).
    pub fn checked_neg(self) -> Option<Self> {
        Self::try_from_i128(-self.repr)
    }

    /// Wrapping (modular) addition.
    pub fn wrapping_add(self, rhs: Self) -> Self {
        Self::wrap(self.repr + rhs.repr)
    }

    /// Wrapping (modular) subtraction.
    pub fn wrapping_sub(self, rhs: Self) -> Self {
        Self::wrap(self.repr - rhs.repr)
    }

    /// Wrapping (modular) multiplication.
    pub fn wrapping_mul(self, rhs: Self) -> Self {
        // The low 96 bits of the product are the low 96 bits of the 128-bit
        // wrapping product of the bit patterns.
        Self::wrap((self.repr as u128).wrapping_mul(rhs.repr as u128) as i128)
    }

    /// Wrapping (modular) negation.
    pub fn wrapping_neg(self) -> Self {
        Self::wrap(-self.repr)
    }

    /// Wrapping division — wraps only on `MIN / -1` (to `MIN`); still panics on a
    /// zero divisor, like the primitive integers.
    pub fn wrapping_div(self, rhs: Self) -> Self {
        Self::wrap(self.repr.wrapping_div(rhs.repr))
    }

    /// Wrapping remainder — `MIN % -1` is `0`; still panics on a zero divisor.
    pub fn wrapping_rem(self, rhs: Self) -> Self {
        Self::wrap(self.repr.wrapping_rem(rhs.repr))
    }

    /// Saturating addition (clamps to [`MIN`](i96::MIN) / [`MAX`](i96::MAX)).
    pub fn saturating_add(self, rhs: Self) -> Self {
        self.checked_add(rhs)
            .unwrap_or(if rhs.repr >= 0 { Self::MAX } else { Self::MIN })
    }

    /// Saturating subtraction.
    pub fn saturating_sub(self, rhs: Self) -> Self {
        self.checked_sub(rhs)
            .unwrap_or(if rhs.repr >= 0 { Self::MIN } else { Self::MAX })
    }

    /// Saturating multiplication.
    pub fn saturating_mul(self, rhs: Self) -> Self {
        self.checked_mul(rhs)
            .unwrap_or(if (self.repr >= 0) == (rhs.repr >= 0) {
                Self::MAX
            } else {
                Self::MIN
            })
    }

    /// Wrapping addition with an overflow flag.
    pub fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        match self.checked_add(rhs) {
            Some(value) => (value, false),
            None => (self.wrapping_add(rhs), true),
        }
    }

    /// Wrapping multiplication with an overflow flag.
    pub fn overflowing_mul(self, rhs: Self) -> (Self, bool) {
        match self.checked_mul(rhs) {
            Some(value) => (value, false),
            None => (self.wrapping_mul(rhs), true),
        }
    }

    /// The absolute value; panics on `MIN` (whose negation overflows).
    pub fn abs(self) -> Self {
        self.checked_neg_if_negative()
            .expect("attempt to compute the absolute value with overflow")
    }

    fn checked_neg_if_negative(self) -> Option<Self> {
        if self.repr < 0 {
            self.checked_neg()
        } else {
            Some(self)
        }
    }

    /// Whether the value is negative.
    pub fn is_negative(self) -> bool {
        self.repr < 0
    }

    /// Whether the value is positive (`> 0`).
    pub fn is_positive(self) -> bool {
        self.repr > 0
    }
}

impl Add for i96 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        self.checked_add(rhs).expect("attempt to add with overflow")
    }
}

impl Sub for i96 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self.checked_sub(rhs)
            .expect("attempt to subtract with overflow")
    }
}

impl Mul for i96 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        self.checked_mul(rhs)
            .expect("attempt to multiply with overflow")
    }
}

impl Div for i96 {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        self.checked_div(rhs)
            .expect("attempt to divide by zero or with overflow")
    }
}

impl Rem for i96 {
    type Output = Self;
    fn rem(self, rhs: Self) -> Self {
        self.checked_rem(rhs)
            .expect("attempt to calculate the remainder with a divisor of zero")
    }
}

impl Neg for i96 {
    type Output = Self;
    fn neg(self) -> Self {
        self.checked_neg().expect("attempt to negate with overflow")
    }
}

impl From<i8> for i96 {
    fn from(value: i8) -> Self {
        Self::from_i64(value as i64)
    }
}

impl From<i16> for i96 {
    fn from(value: i16) -> Self {
        Self::from_i64(value as i64)
    }
}

impl From<i32> for i96 {
    fn from(value: i32) -> Self {
        Self::from_i64(value as i64)
    }
}

impl From<i64> for i96 {
    fn from(value: i64) -> Self {
        Self::from_i64(value)
    }
}

impl From<u32> for i96 {
    fn from(value: u32) -> Self {
        Self::from_i64(value as i64)
    }
}

impl From<i96> for i128 {
    fn from(value: i96) -> Self {
        value.repr
    }
}

impl Default for i96 {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for i96 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.repr, f)
    }
}

impl fmt::Debug for i96 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "i96({})", self.repr)
    }
}
