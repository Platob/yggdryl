//! Dedicated, fixed-width numeric type descriptors — **one native Rust struct per
//! concrete numeric [`DataType`]**, each **generic over its native Rust storage type**
//! ([`Native`]) and defaulting to the natural one: [`Int64`] is `Int64<i64>`,
//! [`Float16`] is `Float16<f16>`, [`Decimal128`] is `Decimal128<i128>`. The native type
//! **reuses the Rust built-in** where one exists (`i8` … `u64`, `f32`, `f64`, `i128`)
//! and the two Rust has **no built-in for** are created here: the half-precision
//! [`f16`](struct@f16) and the 256-bit [`i256`].
//!
//! Each struct owns its `bits` / `kind` / `head` / `native` and an [`info`](FixedInfo)
//! snapshot — the per-type behaviour the [`DataType`] enum delegates to (so the enum
//! has no per-accessor match over widths) — and converts into the matching
//! [`DataType`] via `From`.
//!
//! ```
//! use yggdryl_schema::{DataType, Int32, Decimal128, f16, i256};
//! let int32 = Int32::<i32>::new();
//! assert_eq!(int32.head(), "int32");               // canonical type name
//! assert_eq!(int32.native(), "i32");               // native Rust storage
//! assert_eq!(DataType::from(int32), DataType::int(32, true));
//! assert_eq!(DataType::from(Decimal128::new(10, 2)), DataType::decimal(10, 2));
//! // the two types Rust lacks a built-in for, created here:
//! assert_eq!(f16::from_f32(0.5).to_f32(), 0.5);
//! assert_eq!(i256::from_i128(-5).to_str(), "-5");
//! ```

use std::fmt;

use super::{DataType, SchemaError};

// ===========================================================================
// f16 — half-precision (IEEE-754 binary16) float
// ===========================================================================

/// A half-precision (IEEE-754 binary16) floating-point number — the native storage
/// backing [`Float16`] / [`DataType::Float16`], which Rust has **no built-in** for.
///
/// Stored as the raw 16 sign/exponent/mantissa bits; convert to/from a wider float
/// with [`from_f32`](Self::from_f32) / [`to_f32`](Self::to_f32) (exact in that
/// direction). Equality and hashing are **bit-exact** (so `+0.0` and `-0.0` differ and
/// a `NaN` equals only the same bit pattern), which is what makes the type [`Hash`] +
/// [`Eq`] and usable as a map key.
///
/// ```
/// use yggdryl_schema::f16;
/// assert_eq!(f16::from_f32(1.0).to_f32(), 1.0);
/// assert_eq!(f16::from_f32(0.5).to_str(), "0.5");
/// assert_eq!(f16::from_bytes(&f16::from_f32(-2.0).to_bytes()).unwrap(), f16::from_f32(-2.0));
/// ```
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct f16 {
    /// The raw IEEE-754 binary16 bit pattern.
    bits: u16,
}

impl f16 {
    /// The positive-zero value.
    pub const ZERO: f16 = f16 { bits: 0 };
    /// The value `1.0`.
    pub const ONE: f16 = f16 { bits: 0x3c00 };

    /// Wraps a raw binary16 bit pattern.
    pub fn from_bits(bits: u16) -> f16 {
        f16 { bits }
    }

    /// The raw binary16 bit pattern.
    pub fn to_bits(self) -> u16 {
        self.bits
    }

    /// Rounds an `f32` to the nearest representable half (round-to-nearest-even),
    /// saturating to ±∞ on overflow.
    pub fn from_f32(value: f32) -> f16 {
        let x = value.to_bits();
        let sign = ((x >> 16) & 0x8000) as u16;
        let biased = (x >> 23) & 0xff;
        let mantissa = x & 0x007f_ffff;

        // Inf / NaN: an all-ones f32 exponent.
        if biased == 0xff {
            return if mantissa == 0 {
                f16 {
                    bits: sign | 0x7c00,
                } // ±∞
            } else {
                f16 {
                    bits: sign | 0x7e00,
                } // quiet NaN
            };
        }

        // Re-bias the exponent for half precision (f32 bias 127 → f16 bias 15).
        let exp = biased as i32 - 127 + 15;

        if exp >= 0x1f {
            return f16 {
                bits: sign | 0x7c00,
            }; // overflow → ±∞
        }
        if exp <= 0 {
            // Subnormal half, or underflow to zero.
            if exp < -10 {
                return f16 { bits: sign };
            }
            let mantissa = mantissa | 0x0080_0000; // restore the implicit leading 1
            let shift = (14 - exp) as u32; // 14..=24
            let mut half = mantissa >> shift;
            let rem = mantissa & ((1u32 << shift) - 1);
            let halfway = 1u32 << (shift - 1);
            if rem > halfway || (rem == halfway && (half & 1) == 1) {
                half += 1;
            }
            return f16 {
                bits: sign | half as u16,
            };
        }

        // Normal half: round the 23-bit mantissa down to 10 bits.
        let mut half_exp = (exp as u32) << 10;
        let mut half_mant = mantissa >> 13;
        let rem = mantissa & 0x1fff;
        if rem > 0x1000 || (rem == 0x1000 && (half_mant & 1) == 1) {
            half_mant += 1;
            if half_mant == 0x400 {
                // Mantissa carried into the exponent.
                half_mant = 0;
                half_exp += 0x400;
                if (half_exp >> 10) >= 0x1f {
                    return f16 {
                        bits: sign | 0x7c00,
                    };
                }
            }
        }
        f16 {
            bits: sign | half_exp as u16 | half_mant as u16,
        }
    }

    /// Widens to `f32` — exact (every half is representable as an `f32`).
    pub fn to_f32(self) -> f32 {
        let bits = self.bits;
        let exp = ((bits >> 10) & 0x1f) as i32;
        let mant = (bits & 0x03ff) as u32;
        if exp == 0x1f {
            // Inf / NaN: rebuild the f32 bits so a NaN payload survives.
            let sign = ((bits & 0x8000) as u32) << 16;
            return f32::from_bits(sign | 0x7f80_0000 | (mant << 13));
        }
        let sign = if bits & 0x8000 != 0 { -1.0f32 } else { 1.0f32 };
        let magnitude = if exp == 0 {
            // Subnormal: mantissa × 2⁻²⁴.
            (mant as f32) * (1.0 / 16_777_216.0)
        } else {
            // Normal: (1 + mantissa/1024) × 2^(exp − 15) — exact in f32.
            (1.0 + (mant as f32) / 1024.0) * 2f32.powi(exp - 15)
        };
        sign * magnitude
    }

    /// Rounds an `f64` to the nearest half (via `f32`).
    pub fn from_f64(value: f64) -> f16 {
        f16::from_f32(value as f32)
    }

    /// Widens to `f64`.
    pub fn to_f64(self) -> f64 {
        self.to_f32() as f64
    }

    /// Parses a decimal float string into the nearest half (the inverse of
    /// [`to_str`](Self::to_str)).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<f16, SchemaError> {
        value
            .trim()
            .parse::<f32>()
            .map(f16::from_f32)
            .map_err(|_| SchemaError::Invalid(value.to_string()))
    }

    /// The shortest decimal string that round-trips through `f32`.
    pub fn to_str(self) -> String {
        self.to_f32().to_string()
    }

    /// The little-endian bytes of the bit pattern.
    pub fn to_bytes(self) -> Vec<u8> {
        self.bits.to_le_bytes().to_vec()
    }

    /// Reconstructs from the little-endian bytes of the bit pattern.
    pub fn from_bytes(bytes: &[u8]) -> Result<f16, SchemaError> {
        match bytes {
            [a, b] => Ok(f16 {
                bits: u16::from_le_bytes([*a, *b]),
            }),
            _ => Err(SchemaError::Invalid("f16 expects 2 bytes".into())),
        }
    }
}

impl fmt::Display for f16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for f16 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for f16 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<f16, D::Error> {
        let value = String::deserialize(deserializer)?;
        f16::from_str(&value).map_err(serde::de::Error::custom)
    }
}

// ===========================================================================
// i256 — 256-bit signed two's-complement integer
// ===========================================================================

/// A 256-bit signed two's-complement integer — the native storage backing
/// [`Decimal256`] / [`DataType::Decimal256`], which Rust has **no built-in** for.
///
/// Stored as four little-endian 64-bit limbs. Round-trips through a decimal string
/// ([`from_str`](i256::from_str) / [`to_str`](i256::to_str)), little-endian bytes
/// ([`from_le_bytes`](i256::from_le_bytes) / [`to_le_bytes`](i256::to_le_bytes)) and an
/// `i128` ([`from_i128`](i256::from_i128) / [`to_i128`](i256::to_i128), the latter
/// `None` when out of range), and is [`Hash`] + [`Eq`].
///
/// ```
/// use yggdryl_schema::i256;
/// assert_eq!(i256::from_i128(170141183460469231731687303715884105727).to_str(),
///            "170141183460469231731687303715884105727"); // i128::MAX
/// assert_eq!(i256::from_str("-42").unwrap().to_i128(), Some(-42));
/// ```
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct i256 {
    /// Little-endian limbs (`limbs[0]` is least significant).
    limbs: [u64; 4],
}

/// `10¹⁸` — the largest power of ten that fits a `u64`, used as the decimal grouping
/// base for [`i256`] string conversion.
const DEC_GROUP: u64 = 1_000_000_000_000_000_000;

impl i256 {
    /// The zero value.
    pub const ZERO: i256 = i256 { limbs: [0; 4] };

    /// Sign-extends an `i128` into a 256-bit integer.
    pub fn from_i128(value: i128) -> i256 {
        let unsigned = value as u128;
        let extend = if value < 0 { u64::MAX } else { 0 };
        i256 {
            limbs: [unsigned as u64, (unsigned >> 64) as u64, extend, extend],
        }
    }

    /// Narrows to an `i128`, or `None` if the value does not fit.
    pub fn to_i128(self) -> Option<i128> {
        let low = (self.limbs[0] as u128) | ((self.limbs[1] as u128) << 64);
        let extend = if self.limbs[1] & 0x8000_0000_0000_0000 != 0 {
            u64::MAX
        } else {
            0
        };
        if self.limbs[2] == extend && self.limbs[3] == extend {
            Some(low as i128)
        } else {
            None
        }
    }

    /// Whether the value is negative (its top bit is set).
    pub fn is_negative(self) -> bool {
        self.limbs[3] & 0x8000_0000_0000_0000 != 0
    }

    /// The 32 little-endian bytes (least-significant first).
    pub fn to_le_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, limb) in self.limbs.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&limb.to_le_bytes());
        }
        out
    }

    /// Reconstructs from 32 little-endian bytes.
    pub fn from_le_bytes(bytes: [u8; 32]) -> i256 {
        let mut limbs = [0u64; 4];
        for (i, limb) in limbs.iter_mut().enumerate() {
            *limb = u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
        }
        i256 { limbs }
    }

    /// The little-endian bytes as a `Vec` (the byte-IO surface mirror of the other types).
    pub fn to_bytes(self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }

    /// Reconstructs from the 32 little-endian bytes of [`to_bytes`](i256::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<i256, SchemaError> {
        let array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| SchemaError::Invalid("i256 expects 32 bytes".into()))?;
        Ok(i256::from_le_bytes(array))
    }

    /// Parses a (optionally signed) decimal integer string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<i256, SchemaError> {
        let trimmed = value.trim();
        let (negative, digits) = match trimmed.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SchemaError::Invalid(value.to_string()));
        }
        let mut limbs = [0u64; 4];
        for byte in digits.bytes() {
            mul10_add(&mut limbs, (byte - b'0') as u64)
                .map_err(|_| SchemaError::Invalid(value.to_string()))?;
        }
        Ok(i256 {
            limbs: if negative { negate(limbs) } else { limbs },
        })
    }

    /// The decimal string (the inverse of [`from_str`](i256::from_str)).
    pub fn to_str(self) -> String {
        let negative = self.is_negative();
        let mut magnitude = if negative {
            negate(self.limbs)
        } else {
            self.limbs
        };
        if magnitude == [0; 4] {
            return "0".to_string();
        }
        // Extract base-10¹⁸ groups, least significant first.
        let mut groups = Vec::new();
        while magnitude != [0; 4] {
            groups.push(divmod_dec_group(&mut magnitude));
        }
        let mut out = String::new();
        if negative {
            out.push('-');
        }
        let (most, rest) = groups.split_last().expect("non-empty");
        out.push_str(&most.to_string());
        for group in rest.iter().rev() {
            out.push_str(&format!("{group:018}"));
        }
        out
    }
}

/// Two's-complement negation of a 256-bit little-endian magnitude.
fn negate(mut limbs: [u64; 4]) -> [u64; 4] {
    let mut carry = 1u128;
    for limb in limbs.iter_mut() {
        let value = (!*limb) as u128 + carry;
        *limb = value as u64;
        carry = value >> 64;
    }
    limbs
}

/// `limbs = limbs * 10 + digit`, erroring on overflow past 256 bits.
fn mul10_add(limbs: &mut [u64; 4], digit: u64) -> Result<(), ()> {
    let mut carry = digit as u128;
    for limb in limbs.iter_mut() {
        let value = (*limb as u128) * 10 + carry;
        *limb = value as u64;
        carry = value >> 64;
    }
    if carry != 0 {
        return Err(());
    }
    Ok(())
}

/// Divides a 256-bit little-endian magnitude by [`DEC_GROUP`] in place, returning the
/// remainder.
fn divmod_dec_group(limbs: &mut [u64; 4]) -> u64 {
    let mut rem = 0u128;
    for limb in limbs.iter_mut().rev() {
        let current = (rem << 64) | *limb as u128;
        *limb = (current / DEC_GROUP as u128) as u64;
        rem = current % DEC_GROUP as u128;
    }
    rem as u64
}

impl fmt::Display for i256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

impl From<i128> for i256 {
    fn from(value: i128) -> i256 {
        i256::from_i128(value)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for i256 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for i256 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<i256, D::Error> {
        let value = String::deserialize(deserializer)?;
        i256::from_str(&value).map_err(serde::de::Error::custom)
    }
}

// ===========================================================================
// Native — the native Rust storage type of a fixed-width numeric DataType
// ===========================================================================

/// The numeric family a fixed-width type belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FixedKind {
    /// A signed integer (`int8` … `int64`).
    SignedInt,
    /// An unsigned integer (`uint8` … `uint64`).
    UnsignedInt,
    /// A floating-point number (`float16` … `float64`).
    Float,
    /// A fixed-point decimal (`decimal32` … `decimal256`).
    Decimal,
}

/// The **native Rust storage type** of a fixed-width numeric [`DataType`] — the type a
/// value is actually stored in. Implemented by the Rust built-ins (`i8` … `u64`, `f32`,
/// `f64`, `i128`) and by the two types Rust has no built-in for, created in this module:
/// [`f16`](struct@f16) and [`i256`]. Each fixed type is **generic over its `Native`**,
/// defaulting to the natural one — so [`Int64`] is `Int64<i64>`, [`Float16`] is
/// `Float16<f16>`, [`Decimal256`] is `Decimal256<i256>`.
pub trait Native: Copy {
    /// The canonical short name of the type (`"i64"`, `"f16"`, `"i256"`, …).
    const NAME: &'static str;
}

macro_rules! impl_native {
    ($($ty:ty => $name:literal),+ $(,)?) => {
        $(impl Native for $ty { const NAME: &'static str = $name; })+
    };
}
impl_native!(
    i8 => "i8", i16 => "i16", i32 => "i32", i64 => "i64",
    u8 => "u8", u16 => "u16", u32 => "u32", u64 => "u64",
    f32 => "f32", f64 => "f64", i128 => "i128",
    f16 => "f16", i256 => "i256",
);

/// A read-back snapshot of a fixed-width numeric type's properties — the data the
/// [`DataType`] accessors delegate to, so each width's family / size / storage / name
/// lives on its own descriptor rather than in a per-accessor match at the enum root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedInfo {
    /// The numeric family.
    pub kind: FixedKind,
    /// The physical width in bits.
    pub bits: u16,
    /// The native Rust storage type name (`"i64"`, `"f16"`, …).
    pub native: &'static str,
    /// The canonical type head without parameters (`"int64"`, `"decimal128"`).
    pub head: &'static str,
    /// The `(precision, scale)` of a decimal, else `None`.
    pub decimal: Option<(u8, i8)>,
}

// ===========================================================================
// The fixed-width type descriptors — one per width, generic over its Native type
// ===========================================================================

/// Defines a parameter-less fixed type (integer / float) as a struct generic over its
/// native Rust storage type, defaulting to the natural one (so `Int64` is `Int64<i64>`).
/// Each carries its own `bits` / `kind` / `head` / `native` and an `info()` snapshot —
/// the per-type behaviour the [`DataType`] enum delegates to.
macro_rules! fixed_scalar {
    ($name:ident, $native:ty, $bits:literal, $kind:ident, $head:literal) => {
        #[doc = concat!("The fixed-width `", $head, "` type — by default backed by the native Rust `", stringify!($native), "` (overridable as `", stringify!($name), "<T>`).")]
        #[derive(Clone, Copy, Default, Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", serde(bound = ""))]
        pub struct $name<T = $native>(core::marker::PhantomData<T>);

        impl<T> $name<T> {
            #[doc = concat!("The `", $head, "` type descriptor.")]
            pub const fn new() -> Self {
                $name(core::marker::PhantomData)
            }
            /// The physical width in bits.
            pub const fn bits(&self) -> u16 {
                $bits
            }
            /// The numeric family.
            pub const fn kind(&self) -> FixedKind {
                FixedKind::$kind
            }
            /// The canonical type name (`"int64"`, `"float16"`, …).
            pub const fn head(&self) -> &'static str {
                $head
            }
        }

        impl<T: Native> $name<T> {
            /// The native Rust storage type name (`"i64"`, `"f16"`, …).
            pub fn native(&self) -> &'static str {
                T::NAME
            }
            /// The property snapshot the [`DataType`] accessors read back.
            pub fn info(&self) -> FixedInfo {
                FixedInfo {
                    kind: FixedKind::$kind,
                    bits: $bits,
                    native: T::NAME,
                    head: $head,
                    decimal: None,
                }
            }
        }

        // Identity / hashing ignore the phantom storage type (`f32`/`f64` are neither
        // `Eq` nor `Hash`, so these cannot be derived from `T`).
        impl<T> PartialEq for $name<T> {
            fn eq(&self, _: &Self) -> bool {
                true
            }
        }
        impl<T> Eq for $name<T> {}
        impl<T> core::hash::Hash for $name<T> {
            fn hash<H: core::hash::Hasher>(&self, _: &mut H) {}
        }

        impl From<$name> for DataType {
            fn from(value: $name) -> DataType {
                DataType::$name(value)
            }
        }
    };
}

fixed_scalar!(Int8, i8, 8, SignedInt, "int8");
fixed_scalar!(Int16, i16, 16, SignedInt, "int16");
fixed_scalar!(Int32, i32, 32, SignedInt, "int32");
fixed_scalar!(Int64, i64, 64, SignedInt, "int64");
fixed_scalar!(UInt8, u8, 8, UnsignedInt, "uint8");
fixed_scalar!(UInt16, u16, 16, UnsignedInt, "uint16");
fixed_scalar!(UInt32, u32, 32, UnsignedInt, "uint32");
fixed_scalar!(UInt64, u64, 64, UnsignedInt, "uint64");
fixed_scalar!(Float16, f16, 16, Float, "float16");
fixed_scalar!(Float32, f32, 32, Float, "float32");
fixed_scalar!(Float64, f64, 64, Float, "float64");

/// Defines a fixed-width decimal type (carrying `precision` / `scale`) generic over its
/// native Rust storage, defaulting to the natural one (so `Decimal128` is `Decimal128<i128>`).
macro_rules! fixed_decimal {
    ($name:ident, $native:ty, $bits:literal, $head:literal) => {
        #[doc = concat!("The fixed-width `", $head, "` type — by default backed by the native Rust `", stringify!($native), "` (overridable as `", stringify!($name), "<T>`).")]
        #[derive(Clone, Copy, Default, Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", serde(bound = ""))]
        pub struct $name<T = $native> {
            /// Total number of significant digits.
            pub precision: u8,
            /// Digits after the decimal point (may be negative).
            pub scale: i8,
            #[cfg_attr(feature = "serde", serde(skip))]
            _native: core::marker::PhantomData<T>,
        }

        impl<T> $name<T> {
            #[doc = concat!("A `", $head, "` with the given `precision` and `scale`.")]
            pub const fn new(precision: u8, scale: i8) -> Self {
                $name {
                    precision,
                    scale,
                    _native: core::marker::PhantomData,
                }
            }
            /// The physical width in bits.
            pub const fn bits(&self) -> u16 {
                $bits
            }
            /// The numeric family ([`FixedKind::Decimal`]).
            pub const fn kind(&self) -> FixedKind {
                FixedKind::Decimal
            }
            /// The canonical type name (`"decimal128"`, …).
            pub const fn head(&self) -> &'static str {
                $head
            }
        }

        impl<T: Native> $name<T> {
            /// The native Rust storage type name (`"i128"`, `"i256"`, …).
            pub fn native(&self) -> &'static str {
                T::NAME
            }
            /// The property snapshot the [`DataType`] accessors read back.
            pub fn info(&self) -> FixedInfo {
                FixedInfo {
                    kind: FixedKind::Decimal,
                    bits: $bits,
                    native: T::NAME,
                    head: $head,
                    decimal: Some((self.precision, self.scale)),
                }
            }
        }

        // Identity / hashing ignore the phantom storage type.
        impl<T> PartialEq for $name<T> {
            fn eq(&self, other: &Self) -> bool {
                self.precision == other.precision && self.scale == other.scale
            }
        }
        impl<T> Eq for $name<T> {}
        impl<T> core::hash::Hash for $name<T> {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                self.precision.hash(state);
                self.scale.hash(state);
            }
        }

        impl From<$name> for DataType {
            fn from(value: $name) -> DataType {
                DataType::$name(value)
            }
        }
    };
}

fixed_decimal!(Decimal32, i32, 32, "decimal32");
fixed_decimal!(Decimal64, i64, 64, "decimal64");
fixed_decimal!(Decimal128, i128, 128, "decimal128");
fixed_decimal!(Decimal256, i256, 256, "decimal256");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_round_trips_representative_values() {
        for value in [0.0f32, 1.0, -1.0, 0.5, -2.0, 2.0, 65504.0, 0.25, 100.0] {
            let half = f16::from_f32(value);
            assert_eq!(half.to_f32(), value, "f16 round-trip {value}");
            // string + byte round-trips.
            assert_eq!(f16::from_str(&half.to_str()).unwrap(), half);
            assert_eq!(f16::from_bytes(&half.to_bytes()).unwrap(), half);
        }
        // Signed zero is bit-distinct.
        assert_ne!(f16::from_f32(0.0), f16::from_f32(-0.0));
        // Overflow saturates to infinity.
        assert!(f16::from_f32(1e30).to_f32().is_infinite());
        // NaN stays NaN.
        assert!(f16::from_f32(f32::NAN).to_f32().is_nan());
    }

    #[test]
    fn i256_round_trips_and_narrows() {
        for value in [
            0i128,
            1,
            -1,
            42,
            -42,
            i128::MAX,
            i128::MIN,
            1_000_000_000_000,
        ] {
            let wide = i256::from_i128(value);
            assert_eq!(wide.to_i128(), Some(value), "i256 narrow {value}");
            assert_eq!(wide.to_str(), value.to_string(), "i256 to_str {value}");
            assert_eq!(i256::from_str(&wide.to_str()).unwrap(), wide);
            assert_eq!(i256::from_bytes(&wide.to_bytes()).unwrap(), wide);
        }
        // A value beyond i128 round-trips through the string and does not narrow.
        let big = "57896044618658097711785492504343953926634992332820282019728792003956564819967";
        let parsed = i256::from_str(big).unwrap();
        assert_eq!(parsed.to_str(), big);
        assert_eq!(parsed.to_i128(), None);
        assert!(!parsed.is_negative());
        assert!(i256::from_str(&format!("-{big}")).unwrap().is_negative());
    }

    #[test]
    fn descriptors_carry_their_own_behaviour() {
        // Each concrete type owns its width / family / native storage / name. (The
        // descriptors default to their native type; standalone uses pin it explicitly.)
        assert_eq!(Int32::<i32>::new().bits(), 32);
        assert_eq!(Int32::<i32>::new().head(), "int32");
        assert_eq!(Int32::<i32>::new().native(), "i32");
        assert_eq!(Int32::<i32>::new().kind(), FixedKind::SignedInt);
        assert_eq!(UInt8::<u8>::new().native(), "u8");
        assert_eq!(UInt8::<u8>::new().kind(), FixedKind::UnsignedInt);
        assert_eq!(Float16::<f16>::new().native(), "f16");
        // The decimal carries its parameters.
        let dec = Decimal128::<i128>::new(10, 2);
        assert_eq!(dec.info().decimal, Some((10, 2)));
        assert_eq!(dec.native(), "i128");
        assert_eq!(dec.head(), "decimal128");
        // Into the enum (default native instantiation).
        assert_eq!(DataType::from(Int64::new()), DataType::int(64, true));
        assert_eq!(
            DataType::from(Decimal256::new(76, 0)),
            DataType::decimal_with(76, 0, 256)
        );
        // Generic over the native storage type — the default is the natural one.
        assert_eq!(Int64::<i64>::new().native(), "i64");
        assert_eq!(Decimal256::<i256>::new(1, 0).native(), "i256");
        // The created native types Rust lacks a built-in for.
        assert_eq!(f16::ONE.to_f32(), 1.0);
        assert_eq!(i256::ZERO.to_str(), "0");
    }
}
