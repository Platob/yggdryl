//! [`F16`] — a **manual IEEE-754 half-precision float**, the native Rust equivalent of the
//! [`Float16`](super::Float16) element type (Rust has no built-in `f16` on stable, and the core is
//! dependency-free — so no `half` crate).
//!
//! Stored as the raw 16 half bits (`#[repr(transparent)]` over `u16`), little-endian on the wire.
//! Conversions to / from `f32` are correct round-to-nearest-even, handling zero, subnormals,
//! infinities, and NaN. It is `Copy` / `Default` / `Hash` and **bitwise** `Eq` (so it keys a map and
//! sits in a set), while [`PartialOrd`] compares by **value** (through `f32`), so `min` / `max` order
//! it like a float. It slots into the typed layer exactly like `f32` does — one 2-byte element.

use core::cmp::Ordering;
use core::fmt;

/// A 16-bit IEEE-754 **half-precision** float (the `Float16` backing value), stored as its raw
/// [`to_bits`](F16::to_bits) half bit pattern. `#[repr(transparent)]` over `u16`, so a `&[F16]` **is**
/// a `&[u16]` of the half bits — letting [`Float16`](super::Float16) encode / decode straight through
/// the source's vectorized `u16` array kernels.
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct F16(u16);

impl F16 {
    /// Positive zero (`+0.0`) — also the [`Default`].
    pub const ZERO: F16 = F16(0);

    /// The raw 16-bit half bit pattern.
    pub const fn to_bits(self) -> u16 {
        self.0
    }

    /// The `F16` for a raw 16-bit half bit pattern (the inverse of [`to_bits`](F16::to_bits)).
    pub const fn from_bits(bits: u16) -> F16 {
        F16(bits)
    }

    /// The `F16` nearest to `value`, rounding to nearest-even (subnormals, infinities, and NaN
    /// handled) — a value too large for the half range saturates to `±∞`, one too small to `±0`.
    ///
    /// ```
    /// use yggdryl_core::typed::fixedbyte::F16;
    ///
    /// assert_eq!(F16::from_f32(1.5).to_f32(), 1.5); // exact in half precision
    /// assert!(F16::from_f32(f32::INFINITY).to_f32().is_infinite());
    /// assert!(F16::from_f32(f32::NAN).to_f32().is_nan());
    /// ```
    pub fn from_f32(value: f32) -> F16 {
        F16(f32_to_f16(value))
    }

    /// The `f32` this half represents — an exact widening (every half value is representable in
    /// single precision).
    ///
    /// ```
    /// use yggdryl_core::typed::fixedbyte::F16;
    ///
    /// // The smallest positive subnormal half (2^-24) round-trips exactly through f32.
    /// let tiny = F16::from_f32(2f32.powi(-24));
    /// assert_eq!(tiny.to_f32(), 2f32.powi(-24));
    /// assert_ne!(tiny.to_bits(), 0); // not flushed to zero
    /// ```
    pub fn to_f32(self) -> f32 {
        f16_to_f32(self.0)
    }

    /// The 2 little-endian bytes of the half bit pattern.
    pub fn to_le_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }

    /// The `F16` from 2 little-endian bytes.
    pub fn from_le_bytes(bytes: [u8; 2]) -> F16 {
        F16(u16::from_le_bytes(bytes))
    }

    /// Whether the value is NaN (per float semantics — any non-canonical exponent-all-ones with a
    /// non-zero mantissa).
    pub fn is_nan(self) -> bool {
        (self.0 & 0x7c00) == 0x7c00 && (self.0 & 0x03ff) != 0
    }
}

impl PartialOrd for F16 {
    /// Orders by **value** through `f32` (so `-0.0 == 0.0` and NaN is unordered) — the ordering
    /// `min` / `max` use. Note this is deliberately looser than the **bitwise** [`Eq`]: two distinct
    /// bit patterns of the same value (`±0`) compare `Equal` here yet are unequal under `Eq`.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.to_f32().partial_cmp(&other.to_f32())
    }
}

impl fmt::Display for F16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_f32(), f)
    }
}

impl fmt::Debug for F16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_f32(), f)
    }
}

impl From<f32> for F16 {
    fn from(value: f32) -> F16 {
        F16::from_f32(value)
    }
}

impl From<F16> for f32 {
    fn from(value: F16) -> f32 {
        value.to_f32()
    }
}

/// `f32` → half bits, round-to-nearest-even (the reference scalar conversion — zero / subnormal /
/// overflow / NaN all handled).
fn f32_to_f16(value: f32) -> u16 {
    let x = value.to_bits();
    let sign = x & 0x8000_0000;
    let exp = x & 0x7f80_0000;
    let man = x & 0x007f_ffff;

    // All exponent bits set: infinity (man == 0) or NaN (keep the quiet bit + top mantissa bits).
    if exp == 0x7f80_0000 {
        let nan_bit = if man == 0 { 0 } else { 0x0200 };
        return ((sign >> 16) | 0x7c00 | nan_bit | (man >> 13)) as u16;
    }

    let half_sign = sign >> 16;
    // Unbias (f32 bias 127), then rebias for half (bias 15).
    let unbiased = ((exp >> 23) as i32) - 127;
    let half_exp = unbiased + 15;

    // Overflow → ±∞.
    if half_exp >= 0x1f {
        return (half_sign | 0x7c00) as u16;
    }

    // Underflow → subnormal or ±0.
    if half_exp <= 0 {
        if 14 - half_exp > 24 {
            return half_sign as u16; // too small even for a subnormal
        }
        let man = man | 0x0080_0000; // restore the hidden leading 1
        let mut half_man = man >> (14 - half_exp);
        let round_bit = 1 << (13 - half_exp);
        if (man & round_bit) != 0 && (man & (3 * round_bit - 1)) != 0 {
            half_man += 1;
        }
        return (half_sign | half_man) as u16;
    }

    let half_exp = (half_exp as u32) << 10;
    let half_man = man >> 13;
    let round_bit = 0x0000_1000;
    if (man & round_bit) != 0 && (man & (3 * round_bit - 1)) != 0 {
        ((half_sign | half_exp | half_man) + 1) as u16
    } else {
        (half_sign | half_exp | half_man) as u16
    }
}

/// Half bits → `f32` (exact — the reference scalar widening; signed zero / subnormal / infinity /
/// NaN all handled).
fn f16_to_f32(bits: u16) -> f32 {
    // Signed zero.
    if bits & 0x7fff == 0 {
        return f32::from_bits((bits as u32) << 16);
    }

    let half_sign = (bits & 0x8000) as u32;
    let half_exp = (bits & 0x7c00) as u32;
    let half_man = (bits & 0x03ff) as u32;

    // Infinity / NaN.
    if half_exp == 0x7c00 {
        if half_man == 0 {
            return f32::from_bits((half_sign << 16) | 0x7f80_0000);
        }
        return f32::from_bits((half_sign << 16) | 0x7fc0_0000 | (half_man << 13));
    }

    let sign = half_sign << 16;
    let unbiased = ((half_exp as i32) >> 10) - 15;

    // Subnormal — normalize by shifting the mantissa and adjusting the exponent.
    if half_exp == 0 {
        let e = (half_man as u16).leading_zeros() - 6;
        let exp = (127 - 15 - e) << 23;
        let man = (half_man << (14 + e)) & 0x007f_ffff;
        return f32::from_bits(sign | exp | man);
    }

    let exp = ((unbiased + 127) as u32) << 23;
    let man = half_man << 13;
    f32::from_bits(sign | exp | man)
}
