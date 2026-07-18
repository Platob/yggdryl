//! [`I256`] — a **256-bit signed integer**, the native Rust equivalent of the `Decimal256` element
//! type (Rust has no built-in `i256`).
//!
//! Stored as a signed high 128 bits + an unsigned low 128 bits (two's complement, so the full value
//! is `hi * 2^128 + lo`), little-endian on the wire. It is `Copy` / `Default` / `Ord` / `Hash` and
//! formats as a signed decimal, so it slots into the typed layer exactly like `i128` does.

use core::cmp::Ordering;
use core::fmt;

/// A 256-bit signed integer (the `Decimal256` backing value).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct I256 {
    /// The signed high 128 bits.
    hi: i128,
    /// The unsigned low 128 bits.
    lo: u128,
}

impl I256 {
    /// Zero.
    pub const ZERO: I256 = I256 { hi: 0, lo: 0 };

    /// The `I256` for a native `i128` (sign-extended).
    pub const fn from_i128(value: i128) -> I256 {
        I256 {
            hi: if value < 0 { -1 } else { 0 },
            lo: value as u128,
        }
    }

    /// The value as an `i128` when it fits, else `None`.
    pub fn to_i128(self) -> Option<i128> {
        match self.hi {
            0 if self.lo <= i128::MAX as u128 => Some(self.lo as i128),
            -1 if self.lo >= i128::MIN as u128 => Some(self.lo as i128),
            _ => None,
        }
    }

    /// The 32 little-endian bytes (`lo` then `hi`).
    pub fn to_le_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(&self.lo.to_le_bytes());
        out[16..].copy_from_slice(&self.hi.to_le_bytes());
        out
    }

    /// The `I256` from 32 little-endian bytes.
    pub fn from_le_bytes(bytes: [u8; 32]) -> I256 {
        let lo = u128::from_le_bytes(bytes[..16].try_into().expect("16 bytes"));
        let hi = i128::from_le_bytes(bytes[16..].try_into().expect("16 bytes"));
        I256 { hi, lo }
    }

    /// Whether the value is negative.
    pub fn is_negative(self) -> bool {
        self.hi < 0
    }

    /// The magnitude's little-endian bytes (`self` negated in two's complement when negative).
    fn magnitude_le(self) -> [u8; 32] {
        let bytes = self.to_le_bytes();
        if !self.is_negative() {
            return bytes;
        }
        // Two's-complement negate: bitwise-not, then +1, across the 32 bytes.
        let mut out = [0u8; 32];
        let mut carry = 1u16;
        for (o, b) in out.iter_mut().zip(bytes.iter()) {
            let sum = (!b) as u16 + carry;
            *o = sum as u8;
            carry = sum >> 8;
        }
        out
    }
}

impl Ord for I256 {
    fn cmp(&self, other: &Self) -> Ordering {
        // Signed high word first, then the unsigned low word.
        self.hi.cmp(&other.hi).then(self.lo.cmp(&other.lo))
    }
}

impl PartialOrd for I256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<i128> for I256 {
    fn from(value: i128) -> I256 {
        I256::from_i128(value)
    }
}

impl fmt::Display for I256 {
    /// Formats as a signed decimal — long-division of the magnitude by 10, digit by digit.
    ///
    /// ```
    /// use yggdryl_core::typed::fixedbyte::I256;
    ///
    /// assert_eq!(I256::from_i128(-12345).to_string(), "-12345");
    /// assert_eq!(I256::from_i128(i128::MAX).to_string(), i128::MAX.to_string());
    /// assert_eq!(I256::ZERO.to_string(), "0");
    /// // A value beyond i128 round-trips through the 256-bit formatter (4 * 2^128).
    /// let big = { let mut b = [0u8; 32]; b[16] = 4; I256::from_le_bytes(b) };
    /// assert!(big.to_i128().is_none());
    /// assert!(big.to_string().len() >= 39 && big.to_string().starts_with('1'));
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == I256::ZERO {
            return f.write_str("0");
        }
        // Magnitude as little-endian 32-bit limbs, most-significant last.
        let bytes = self.magnitude_le();
        let mut limbs = [0u32; 8];
        for (i, limb) in limbs.iter_mut().enumerate() {
            *limb = u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        }
        let mut digits = Vec::with_capacity(80);
        while limbs.iter().any(|&l| l != 0) {
            let mut remainder = 0u64;
            for limb in limbs.iter_mut().rev() {
                let current = (remainder << 32) | u64::from(*limb);
                *limb = (current / 10) as u32;
                remainder = current % 10;
            }
            digits.push(b'0' + remainder as u8);
        }
        if self.is_negative() {
            f.write_str("-")?;
        }
        for &digit in digits.iter().rev() {
            f.write_str(core::str::from_utf8(&[digit]).expect("ascii digit"))?;
        }
        Ok(())
    }
}
