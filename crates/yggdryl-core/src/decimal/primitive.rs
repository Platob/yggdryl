//! The `decimal_primitive!` macro — the shared logic for the native-mantissa decimals
//! ([`Decimal32`](crate::Decimal32) / [`Decimal64`](crate::Decimal64) /
//! [`Decimal128`](crate::Decimal128)), stamped once per width. [`Decimal256`](crate::Decimal256)
//! is hand-written on the `i256` mantissa but mirrors this surface method-for-method.

/// Generates one fixed-width decimal type `$name` over a native integer mantissa `$int`
/// of `$bits` bits (`$width` bytes): `value = mantissa × 10^(−scale)`.
macro_rules! decimal_primitive {
    ($name:ident, $int:ty, $bits:literal, $width:literal) => {
        #[doc = concat!("A ", stringify!($bits), "-bit fixed-width decimal (mantissa `", stringify!($int), "` + scale).")]
        ///
        /// The value is `mantissa × 10^(−scale)`. It round-trips through
        #[doc = concat!("`", stringify!($width), " + 1` little-endian bytes (mantissa then the scale byte),")]
        /// compares and hashes by those bytes (rule 7), and converts to an [`f64`] /
        /// integer and between the decimal widths.
        ///
        #[doc = concat!("```")]
        #[doc = concat!("use yggdryl_core::{Decimal, ", stringify!($name), "};")]
        #[doc = concat!("let d = ", stringify!($name), "::new(12345, 2); // 123.45")]
        #[doc = concat!("assert_eq!(d.to_f64(), 123.45);")]
        #[doc = concat!("assert_eq!(d.to_i128(), Some(123));")]
        #[doc = concat!("assert_eq!(", stringify!($name), "::deserialize_bytes(&d.serialize_bytes()).unwrap(), d);")]
        #[doc = concat!("```")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name {
            mantissa: $int,
            scale: i8,
        }

        impl $name {
            /// The mantissa width in bits.
            pub const BITS: u32 = $bits;
            /// The zero value (scale 0).
            pub const ZERO: Self = Self { mantissa: 0, scale: 0 };

            #[doc = concat!("Creates a decimal from a `", stringify!($int), "` mantissa and a `scale`.")]
            pub fn new(mantissa: $int, scale: i8) -> Self {
                Self { mantissa, scale }
            }

            /// The unscaled integer mantissa.
            pub fn mantissa(&self) -> $int {
                self.mantissa
            }

            /// The scale (number of fractional decimal digits).
            pub fn scale(&self) -> i8 {
                self.scale
            }

            /// The mantissa widened to `i128` (always exact for this width).
            pub fn mantissa_i128(&self) -> i128 {
                self.mantissa as i128
            }

            /// Builds a decimal from an integer `value` (as the mantissa) and `scale`,
            /// checking the value fits the mantissa width.
            ///
            /// # Errors
            /// [`DecimalError::Overflow`](crate::DecimalError::Overflow) if `value` does
            /// not fit `$bits` bits.
            pub fn from_integer(value: i128, scale: i8) -> Result<Self, $crate::DecimalError> {
                let mantissa = <$int>::try_from(value)
                    .map_err(|_| $crate::DecimalError::Overflow { bits: $bits })?;
                Ok(Self { mantissa, scale })
            }

            /// Builds a decimal approximating `value` at `scale` (rounding the mantissa to
            /// the nearest integer). Saturates the mantissa on overflow.
            pub fn from_f64(value: f64, scale: i8) -> Self {
                let scaled = value * 10f64.powi(scale as i32);
                Self { mantissa: scaled.round() as $int, scale }
            }

            /// The value as an [`f64`] (`mantissa / 10^scale`; lossy for large mantissas).
            pub fn to_f64(&self) -> f64 {
                self.mantissa as f64 / 10f64.powi(self.scale as i32)
            }

            /// The integer part as an [`i128`], truncated toward zero, or `None` on overflow.
            pub fn to_i128(&self) -> Option<i128> {
                super::primitive::decimal_to_i128(self.mantissa as i128, self.scale)
            }

            /// Re-expresses the value at `new_scale`, scaling the mantissa by the matching
            /// power of ten (exact when widening the scale).
            ///
            /// # Errors
            /// [`DecimalError::Overflow`](crate::DecimalError::Overflow) if the rescaled
            /// mantissa no longer fits `$bits` bits.
            pub fn rescale(&self, new_scale: i8) -> Result<Self, $crate::DecimalError> {
                let mantissa = super::primitive::rescale_i128(self.mantissa as i128, self.scale, new_scale)
                    .and_then(|m| <$int>::try_from(m).ok())
                    .ok_or($crate::DecimalError::Overflow { bits: $bits })?;
                Ok(Self { mantissa, scale: new_scale })
            }

            /// Widens to a [`Decimal256`](crate::Decimal256) (same scale; always exact).
            pub fn to_decimal256(&self) -> $crate::Decimal256 {
                $crate::Decimal256::new($crate::i256::from_i128(self.mantissa as i128), self.scale)
            }

            /// The mantissa's little-endian bytes followed by the scale byte.
            pub fn serialize_bytes(&self) -> Vec<u8> {
                let mut out = self.mantissa.to_le_bytes().to_vec();
                out.push(self.scale as u8);
                out
            }

            #[doc = concat!("Reconstructs a decimal from `", stringify!($width), " + 1` little-endian bytes.")]
            ///
            /// # Errors
            /// [`DecimalError::InvalidByteLength`](crate::DecimalError::InvalidByteLength)
            /// if `bytes.len()` is not the expected length.
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, $crate::DecimalError> {
                const W: usize = $width;
                if bytes.len() != W + 1 {
                    return Err($crate::DecimalError::InvalidByteLength {
                        len: bytes.len(),
                        expected: W + 1,
                    });
                }
                let mantissa = <$int>::from_le_bytes(
                    bytes[..W].try_into().expect("W bytes"),
                );
                Ok(Self { mantissa, scale: bytes[W] as i8 })
            }
        }

        impl $crate::Decimal for $name {
            fn scale(&self) -> i8 {
                self.scale
            }
            fn bits(&self) -> u32 {
                $bits
            }
            fn mantissa_le_bytes(&self) -> Vec<u8> {
                self.mantissa.to_le_bytes().to_vec()
            }
            fn to_f64(&self) -> f64 {
                $name::to_f64(self)
            }
            fn to_i128(&self) -> Option<i128> {
                $name::to_i128(self)
            }
            fn serialize_bytes(&self) -> Vec<u8> {
                $name::serialize_bytes(self)
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", super::primitive::format_decimal(self.mantissa as i128, self.scale))
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($name), "({}e{})"), self.mantissa, -(self.scale as i32))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::ZERO
            }
        }
    };
}

pub(crate) use decimal_primitive;

/// The integer part of `mantissa × 10^(−scale)`, truncated toward zero, or `None` on
/// overflow. Shared by every width (the mantissa is pre-widened to `i128`; `Decimal256`
/// passes `None` when its mantissa itself exceeds `i128`).
pub(crate) fn decimal_to_i128(mantissa: i128, scale: i8) -> Option<i128> {
    if scale >= 0 {
        // A divisor beyond `i128` (scale >= 39) means `|value| < 1`, so the integer part is
        // `0` — not an overflow. Only the intermediate divisor overflowed, never the result.
        match 10i128.checked_pow(scale as u32) {
            Some(pow) => Some(mantissa / pow),
            None => Some(0),
        }
    } else {
        mantissa.checked_mul(10i128.checked_pow((-(scale as i32)) as u32)?)
    }
}

/// Rescales an `i128` mantissa from `old` to `new` scale, or `None` when *widening* the
/// scale overflows the width (narrowing can only shrink the mantissa, so it never fails).
pub(crate) fn rescale_i128(mantissa: i128, old: i8, new: i8) -> Option<i128> {
    let diff = new as i32 - old as i32;
    if diff >= 0 {
        mantissa.checked_mul(10i128.checked_pow(diff as u32)?)
    } else {
        // Dividing by a power beyond `i128` truncates the mantissa to `0` — never overflow.
        match 10i128.checked_pow((-diff) as u32) {
            Some(pow) => Some(mantissa / pow),
            None => Some(0),
        }
    }
}

/// Renders `mantissa × 10^(−scale)` as a decimal string (used by every width's `Display`).
pub(crate) fn format_decimal(mantissa: i128, scale: i8) -> String {
    if scale <= 0 {
        // Whole number: the mantissa's digits with `-scale` trailing zeros. Built as text so
        // no power-of-ten multiplication can overflow (`scale == i8::MIN`) or silently
        // saturate/drop the magnitude for large negative scales.
        if mantissa == 0 {
            return "0".to_string();
        }
        let zeros = (-(scale as i32)) as usize;
        let sign = if mantissa < 0 { "-" } else { "" };
        return format!("{sign}{}{}", mantissa.unsigned_abs(), "0".repeat(zeros));
    }
    let scale = scale as usize;
    let neg = mantissa < 0;
    let digits = mantissa.unsigned_abs().to_string();
    let digits = if digits.len() <= scale {
        format!("{:0>width$}", digits, width = scale + 1)
    } else {
        digits
    };
    let point = digits.len() - scale;
    let (int_part, frac_part) = digits.split_at(point);
    format!("{}{}.{}", if neg { "-" } else { "" }, int_part, frac_part)
}
