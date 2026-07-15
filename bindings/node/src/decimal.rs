//! The `yggdryl.decimal` namespace — the fixed-width **scaled-decimal value types** `D32` / `D64`
//! / `D128` / `D256`, mirroring `yggdryl_core::io::fixed`'s `Decimal<B>` family. Each value is a
//! coefficient integer scaled by a power of ten (`value = coefficient × 10^-scale`), with checked
//! arithmetic, true numeric ordering, value identity (`2.5` equals `2.50`, hashes equal), a byte
//! codec, conversions to/from integers and floats, and casts between the widths.
//!
//! The coefficient marshals as a JS `bigint` for every width — including `D256`'s 256-bit
//! coefficient — through its decimal digits (a `bigint` is arbitrary-precision; a `number` is not).
//! The constructor range-checks it in the core, so the guided error reads identically across Node,
//! Python, and Rust.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer};
use napi_derive::napi;

use yggdryl_core::io::fixed::{Dec128, Dec256, Dec32, Dec64};

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// A two's-complement little-endian coefficient (any width) as a sign-magnitude napi `BigInt`
/// (whose `words` are little-endian `u64`s). The sign is split off first, so a negative
/// coefficient — including the two's-complement minimum — round-trips exactly.
fn le_to_bigint(le: &[u8]) -> BigInt {
    let negative = le.last().is_some_and(|b| b & 0x80 != 0);
    // Two's-complement negate (invert + 1) yields the magnitude bytes; the width's minimum reads
    // back as exactly 2^(bits-1), the correct magnitude.
    let magnitude: Vec<u8> = if negative {
        let mut bytes = le.to_vec();
        for byte in &mut bytes {
            *byte = !*byte;
        }
        let mut carry = 1u16;
        for byte in &mut bytes {
            let value = *byte as u16 + carry;
            *byte = value as u8;
            carry = value >> 8;
        }
        bytes
    } else {
        le.to_vec()
    };
    let mut words: Vec<u64> = magnitude
        .chunks(8)
        .map(|chunk| {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            u64::from_le_bytes(word)
        })
        .collect();
    while words.len() > 1 && *words.last().unwrap() == 0 {
        words.pop(); // trim high zero words but keep >= 1 (an empty `words` is invalid)
    }
    let is_zero = words.iter().all(|&word| word == 0);
    BigInt {
        sign_bit: negative && !is_zero,
        words,
    }
}

/// A napi `BigInt`'s decimal-integer string (`"-12345"`) — base-2^64 words divided down to base
/// ten. Any magnitude is representable; an out-of-range value is rejected later by the core
/// constructor (so the guided message matches the other bindings).
fn bigint_to_decimal_string(big: &BigInt) -> String {
    if big.words.iter().all(|&word| word == 0) {
        return "0".to_string();
    }
    let mut words = big.words.clone();
    let mut digits = Vec::new();
    while words.iter().any(|&word| word != 0) {
        let mut remainder: u128 = 0;
        for word in words.iter_mut().rev() {
            let current = (remainder << 64) | (*word as u128);
            *word = (current / 10) as u64;
            remainder = current % 10;
        }
        digits.push(b'0' + remainder as u8);
    }
    if big.sign_bit {
        digits.push(b'-');
    }
    digits.reverse();
    String::from_utf8(digits).expect("ascii digits")
}

/// Generates the napi wrapper for one decimal width.
macro_rules! napi_decimal {
    ($Wrapper:ident, $core:ty) => {
        #[doc = concat!("A fixed-width `", stringify!($Wrapper), "` decimal — a coefficient integer × 10^(−scale).")]
        #[napi(namespace = "decimal")]
        pub struct $Wrapper {
            pub(crate) inner: $core,
        }

        #[napi(namespace = "decimal")]
        impl $Wrapper {
            /// Builds a decimal from an integer `coefficient` (a `bigint`) and `scale` (default
            /// `0`), throwing a guided error if the coefficient does not fit the width.
            #[napi(constructor)]
            pub fn new(coefficient: BigInt, scale: Option<i8>) -> napi::Result<Self> {
                <$core>::from_coeff_str(&bigint_to_decimal_string(&coefficient), scale.unwrap_or(0))
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Parses a decimal literal (`"-123.45"`), throwing on a malformed string.
            #[napi(factory)]
            pub fn from_string(text: String) -> napi::Result<Self> {
                text.parse::<$core>().map(|inner| Self { inner }).map_err(to_error)
            }

            /// The decimal nearest `value` at `scale` (default `0`), throwing for a non-finite
            /// float or an out-of-range result.
            #[napi(factory)]
            pub fn from_float(value: f64, scale: Option<i8>) -> napi::Result<Self> {
                <$core>::from_f64(value, scale.unwrap_or(0))
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// The unscaled integer coefficient (a `bigint`, any width).
            #[napi(getter)]
            pub fn coefficient(&self) -> BigInt {
                le_to_bigint(&self.inner.coefficient_le_bytes())
            }

            /// The scale (number of fractional decimal digits).
            #[napi(getter)]
            pub fn scale(&self) -> i32 {
                self.inner.scale() as i32
            }

            /// The value's precision — its significant-digit count.
            #[napi(getter)]
            pub fn precision(&self) -> u32 {
                self.inner.precision()
            }

            /// The width's maximum precision (`9`/`18`/`38`/`76`).
            #[napi(getter)]
            pub fn max_precision(&self) -> u32 {
                <$core>::max_precision() as u32
            }

            /// The coefficient width in bits (`32`/`64`/`128`/`256`).
            #[napi(getter)]
            pub fn bits(&self) -> u32 {
                <$core>::bit_width()
            }

            /// Whether the value is exactly zero.
            #[napi]
            pub fn is_zero(&self) -> bool {
                self.inner.is_zero()
            }
            /// Whether the value is strictly negative.
            #[napi]
            pub fn is_negative(&self) -> bool {
                self.inner.is_negative()
            }
            /// Whether the value is strictly positive.
            #[napi]
            pub fn is_positive(&self) -> bool {
                self.inner.is_positive()
            }

            /// The value as a float (lossy beyond 53 bits of mantissa).
            #[napi]
            pub fn to_float(&self) -> f64 {
                self.inner.to_f64()
            }

            /// The exact integer value (a `bigint`), throwing if it has a fractional part or
            /// exceeds `i128`.
            #[napi]
            pub fn to_int(&self) -> napi::Result<i128> {
                self.inner.to_i128().map_err(to_error)
            }

            /// The integer part (a `bigint`), truncated toward zero — the JS analogue of Python's
            /// `int(decimal)`; carries any width via the coefficient digits.
            #[napi]
            pub fn to_big_int(&self) -> BigInt {
                le_to_bigint(&self.inner.trunc().coefficient_le_bytes())
            }

            /// This value re-expressed at `newScale`, exactly — throwing if lowering the scale
            /// would drop non-zero digits, or on overflow.
            #[napi]
            pub fn rescale(&self, new_scale: i8) -> napi::Result<Self> {
                self.inner.rescale(new_scale).map(|inner| Self { inner }).map_err(to_error)
            }

            /// This value at `newScale`, rounding dropped digits half-away-from-zero.
            #[napi]
            pub fn round_to_scale(&self, new_scale: i8) -> napi::Result<Self> {
                self.inner.round_to_scale(new_scale).map(|inner| Self { inner }).map_err(to_error)
            }

            /// This value at `newScale`, truncating dropped digits toward zero.
            #[napi]
            pub fn trunc_to_scale(&self, new_scale: i8) -> napi::Result<Self> {
                self.inner.trunc_to_scale(new_scale).map(|inner| Self { inner }).map_err(to_error)
            }

            /// The integer part, truncated toward zero (scale `0`).
            #[napi]
            pub fn trunc(&self) -> Self {
                Self { inner: self.inner.trunc() }
            }

            /// The value with trailing fractional zeros stripped (`2.50` → `2.5`).
            #[napi]
            pub fn normalized(&self) -> Self {
                Self { inner: self.inner.normalized() }
            }

            /// `self + other`, throwing on overflow.
            #[napi]
            pub fn add(&self, other: &$Wrapper) -> napi::Result<Self> {
                self.inner.checked_add(&other.inner).map(|inner| Self { inner }).map_err(to_error)
            }
            /// `self - other`, throwing on overflow.
            #[napi]
            pub fn sub(&self, other: &$Wrapper) -> napi::Result<Self> {
                self.inner.checked_sub(&other.inner).map(|inner| Self { inner }).map_err(to_error)
            }
            /// `self * other`, throwing on overflow.
            #[napi]
            pub fn mul(&self, other: &$Wrapper) -> napi::Result<Self> {
                self.inner.checked_mul(&other.inner).map(|inner| Self { inner }).map_err(to_error)
            }
            /// `self % other` (scales aligned), throwing on divide-by-zero.
            #[napi]
            pub fn rem(&self, other: &$Wrapper) -> napi::Result<Self> {
                self.inner.checked_rem(&other.inner).map(|inner| Self { inner }).map_err(to_error)
            }
            /// `self / other` at `resultScale`, throwing on divide-by-zero or overflow.
            #[napi]
            pub fn div(&self, other: &$Wrapper, result_scale: i8) -> napi::Result<Self> {
                self.inner
                    .checked_div(&other.inner, result_scale)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            /// `-self`, throwing on overflow (the two's-complement minimum).
            #[napi]
            pub fn neg(&self) -> napi::Result<Self> {
                self.inner.checked_neg().map(|inner| Self { inner }).map_err(to_error)
            }
            /// The absolute value, throwing on overflow.
            #[napi]
            pub fn abs(&self) -> napi::Result<Self> {
                self.inner.checked_abs().map(|inner| Self { inner }).map_err(to_error)
            }

            /// This value cast to `d32`, throwing if it does not fit.
            #[napi]
            pub fn to_d32(&self) -> napi::Result<D32> {
                self.inner.cast::<Dec32>().map(|inner| D32 { inner }).map_err(to_error)
            }
            /// This value cast to `d64`, throwing if it does not fit.
            #[napi]
            pub fn to_d64(&self) -> napi::Result<D64> {
                self.inner.cast::<Dec64>().map(|inner| D64 { inner }).map_err(to_error)
            }
            /// This value cast to `d128`, throwing if it does not fit.
            #[napi]
            pub fn to_d128(&self) -> napi::Result<D128> {
                self.inner.cast::<Dec128>().map(|inner| D128 { inner }).map_err(to_error)
            }
            /// This value cast to `d256` (always exact from a narrower width).
            #[napi]
            pub fn to_d256(&self) -> napi::Result<D256> {
                self.inner.cast::<Dec256>().map(|inner| D256 { inner }).map_err(to_error)
            }

            /// The canonical byte encoding — `[scale][coefficient little-endian]` of the
            /// normalized value.
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a decimal from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                <$core>::deserialize_bytes(&bytes).map(|inner| Self { inner }).map_err(to_error)
            }

            /// Value equality (`2.5` equals `2.50`).
            #[napi]
            pub fn equals(&self, other: &$Wrapper) -> bool {
                self.inner == other.inner
            }

            /// True numeric comparison: `-1` / `0` / `1` for `self` less than / equal / greater
            /// than `other`.
            #[napi]
            pub fn compare_to(&self, other: &$Wrapper) -> i32 {
                self.inner.cmp(&other.inner) as i32
            }

            /// A content hash consistent with [`equals`](Self::equals) (equal values hash equal).
            #[napi]
            pub fn hash_code(&self) -> i32 {
                java_hash(&self.inner)
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self { inner: self.inner }
            }

            /// The value in plain decimal form, e.g. `"123.45"`.
            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                self.inner.to_string()
            }
        }
    };
}

napi_decimal!(D32, yggdryl_core::io::fixed::D32);
napi_decimal!(D64, yggdryl_core::io::fixed::D64);
napi_decimal!(D128, yggdryl_core::io::fixed::D128);
napi_decimal!(D256, yggdryl_core::io::fixed::D256);
