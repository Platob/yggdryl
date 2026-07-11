//! The `yggdryl.decimal` namespace — fixed-width decimals.
//!
//! Exposes the four Arrow decimal widths (`Decimal32` / `Decimal64` / `Decimal128` /
//! `Decimal256`), mirroring `yggdryl_core`'s `decimal` module. Each is an integer
//! **mantissa** scaled by a power of ten (`value = mantissa × 10^(−scale)`), with value
//! semantics (equal iff `serializeBytes` are equal), a byte round-trip, `f64` / integer
//! conversion, rescaling, and widening / narrowing between the widths.
//!
//! Mantissa marshalling follows the usual JS mapping: `Decimal32`'s `i32` mantissa is a
//! `number`, while the wider `i64` / `i128` / `i256` mantissas marshal as `bigint` (a JS
//! `number` cannot hold them without precision loss). The constructor range-checks the
//! mantissa against the width and throws a guided `Error` naming the accepted range when it
//! does not fit (`CLAUDE.md` rule 12); `toI128` likewise returns a `bigint` (or `null`).
//!
//! The direct narrow-to-narrow widenings (`Decimal32` → `Decimal64`, etc.) are Rust-only
//! `From` conveniences; the bindings expose the value-preserving ladder as `toDecimal256()`
//! (on the three narrow widths) and the fallible narrowing `Decimal256.tryToDecimal128()`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer};
use napi_derive::napi;

use yggdryl_core::i256;

/// Maps any core error to a thrown JS `Error` (its guided text).
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

/// Converts a two's-complement [`i256`] into a sign-magnitude napi `BigInt` (whose `words`
/// are little-endian u64s). The sign is split off first, so a negative mantissa — including
/// `i256::MIN` — round-trips exactly.
fn i256_to_bigint(v: i256) -> BigInt {
    let negative = v.to_parts().1 < 0;
    // wrapping_neg(MIN) == MIN, whose 32 bytes read *unsigned* are exactly 2^255 — the
    // correct magnitude — so reading the bytes below as unsigned is what makes MIN work.
    let magnitude = if negative { v.wrapping_neg() } else { v };
    let mut words: Vec<u64> = magnitude
        .to_le_bytes()
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    while words.len() > 1 && *words.last().unwrap() == 0 {
        words.pop(); // trim high zero words but keep >=1 (empty words panics get_*)
    }
    BigInt {
        sign_bit: negative,
        words,
    }
}

/// Converts a napi `BigInt` into an [`i256`], throwing a guided error if it exceeds the
/// signed 256-bit range (a 4-word magnitude can reach 2^256-1, but `i256` holds only
/// `-2^255 ..= 2^255-1`).
fn bigint_to_i256(big: BigInt) -> napi::Result<i256> {
    let out_of_range = || {
        to_error(format!(
            "mantissa is out of range for decimal256; expected {}..={}",
            i256::MIN,
            i256::MAX
        ))
    };
    let BigInt { sign_bit, words } = big;
    if words.len() > 4 {
        return Err(out_of_range());
    }
    let mut bytes = [0u8; 32];
    for (i, w) in words.iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    let raw = i256::from_le_bytes(bytes);
    // raw's high i128 < 0  <=>  magnitude >= 2^255 (top bit set).
    let value = if raw.to_parts().1 >= 0 {
        if sign_bit {
            raw.wrapping_neg()
        } else {
            raw
        }
    } else if sign_bit && raw == i256::MIN {
        // magnitude == 2^255 with a minus sign is the one in-range top-bit case: i256::MIN.
        i256::MIN
    } else {
        return Err(out_of_range());
    };
    Ok(value)
}

/// Generates the napi wrapper for one decimal width whose mantissa marshals as a `bigint`
/// (`Decimal64` / `Decimal128` — `i64` / `i128`). The constructor reads the `bigint` as an
/// `i128` and routes the range check through the core `from_integer` (rule-12 parity).
macro_rules! napi_decimal_bigint {
    ($( ($name:ident, $int:ty) ),+ $(,)?) => {
        $(
            #[doc = concat!("A fixed-width decimal over an `", stringify!($int), "` mantissa (× 10^(−scale)).")]
            #[napi(namespace = "decimal")]
            pub struct $name {
                pub(crate) inner: yggdryl_core::$name,
            }

            #[napi(namespace = "decimal")]
            impl $name {
                /// Builds a decimal from an integer `mantissa` (a `bigint`) and `scale`
                /// (default 0), throwing a guided error if the mantissa does not fit the
                /// width. The range check routes through the core `from_integer`, so the
                /// overflow message reads identically to the Python binding (rule 12).
                #[napi(constructor)]
                pub fn new(mantissa: BigInt, scale: Option<i8>) -> napi::Result<Self> {
                    let (value, lossless) = mantissa.get_i128();
                    if !lossless {
                        return Err(to_error(
                            "mantissa exceeds 128 bits; use a wider decimal (Decimal256)",
                        ));
                    }
                    yggdryl_core::$name::from_integer(value, scale.unwrap_or(0))
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Builds a decimal approximating `value` at `scale` (rounding the mantissa).
                #[napi(factory)]
                pub fn from_f64(value: f64, scale: Option<i8>) -> Self {
                    Self { inner: yggdryl_core::$name::from_f64(value, scale.unwrap_or(0)) }
                }

                /// The unscaled integer mantissa (a `bigint`).
                #[napi(getter)]
                pub fn mantissa(&self) -> BigInt {
                    BigInt::from(self.inner.mantissa())
                }

                /// The scale (number of fractional decimal digits).
                #[napi(getter)]
                pub fn scale(&self) -> i8 {
                    self.inner.scale()
                }

                /// The mantissa width in bits.
                #[napi(getter)]
                pub fn bits(&self) -> u32 {
                    yggdryl_core::$name::BITS
                }

                /// The value as a float (`mantissa / 10^scale`; lossy for large mantissas).
                #[napi]
                pub fn to_f64(&self) -> f64 {
                    self.inner.to_f64()
                }

                /// The integer part (a `bigint`), truncated toward zero, or `null` on overflow.
                #[napi]
                pub fn to_i128(&self) -> Option<BigInt> {
                    self.inner.to_i128().map(BigInt::from)
                }

                /// Re-expresses the value at `newScale`, throwing a guided error on overflow.
                #[napi]
                pub fn rescale(&self, new_scale: i8) -> napi::Result<Self> {
                    self.inner.rescale(new_scale).map(|inner| Self { inner }).map_err(to_error)
                }

                /// Widens to a `Decimal256` (same scale; always exact).
                #[napi]
                pub fn to_decimal256(&self) -> Decimal256 {
                    Decimal256 { inner: self.inner.to_decimal256() }
                }

                /// The value's bytes: the mantissa's little-endian bytes then the scale byte.
                #[napi]
                pub fn serialize_bytes(&self) -> Buffer {
                    self.inner.serialize_bytes().into()
                }

                /// Reconstructs a decimal from its serialised bytes.
                #[napi(factory)]
                pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                    yggdryl_core::$name::deserialize_bytes(bytes.as_ref())
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Content equality.
                #[napi]
                pub fn equals(&self, other: &$name) -> bool {
                    self.inner == other.inner
                }

                /// Java-style `i32` content hash.
                #[napi]
                pub fn hash_code(&self) -> i32 {
                    java_hash(&self.inner)
                }

                /// The decimal value in plain form, e.g. `"123.45"`.
                #[napi(js_name = "toString")]
                pub fn text(&self) -> String {
                    self.inner.to_string()
                }
            }
        )+
    };
}

napi_decimal_bigint! {
    (Decimal64, i64),
    (Decimal128, i128),
}

/// A fixed-width `decimal32` (32-bit integer mantissa × 10^(−scale)). Its `i32` mantissa
/// marshals as a JS `number`.
#[napi(namespace = "decimal")]
pub struct Decimal32 {
    pub(crate) inner: yggdryl_core::Decimal32,
}

#[napi(namespace = "decimal")]
impl Decimal32 {
    /// Builds a decimal from a whole-number `mantissa` and `scale` (default 0), throwing a
    /// guided error if the mantissa is fractional or does not fit the width. The range check
    /// routes through the core `from_integer`, so the overflow message reads identically to
    /// the Python binding (rule 12).
    #[napi(constructor)]
    pub fn new(mantissa: f64, scale: Option<i8>) -> napi::Result<Self> {
        if !mantissa.is_finite() || mantissa.fract() != 0.0 {
            return Err(to_error(format!(
                "mantissa {mantissa} is not a whole number; decimal32 needs a whole number"
            )));
        }
        yggdryl_core::Decimal32::from_integer(mantissa as i128, scale.unwrap_or(0))
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Builds a decimal approximating `value` at `scale` (rounding the mantissa).
    #[napi(factory)]
    pub fn from_f64(value: f64, scale: Option<i8>) -> Self {
        Self {
            inner: yggdryl_core::Decimal32::from_f64(value, scale.unwrap_or(0)),
        }
    }

    /// The unscaled integer mantissa (a `number`).
    #[napi(getter)]
    pub fn mantissa(&self) -> i32 {
        self.inner.mantissa()
    }

    /// The scale (number of fractional decimal digits).
    #[napi(getter)]
    pub fn scale(&self) -> i8 {
        self.inner.scale()
    }

    /// The mantissa width in bits.
    #[napi(getter)]
    pub fn bits(&self) -> u32 {
        yggdryl_core::Decimal32::BITS
    }

    /// The value as a float (`mantissa / 10^scale`).
    #[napi]
    pub fn to_f64(&self) -> f64 {
        self.inner.to_f64()
    }

    /// The integer part (a `bigint`), truncated toward zero, or `null` on overflow.
    #[napi]
    pub fn to_i128(&self) -> Option<BigInt> {
        self.inner.to_i128().map(BigInt::from)
    }

    /// Re-expresses the value at `newScale`, throwing a guided error on overflow.
    #[napi]
    pub fn rescale(&self, new_scale: i8) -> napi::Result<Self> {
        self.inner
            .rescale(new_scale)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Widens to a `Decimal256` (same scale; always exact).
    #[napi]
    pub fn to_decimal256(&self) -> Decimal256 {
        Decimal256 {
            inner: self.inner.to_decimal256(),
        }
    }

    /// The value's bytes: the mantissa's little-endian bytes then the scale byte.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a decimal from its serialised bytes.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_core::Decimal32::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &Decimal32) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The decimal value in plain form, e.g. `"123.45"`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}

/// A fixed-width `decimal256` (256-bit integer mantissa × 10^(−scale)). Its mantissa
/// marshals as a `bigint`, assembled from / split into the value's sign and 64-bit words.
#[napi(namespace = "decimal")]
pub struct Decimal256 {
    pub(crate) inner: yggdryl_core::Decimal256,
}

#[napi(namespace = "decimal")]
impl Decimal256 {
    /// Builds a decimal from an integer `mantissa` (a `bigint`) and `scale` (default 0),
    /// throwing a guided error if the mantissa exceeds the signed 256-bit range.
    #[napi(constructor)]
    pub fn new(mantissa: BigInt, scale: Option<i8>) -> napi::Result<Self> {
        Ok(Self {
            inner: yggdryl_core::Decimal256::new(bigint_to_i256(mantissa)?, scale.unwrap_or(0)),
        })
    }

    /// Builds a decimal approximating `value` at `scale`.
    #[napi(factory)]
    pub fn from_f64(value: f64, scale: Option<i8>) -> Self {
        Self {
            inner: yggdryl_core::Decimal256::from_f64(value, scale.unwrap_or(0)),
        }
    }

    /// The unscaled integer mantissa (a `bigint`, possibly beyond 128 bits).
    #[napi(getter)]
    pub fn mantissa(&self) -> BigInt {
        i256_to_bigint(self.inner.mantissa())
    }

    /// The scale (number of fractional decimal digits).
    #[napi(getter)]
    pub fn scale(&self) -> i8 {
        self.inner.scale()
    }

    /// The mantissa width in bits.
    #[napi(getter)]
    pub fn bits(&self) -> u32 {
        yggdryl_core::Decimal256::BITS
    }

    /// The value as a float (`mantissa / 10^scale`; lossy for large mantissas).
    #[napi]
    pub fn to_f64(&self) -> f64 {
        self.inner.to_f64()
    }

    /// The integer part (a `bigint`), truncated toward zero, or `null` if it exceeds `i128`.
    #[napi]
    pub fn to_i128(&self) -> Option<BigInt> {
        self.inner.to_i128().map(BigInt::from)
    }

    /// Re-expresses the value at `newScale`, throwing a guided error on overflow.
    #[napi]
    pub fn rescale(&self, new_scale: i8) -> napi::Result<Self> {
        self.inner
            .rescale(new_scale)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Narrows to a `Decimal128` if the mantissa fits `i128`, else throws a guided error.
    #[napi]
    pub fn try_to_decimal128(&self) -> napi::Result<Decimal128> {
        self.inner
            .try_to_decimal128()
            .map(|inner| Decimal128 { inner })
            .map_err(to_error)
    }

    /// The value's bytes: the mantissa's 32 little-endian bytes then the scale byte.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a decimal from its serialised bytes.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_core::Decimal256::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &Decimal256) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The decimal value in plain form, e.g. `"123.45"`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}
