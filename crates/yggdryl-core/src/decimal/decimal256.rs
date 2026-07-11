//! [`Decimal256`] — a 256-bit fixed-width decimal (`i256` mantissa + scale).

use super::primitive::{decimal_to_i128, format_decimal};
use crate::{i256, Decimal, Decimal128, DecimalError};

/// A 256-bit fixed-width decimal: an [`i256`] mantissa scaled by a power of ten
/// (`value = mantissa × 10^(−scale)`), matching Arrow's `Decimal256`. It mirrors the
/// narrower widths ([`Decimal32`](crate::Decimal32) … [`Decimal128`]) method-for-method,
/// over the 256-bit mantissa, and round-trips through `32 + 1` little-endian bytes.
///
/// ```
/// use yggdryl_core::{Decimal, Decimal128, Decimal256, i256};
///
/// let big = Decimal256::new(i256::from_i128(12_345), 2); // 123.45
/// assert_eq!(big.to_f64(), 123.45);
/// assert_eq!(big.try_to_decimal128().unwrap(), Decimal128::new(12_345, 2));
/// assert_eq!(Decimal256::deserialize_bytes(&big.serialize_bytes()).unwrap(), big);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal256 {
    mantissa: i256,
    scale: i8,
}

/// `10^n` as an [`i256`], or `None` on overflow (built by repeated checked multiplication
/// so any exponent up to the 256-bit limit works).
fn i256_ten_pow(n: u32) -> Option<i256> {
    let ten = i256::from_i128(10);
    let mut acc = i256::from_i128(1);
    for _ in 0..n {
        acc = acc.checked_mul(ten)?;
    }
    Some(acc)
}

/// The `i256` mantissa as an `f64` (exact when it fits `i128`, else parsed from its
/// base-10 text — lossy but bounded, as `f64` cannot hold 256-bit magnitudes anyway).
fn i256_to_f64(value: i256) -> f64 {
    match value.to_i128() {
        Some(v) => v as f64,
        None => value.to_string().parse().unwrap_or(f64::INFINITY),
    }
}

impl Decimal256 {
    /// The mantissa width in bits.
    pub const BITS: u32 = 256;

    /// Creates a decimal from an [`i256`] mantissa and a `scale`.
    pub fn new(mantissa: i256, scale: i8) -> Self {
        Self { mantissa, scale }
    }

    /// The unscaled integer mantissa.
    pub fn mantissa(&self) -> i256 {
        self.mantissa
    }

    /// The scale (number of fractional decimal digits).
    pub fn scale(&self) -> i8 {
        self.scale
    }

    /// Builds a decimal from an `i128` `value` (as the mantissa) and `scale`.
    pub fn from_integer(value: i128, scale: i8) -> Self {
        Self {
            mantissa: i256::from_i128(value),
            scale,
        }
    }

    /// Builds a decimal approximating `value` at `scale` (via [`Decimal128`] for the
    /// common in-range case; large magnitudes saturate).
    pub fn from_f64(value: f64, scale: i8) -> Self {
        let scaled = value * 10f64.powi(scale as i32);
        Self {
            mantissa: i256::from_i128(scaled.round() as i128),
            scale,
        }
    }

    /// The value as an [`f64`] (`mantissa / 10^scale`; lossy for large mantissas).
    pub fn to_f64(&self) -> f64 {
        i256_to_f64(self.mantissa) / 10f64.powi(self.scale as i32)
    }

    /// The integer part as an [`i128`], truncated toward zero, or `None` if the mantissa
    /// (or the integer part) exceeds `i128`.
    pub fn to_i128(&self) -> Option<i128> {
        decimal_to_i128(self.mantissa.to_i128()?, self.scale)
    }

    /// Re-expresses the value at `new_scale`, scaling the mantissa by the matching power
    /// of ten.
    ///
    /// # Errors
    /// [`DecimalError::Overflow`] if the rescaled mantissa exceeds 256 bits.
    pub fn rescale(&self, new_scale: i8) -> Result<Self, DecimalError> {
        let diff = new_scale as i32 - self.scale as i32;
        let mantissa = if diff >= 0 {
            self.mantissa
                .checked_mul(i256_ten_pow(diff as u32).ok_or(DecimalError::Overflow { bits: 256 })?)
                .ok_or(DecimalError::Overflow { bits: 256 })?
        } else {
            self.mantissa
                / i256_ten_pow((-diff) as u32).ok_or(DecimalError::Overflow { bits: 256 })?
        };
        Ok(Self {
            mantissa,
            scale: new_scale,
        })
    }

    /// Narrows to a [`Decimal128`] if the mantissa fits `i128`.
    ///
    /// # Errors
    /// [`DecimalError::Overflow`] if the mantissa exceeds `i128`.
    pub fn try_to_decimal128(&self) -> Result<Decimal128, DecimalError> {
        let mantissa = self
            .mantissa
            .to_i128()
            .ok_or(DecimalError::Overflow { bits: 128 })?;
        Ok(Decimal128::new(mantissa, self.scale))
    }

    /// The mantissa's 32 little-endian bytes followed by the scale byte.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut out = self.mantissa.to_le_bytes().to_vec();
        out.push(self.scale as u8);
        out
    }

    /// Reconstructs a decimal from `32 + 1` little-endian bytes.
    ///
    /// # Errors
    /// [`DecimalError::InvalidByteLength`] if `bytes.len()` is not `33`.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, DecimalError> {
        if bytes.len() != 33 {
            return Err(DecimalError::InvalidByteLength {
                len: bytes.len(),
                expected: 33,
            });
        }
        let mantissa = i256::from_le_bytes(bytes[..32].try_into().expect("32 bytes"));
        Ok(Self {
            mantissa,
            scale: bytes[32] as i8,
        })
    }
}

impl Decimal for Decimal256 {
    fn scale(&self) -> i8 {
        self.scale
    }
    fn bits(&self) -> u32 {
        256
    }
    fn mantissa_le_bytes(&self) -> Vec<u8> {
        self.mantissa.to_le_bytes().to_vec()
    }
    fn to_f64(&self) -> f64 {
        Decimal256::to_f64(self)
    }
    fn to_i128(&self) -> Option<i128> {
        Decimal256::to_i128(self)
    }
    fn serialize_bytes(&self) -> Vec<u8> {
        Decimal256::serialize_bytes(self)
    }
}

impl core::fmt::Display for Decimal256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.mantissa.to_i128() {
            Some(m) => write!(f, "{}", format_decimal(m, self.scale)),
            // Beyond i128: fall back to the integer text (scale 0 magnitudes are rare here).
            None => write!(f, "{}e{}", self.mantissa, -(self.scale as i32)),
        }
    }
}

impl core::fmt::Debug for Decimal256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Decimal256({}e{})", self.mantissa, -(self.scale as i32))
    }
}

impl Default for Decimal256 {
    fn default() -> Self {
        Self::new(i256::from_i128(0), 0)
    }
}
