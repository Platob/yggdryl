//! Floating scalar canonicalization.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use crate::types::arithmetic::{Arithmetic, invalid_binary};
use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, ScalarFamily, ScalarValue};

/// Operations shared by every IEEE floating-point representation.
pub trait FloatingValue: ScalarValue {
    /// The physical width in bits.
    const BIT_WIDTH: u8;

    /// Return this value widened to binary64.
    fn as_f64(&self) -> f64;
}

define_scalar_type!(
    Float16Scalar,
    super::Float16Type,
    "float16",
    crate::DataType::Float16
);
define_scalar_type!(
    Float32Scalar,
    super::Float32Type,
    "float32",
    crate::DataType::Float32
);
define_scalar_type!(
    Float64Scalar,
    super::Float64Type,
    "float64",
    crate::DataType::Float64
);

pub(crate) enum FloatWidth {
    Float16,
    Float32,
    Float64,
}

pub(crate) fn canonical_float(value: &Scalar, width: FloatWidth) -> Result<(Scalar, bool)> {
    let Some(number) = value.as_f64() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated float value could not be canonicalized"),
        });
    };
    let canonical = match width {
        FloatWidth::Float16 => Scalar::from(half::f16::from_f64(number)),
        FloatWidth::Float32 => Scalar::from(number as f32),
        FloatWidth::Float64 => Scalar::from(number),
    };
    let changed = value != &canonical;
    Ok((canonical, changed))
}

/// A bit-preserving, totally ordered 64-bit floating-point value.
///
/// All NaN payloads are normalized at construction. Positive and negative
/// zero remain distinct so codecs can round-trip their exact representation.
#[derive(Clone, Copy, Default)]
pub struct Float64(u64);

impl Float64 {
    /// Construct from a native `f64`.
    pub fn from_f64(value: f64) -> Self {
        if value.is_nan() {
            Self(f64::NAN.to_bits())
        } else {
            Self(value.to_bits())
        }
    }

    /// Return the native `f64` value.
    pub const fn as_f64(self) -> f64 {
        f64::from_bits(self.0)
    }

    /// Consume and return the native `f64` value.
    pub const fn into_f64(self) -> f64 {
        self.as_f64()
    }

    /// Return the non-negative IEEE magnitude, preserving this width.
    pub fn abs(self) -> Self {
        Self::from_f64(self.as_f64().abs())
    }

    /// Return the deterministic hash of the canonical IEEE bits.
    ///
    /// The value is hashed through [`Scalar::write_bytes`], so every float
    /// width answering the same number answers the same hash.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        Scalar::Floating(Floating::F64(*self)).stable_hash()
    }
}

impl fmt::Debug for Float64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f64().fmt(formatter)
    }
}

impl fmt::Display for Float64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f64().fmt(formatter)
    }
}

impl PartialEq for Float64 {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Float64 {}

impl PartialOrd for Float64 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float64 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_f64().total_cmp(&other.as_f64())
    }
}

impl Hash for Float64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<f64> for Float64 {
    fn from(value: f64) -> Self {
        Self::from_f64(value)
    }
}

impl From<Float64> for f64 {
    fn from(value: Float64) -> Self {
        value.into_f64()
    }
}

impl Serialize for Float64 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct Bits(u64);

        impl fmt::Display for Bits {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:016x}", self.0)
            }
        }

        serializer.collect_str(&Bits(self.0))
    }
}

impl<'de> Deserialize<'de> for Float64 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FloatVisitor;

        impl<'de> serde::de::Visitor<'de> for FloatVisitor {
            type Value = Float64;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact 0x-prefixed f64 bit string or floating number")
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Ok(Float64::from_f64(value))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(Float64::from_f64(value as f64))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Float64::from_f64(value as f64))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let digits = value
                    .strip_prefix("0x")
                    .ok_or_else(|| E::custom("float bits must start with 0x"))?;
                if digits.len() != 16 {
                    return Err(E::custom("float bits must contain exactly 16 hex digits"));
                }
                let bits = u64::from_str_radix(digits, 16)
                    .map_err(|_| E::custom("float bits contain invalid hex"))?;
                Ok(Float64::from_f64(f64::from_bits(bits)))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(FloatVisitor)
    }
}

/// A bit-preserving, totally ordered IEEE 754 binary16 value.
#[derive(Clone, Copy, Default)]
pub struct Float16(u16);

impl Float16 {
    /// Construct from a native half-precision value.
    pub fn from_f16(value: half::f16) -> Self {
        if value.is_nan() {
            Self(half::f16::NAN.to_bits())
        } else {
            Self(value.to_bits())
        }
    }

    /// Return the native half-precision value.
    pub const fn as_f16(self) -> half::f16 {
        half::f16::from_bits(self.0)
    }

    /// Return the value widened exactly to `f32`.
    pub fn as_f32(self) -> f32 {
        self.as_f16().to_f32()
    }

    /// Return the value widened exactly to `f64`.
    pub fn as_f64(self) -> f64 {
        f64::from(self.as_f32())
    }

    /// Return the non-negative IEEE magnitude, preserving this width.
    pub fn abs(self) -> Self {
        Self::from_f16(half::f16::from_bits(self.0 & 0x7fff))
    }

    /// Return the deterministic hash of the canonical IEEE bits.
    ///
    /// The value is hashed through [`Scalar::write_bytes`], so every float
    /// width answering the same number answers the same hash.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        Scalar::Floating(Floating::F16(*self)).stable_hash()
    }
}

impl fmt::Debug for Float16 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f16().fmt(formatter)
    }
}

impl fmt::Display for Float16 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f16().fmt(formatter)
    }
}

impl PartialEq for Float16 {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Float16 {}

impl PartialOrd for Float16 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float16 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_f32().total_cmp(&other.as_f32())
    }
}

impl Hash for Float16 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<half::f16> for Float16 {
    fn from(value: half::f16) -> Self {
        Self::from_f16(value)
    }
}

impl From<Float16> for half::f16 {
    fn from(value: Float16) -> Self {
        value.as_f16()
    }
}

impl Serialize for Float16 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct Bits(u16);

        impl fmt::Display for Bits {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:04x}", self.0)
            }
        }

        serializer.collect_str(&Bits(self.0))
    }
}

impl<'de> Deserialize<'de> for Float16 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Float16;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact 0x-prefixed f16 bit string or floating number")
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Ok(Float16::from_f16(half::f16::from_f64(value)))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(Float16::from_f16(half::f16::from_f64(value as f64)))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Float16::from_f16(half::f16::from_f64(value as f64)))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let digits = value
                    .strip_prefix("0x")
                    .ok_or_else(|| E::custom("float bits must start with 0x"))?;
                if digits.len() != 4 {
                    return Err(E::custom("f16 bits must contain exactly 4 hex digits"));
                }
                let bits = u16::from_str_radix(digits, 16)
                    .map_err(|_| E::custom("float bits contain invalid hex"))?;
                Ok(Float16::from_f16(half::f16::from_bits(bits)))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// A bit-preserving, totally ordered 32-bit floating-point value.
///
/// The narrow sibling of [`Float64`], for a value that arrived through a
/// `Float32` column and must leave through one: widening it to 64 bits would
/// erase which width the column declared. All NaN payloads are normalized at
/// construction; the two zeros remain distinct.
#[derive(Clone, Copy, Default)]
pub struct Float32(u32);

impl Float32 {
    /// Construct from a native `f32`.
    pub fn from_f32(value: f32) -> Self {
        if value.is_nan() {
            Self(f32::NAN.to_bits())
        } else {
            Self(value.to_bits())
        }
    }

    /// Return the native `f32` value.
    pub const fn as_f32(self) -> f32 {
        f32::from_bits(self.0)
    }

    /// Return the value widened to `f64`, which is exact for every `f32`.
    pub const fn as_f64(self) -> f64 {
        self.as_f32() as f64
    }

    /// Return the non-negative IEEE magnitude, preserving this width.
    pub fn abs(self) -> Self {
        Self::from_f32(self.as_f32().abs())
    }

    /// Return the deterministic hash of the canonical IEEE bits.
    ///
    /// The value is hashed through [`Scalar::write_bytes`], so every float
    /// width answering the same number answers the same hash.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        Scalar::Floating(Floating::F32(*self)).stable_hash()
    }
}

impl fmt::Debug for Float32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f32().fmt(formatter)
    }
}

impl fmt::Display for Float32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f32().fmt(formatter)
    }
}

impl PartialEq for Float32 {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Float32 {}

impl PartialOrd for Float32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float32 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_f32().total_cmp(&other.as_f32())
    }
}

impl Hash for Float32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<f32> for Float32 {
    fn from(value: f32) -> Self {
        Self::from_f32(value)
    }
}

impl From<Float32> for f32 {
    fn from(value: Float32) -> Self {
        value.as_f32()
    }
}

impl Serialize for Float32 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct Bits(u32);

        impl fmt::Display for Bits {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:08x}", self.0)
            }
        }

        serializer.collect_str(&Bits(self.0))
    }
}

impl<'de> Deserialize<'de> for Float32 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Float32Visitor;

        impl serde::de::Visitor<'_> for Float32Visitor {
            type Value = Float32;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact 0x-prefixed f32 bit string or floating number")
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Ok(Float32::from_f32(value as f32))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(Float32::from_f32(value as f32))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Float32::from_f32(value as f32))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let digits = value
                    .strip_prefix("0x")
                    .ok_or_else(|| E::custom("float bits must start with 0x"))?;
                if digits.len() != 8 {
                    return Err(E::custom("f32 bits must contain exactly 8 hex digits"));
                }
                let bits = u32::from_str_radix(digits, 16)
                    .map_err(|_| E::custom("float bits contain invalid hex"))?;
                Ok(Float32::from_f32(f32::from_bits(bits)))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(Float32Visitor)
    }
}

macro_rules! floating_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $dtype:ident, $bits:literal) => {
        impl ScalarValue for $leaf {
            type Family = Floating;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Floating;

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$dtype)
            }

            fn into_family(self) -> Self::Family {
                Floating::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Floating::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Floating(Floating::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Floating(Floating::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl FloatingValue for $leaf {
            const BIT_WIDTH: u8 = $bits;

            fn as_f64(&self) -> f64 {
                <$leaf>::as_f64(*self)
            }
        }
    };
}

floating_value!(
    Float16,
    super::fields::Float16Type,
    F16,
    Float16,
    Float16,
    16
);
floating_value!(
    Float32,
    super::fields::Float32Type,
    F32,
    Float32,
    Float32,
    32
);
floating_value!(
    Float64,
    super::fields::Float64Type,
    F64,
    Float64,
    Float64,
    64
);

/// A copyable view over any exact floating-point width.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Floating {
    /// IEEE binary16.
    F16(Float16),
    /// IEEE binary32.
    F32(Float32),
    /// IEEE binary64.
    F64(Float64),
}

const _: () = assert!(std::mem::size_of::<Floating>() == 16);

impl Floating {
    /// Return the exact physical bit width.
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::F16(_) => 16,
            Self::F32(_) => 32,
            Self::F64(_) => 64,
        }
    }

    /// Return the value widened exactly to binary64.
    pub fn as_f64(self) -> f64 {
        match self {
            Self::F16(value) => value.as_f64(),
            Self::F32(value) => value.as_f64(),
            Self::F64(value) => value.as_f64(),
        }
    }

    /// Return the value at binary32 when no binary64 narrowing is needed.
    pub fn as_f32(self) -> Option<f32> {
        match self {
            Self::F16(value) => Some(value.as_f32()),
            Self::F32(value) => Some(value.as_f32()),
            Self::F64(_) => None,
        }
    }

    /// Return the value only when its exact width is binary16.
    pub const fn as_f16(self) -> Option<half::f16> {
        match self {
            Self::F16(value) => Some(value.as_f16()),
            Self::F32(_) | Self::F64(_) => None,
        }
    }

    /// Widen this family value to the scalar root.
    pub const fn into_scalar(self) -> Scalar {
        Scalar::Floating(self)
    }

    /// Return the deterministic hash shared by equal widths and values.
    pub fn stable_hash(&self) -> u64 {
        self.into_scalar().stable_hash()
    }

    fn common(self) -> Float64 {
        Float64::from_f64(self.as_f64())
    }
}

impl fmt::Display for Floating {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::F16(value) => value.fmt(formatter),
            Self::F32(value) => value.fmt(formatter),
            Self::F64(value) => value.fmt(formatter),
        }
    }
}

impl PartialEq for Floating {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Floating {}

impl PartialOrd for Floating {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Floating {
    fn cmp(&self, other: &Self) -> Ordering {
        self.common().cmp(&other.common())
    }
}

impl Hash for Floating {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.common().hash(state);
    }
}

impl ScalarFamily for Floating {
    const KIND: DataTypeKind = DataTypeKind::Floating;

    fn id(&self) -> DataTypeId {
        match self {
            Self::F16(_) => DataTypeId::Float16,
            Self::F32(_) => DataTypeId::Float32,
            Self::F64(_) => DataTypeId::Float64,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(match self {
            Self::F16(_) => DataType::Float16,
            Self::F32(_) => DataType::Float32,
            Self::F64(_) => DataType::Float64,
        })
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Floating(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Floating(value) => Some(value),
            _ => None,
        }
    }
}

impl From<half::f16> for Floating {
    fn from(value: half::f16) -> Self {
        Self::F16(Float16::from_f16(value))
    }
}

impl From<f32> for Floating {
    fn from(value: f32) -> Self {
        Self::F32(Float32::from_f32(value))
    }
}

impl From<f64> for Floating {
    fn from(value: f64) -> Self {
        Self::F64(Float64::from_f64(value))
    }
}

impl Scalar {
    /// Build the requested width, applying IEEE rounding when narrowing.
    pub fn from_float(value: f64, bit_width: u8) -> Result<Self> {
        match bit_width {
            16 => Ok(Self::Floating(Floating::F16(Float16::from_f16(
                half::f16::from_f64(value),
            )))),
            32 => Ok(Self::Floating(Floating::F32(Float32::from_f32(
                value as f32,
            )))),
            64 => Ok(Self::Floating(Floating::F64(Float64::from_f64(value)))),
            _ => Err(Error::InvalidRecord {
                path: "$".into(),
                reason: format!("float bit width must be 16, 32, or 64, got {bit_width}").into(),
            }),
        }
    }
}

impl Scalar {
    /// Return the exact floating-point width and value.
    pub const fn as_float(&self) -> Option<Floating> {
        match self {
            Self::Floating(value) => Some(*value),
            _ => None,
        }
    }

    /// Return a floating-point value when this is a float of either width.
    ///
    /// The 32-bit width widens exactly, so no float answers differently here
    /// than it would at its own width.
    pub fn as_f64(&self) -> Option<f64> {
        self.as_float().map(Floating::as_f64)
    }

    /// Return the 32-bit float when this is one.
    ///
    /// The wide float does not narrow here, because `as_f64` widening is
    /// exact and narrowing is not; a caller who wants the rounding asks for
    /// it with `as f32` where the loss is visible.
    pub fn as_f32(&self) -> Option<f32> {
        self.as_float().and_then(Floating::as_f32)
    }

    /// Return the 16-bit float when this is one.
    pub const fn as_f16(&self) -> Option<half::f16> {
        match self.as_float() {
            Some(value) => value.as_f16(),
            None => None,
        }
    }
}

impl From<f32> for Scalar {
    fn from(value: f32) -> Self {
        Self::Floating(Floating::F32(Float32::from_f32(value)))
    }
}

impl From<half::f16> for Scalar {
    fn from(value: half::f16) -> Self {
        Self::Floating(Floating::F16(Float16::from_f16(value)))
    }
}

impl From<f64> for Scalar {
    fn from(value: f64) -> Self {
        Self::Floating(Floating::F64(Float64::from_f64(value)))
    }
}

impl From<Floating> for Scalar {
    fn from(value: Floating) -> Self {
        value.into_scalar()
    }
}

impl From<Float16> for Scalar {
    fn from(value: Float16) -> Self {
        Self::Floating(Floating::F16(value))
    }
}

impl From<Float32> for Scalar {
    fn from(value: Float32) -> Self {
        Self::Floating(Floating::F32(value))
    }
}

impl From<Float64> for Scalar {
    fn from(value: Float64) -> Self {
        Self::Floating(Floating::F64(value))
    }
}

pub(crate) fn float_value_width(value: &Scalar) -> Option<u8> {
    value.as_float().map(|value| value.bit_width())
}

pub(crate) fn float_width(dtype: &DataType) -> Option<u8> {
    match dtype {
        DataType::Float16 => Some(16),
        DataType::Float32 => Some(32),
        DataType::Float64 => Some(64),
        _ => None,
    }
}

fn arithmetic_float(value: &Scalar) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_i128()
            .map(|value| value as f64)
            .or_else(|| value.as_u128().map(|value| value as f64))
    })
}

pub(crate) fn float_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    width: u8,
) -> Result<Scalar> {
    let left_number = arithmetic_float(left)
        .ok_or_else(|| invalid_binary(operation, left, right, "left operand is not numeric"))?;
    let right_number = arithmetic_float(right)
        .ok_or_else(|| invalid_binary(operation, left, right, "right operand is not numeric"))?;
    if right_number == 0.0 && matches!(operation, Arithmetic::Div | Arithmetic::Rem) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    Ok(match width {
        16 => {
            let left = left_number as f32;
            let right = right_number as f32;
            let held = float_operation(left, operation, right);
            Scalar::Floating(Floating::F16(Float16::from_f16(half::f16::from_f32(held))))
        }
        32 => Scalar::Floating(Floating::F32(Float32::from_f32(float_operation(
            left_number as f32,
            operation,
            right_number as f32,
        )))),
        _ => Scalar::Floating(Floating::F64(Float64::from_f64(float_operation(
            left_number,
            operation,
            right_number,
        )))),
    })
}

fn float_operation<T>(left: T, operation: Arithmetic, right: T) -> T
where
    T: Add<Output = T> + Sub<Output = T> + Mul<Output = T> + Div<Output = T> + Rem<Output = T>,
{
    match operation {
        Arithmetic::Add => left + right,
        Arithmetic::Sub => left - right,
        Arithmetic::Mul => left * right,
        Arithmetic::Div => left / right,
        Arithmetic::Rem => left % right,
    }
}

macro_rules! float_operators {
    ($value:ty, $native:ty, $get:ident, $new:ident) => {
        impl Add for $value {
            type Output = Self;
            fn add(self, other: Self) -> Self {
                Self::$new(self.$get() + other.$get())
            }
        }
        impl Sub for $value {
            type Output = Self;
            fn sub(self, other: Self) -> Self {
                Self::$new(self.$get() - other.$get())
            }
        }
        impl Mul for $value {
            type Output = Self;
            fn mul(self, other: Self) -> Self {
                Self::$new(self.$get() * other.$get())
            }
        }
        impl Div for $value {
            type Output = Self;
            fn div(self, other: Self) -> Self {
                Self::$new(self.$get() / other.$get())
            }
        }
        impl Rem for $value {
            type Output = Self;
            fn rem(self, other: Self) -> Self {
                Self::$new(self.$get() % other.$get())
            }
        }
        impl Neg for $value {
            type Output = Self;
            fn neg(self) -> Self {
                Self::$new(-self.$get())
            }
        }
        impl AddAssign for $value {
            fn add_assign(&mut self, other: Self) {
                *self = *self + other;
            }
        }
        impl SubAssign for $value {
            fn sub_assign(&mut self, other: Self) {
                *self = *self - other;
            }
        }
        impl MulAssign for $value {
            fn mul_assign(&mut self, other: Self) {
                *self = *self * other;
            }
        }
        impl DivAssign for $value {
            fn div_assign(&mut self, other: Self) {
                *self = *self / other;
            }
        }
        impl RemAssign for $value {
            fn rem_assign(&mut self, other: Self) {
                *self = *self % other;
            }
        }
    };
}

float_operators!(Float16, half::f16, as_f16, from_f16);
float_operators!(Float32, f32, as_f32, from_f32);
float_operators!(Float64, f64, as_f64, from_f64);
