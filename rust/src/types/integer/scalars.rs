//! Integer scalar canonicalization and validation.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::types::arithmetic::{Arithmetic, ArithmeticTarget, invalid_binary};
use crate::types::typed::define_scalar_type;
use crate::types::value::{PathSegment, ValidationFailure, canonical_error, expected};
use crate::{DataType, Error, Result, Scalar, TimeUnit};

/// Operations shared by every signed and unsigned integer representation.
pub trait IntegerValue: crate::ScalarValue {
    /// Whether this representation is signed.
    const SIGNED: bool;
    /// The physical width in bits.
    const BIT_WIDTH: u8;

    /// Return this integer as a signed 128-bit value.
    fn as_i128(&self) -> i128;
    /// Build this width from a signed 128-bit value.
    fn from_i128(value: i128) -> Result<Self>;
}

macro_rules! integer_leaf {
    ($name:ident, $native:ty) => {
        #[doc = concat!("One exact `", stringify!($native), "` value.")]
        #[repr(transparent)]
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name($native);

        impl $name {
            /// Construct this exact integer width.
            pub const fn new(value: $native) -> Self {
                Self(value)
            }

            /// Return the native integer.
            pub const fn get(self) -> $native {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl From<$native> for $name {
            fn from(value: $native) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for $native {
            fn from(value: $name) -> Self {
                value.get()
            }
        }
    };
}

integer_leaf!(Int8, i8);
integer_leaf!(Int16, i16);
integer_leaf!(Int32, i32);
integer_leaf!(Int64, i64);
integer_leaf!(UInt8, u8);
integer_leaf!(UInt16, u16);
integer_leaf!(UInt32, u32);
integer_leaf!(UInt64, u64);
integer_leaf!(Int128, i128);
integer_leaf!(UInt128, u128);

const _: () = assert!(std::mem::size_of::<Int32>() == 4);

define_scalar_type!(Int8Scalar, super::Int8Type, "int8", crate::DataType::Int8);
define_scalar_type!(
    Int16Scalar,
    super::Int16Type,
    "int16",
    crate::DataType::Int16
);
define_scalar_type!(
    Int32Scalar,
    super::Int32Type,
    "int32",
    crate::DataType::Int32
);
define_scalar_type!(
    Int64Scalar,
    super::Int64Type,
    "int64",
    crate::DataType::Int64
);
define_scalar_type!(
    UInt8Scalar,
    super::UInt8Type,
    "uint8",
    crate::DataType::UInt8
);
define_scalar_type!(
    UInt16Scalar,
    super::UInt16Type,
    "uint16",
    crate::DataType::UInt16
);
define_scalar_type!(
    UInt32Scalar,
    super::UInt32Type,
    "uint32",
    crate::DataType::UInt32
);
define_scalar_type!(
    UInt64Scalar,
    super::UInt64Type,
    "uint64",
    crate::DataType::UInt64
);

pub(crate) fn canonical_signed(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(integer) = value.as_i128() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "validated signed value could not be canonicalized from {}",
                value.kind()
            ),
        });
    };
    let canonical = match dtype {
        DataType::Int8 => Scalar::I8(i8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int16 => Scalar::I16(i16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int32 => Scalar::I32(i32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int64 | DataType::Interval(TimeUnit::YearMonth) => {
            Scalar::I64(i64::try_from(integer).map_err(canonical_integer_error)?)
        }
        _ => unreachable!("signed canonicalization requires a signed datatype"),
    };
    let changed = match &canonical {
        Scalar::I8(expected) => !matches!(value, Scalar::I8(current) if current == expected),
        Scalar::I16(expected) => !matches!(value, Scalar::I16(current) if current == expected),
        Scalar::I32(expected) => !matches!(value, Scalar::I32(current) if current == expected),
        Scalar::I64(expected) => !matches!(value, Scalar::I64(current) if current == expected),
        _ => unreachable!("signed canonical value has a signed kind"),
    };
    Ok((canonical, changed))
}

pub(crate) fn canonical_unsigned(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(integer) = value.as_u128() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated unsigned value could not be canonicalized"),
        });
    };
    let canonical = match dtype {
        DataType::UInt8 => Scalar::U8(u8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt16 => Scalar::U16(u16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt32 => Scalar::U32(u32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt64 => Scalar::U64(u64::try_from(integer).map_err(canonical_integer_error)?),
        _ => unreachable!("unsigned canonicalization requires an unsigned datatype"),
    };
    let changed = match &canonical {
        Scalar::U8(expected) => !matches!(value, Scalar::U8(current) if current == expected),
        Scalar::U16(expected) => !matches!(value, Scalar::U16(current) if current == expected),
        Scalar::U32(expected) => !matches!(value, Scalar::U32(current) if current == expected),
        Scalar::U64(expected) => !matches!(value, Scalar::U64(current) if current == expected),
        _ => unreachable!("unsigned canonical value has an unsigned kind"),
    };
    Ok((canonical, changed))
}

fn canonical_integer_error(_error: impl std::fmt::Display) -> Error {
    canonical_error("integer does not fit declared width")
}

pub(crate) fn validate_signed(
    value: &Scalar,
    minimum: i128,
    maximum: i128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_i128() {
        Some(value) if (minimum..=maximum).contains(&value) => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

pub(crate) fn validate_unsigned(
    value: &Scalar,
    maximum: u128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_u128() {
        Some(value) if value <= maximum => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

pub(crate) fn validate_integer_tuple(
    value: &Scalar,
    widths: &[u8],
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    let values = value
        .as_sequence()
        .ok_or_else(|| expected(expected_name, value))?;
    if values.len() != widths.len() {
        return Err(ValidationFailure::new(format_smolstr!(
            "{expected_name} requires {} integer components, got {}",
            widths.len(),
            values.len()
        )));
    }
    for (index, (value, width)) in values.iter().zip(widths).enumerate() {
        let (minimum, maximum) = if *width == 32 {
            (i128::from(i32::MIN), i128::from(i32::MAX))
        } else {
            (i128::from(i64::MIN), i128::from(i64::MAX))
        };
        validate_signed(value, minimum, maximum, expected_name)
            .map_err(|failure| failure.prepend(PathSegment::Index(index)))?;
    }
    Ok(())
}

/// One exact signed or unsigned integer representation.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Integer {
    /// Signed 8-bit integer.
    I8(Int8),
    /// Signed 16-bit integer.
    I16(Int16),
    /// Signed 32-bit integer.
    I32(Int32),
    /// Signed 64-bit integer.
    I64(Int64),
    /// Unsigned 8-bit integer.
    U8(UInt8),
    /// Unsigned 16-bit integer.
    U16(UInt16),
    /// Unsigned 32-bit integer.
    U32(UInt32),
    /// Unsigned 64-bit integer.
    U64(UInt64),
    /// Signed 128-bit integer.
    I128(Int128),
    /// Unsigned 128-bit integer.
    U128(UInt128),
}

const _: () = assert!(std::mem::size_of::<Integer>() == 32);

impl Integer {
    const fn normalized(self) -> (bool, u128) {
        match self {
            Self::I8(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
            Self::I16(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
            Self::I32(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
            Self::I64(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
            Self::I128(value) => (value.get() < 0, value.get().unsigned_abs()),
            Self::U8(value) => (false, value.get() as u128),
            Self::U16(value) => (false, value.get() as u128),
            Self::U32(value) => (false, value.get() as u128),
            Self::U64(value) => (false, value.get() as u128),
            Self::U128(value) => (false, value.get()),
        }
    }

    /// Return whether this value is negative.
    pub const fn is_negative(self) -> bool {
        self.normalized().0
    }

    /// Return the unsigned magnitude.
    pub const fn magnitude(self) -> u128 {
        self.normalized().1
    }

    /// Return the signed value when it fits `i128`.
    pub const fn as_i128(self) -> Option<i128> {
        let (negative, magnitude) = self.normalized();
        if negative {
            if magnitude == (i128::MAX as u128) + 1 {
                Some(i128::MIN)
            } else {
                Some(-(magnitude as i128))
            }
        } else if magnitude <= i128::MAX as u128 {
            Some(magnitude as i128)
        } else {
            None
        }
    }

    /// Return the unsigned value when it is non-negative.
    pub const fn as_u128(self) -> Option<u128> {
        if self.is_negative() {
            None
        } else {
            Some(self.magnitude())
        }
    }

    /// Rebuild the exact flat scalar variant this family member represents.
    pub const fn into_scalar(self) -> Scalar {
        match self {
            Self::I8(value) => Scalar::I8(value.get()),
            Self::I16(value) => Scalar::I16(value.get()),
            Self::I32(value) => Scalar::I32(value.get()),
            Self::I64(value) => Scalar::I64(value.get()),
            Self::U8(value) => Scalar::U8(value.get()),
            Self::U16(value) => Scalar::U16(value.get()),
            Self::U32(value) => Scalar::U32(value.get()),
            Self::U64(value) => Scalar::U64(value.get()),
            Self::I128(value) => Scalar::I128(value.get()),
            Self::U128(value) => Scalar::U128(value.get()),
        }
    }

    /// Return the deterministic logical integer hash.
    pub fn stable_hash(&self) -> u64 {
        self.into_scalar().stable_hash()
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I8(value) => value.fmt(formatter),
            Self::I16(value) => value.fmt(formatter),
            Self::I32(value) => value.fmt(formatter),
            Self::I64(value) => value.fmt(formatter),
            Self::U8(value) => value.fmt(formatter),
            Self::U16(value) => value.fmt(formatter),
            Self::U32(value) => value.fmt(formatter),
            Self::U64(value) => value.fmt(formatter),
            Self::I128(value) => value.fmt(formatter),
            Self::U128(value) => value.fmt(formatter),
        }
    }
}

impl PartialEq for Integer {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Integer {}

impl Ord for Integer {
    fn cmp(&self, other: &Self) -> Ordering {
        let (negative, magnitude) = self.normalized();
        let (other_negative, other_magnitude) = other.normalized();
        match (negative, other_negative) {
            (true, true) => other_magnitude.cmp(&magnitude),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => magnitude.cmp(&other_magnitude),
        }
    }
}

impl PartialOrd for Integer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Integer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.normalized().hash(state);
    }
}

impl Scalar {
    /// Return the logical sign and magnitude of any exact integer width.
    pub const fn as_integer(&self) -> Option<Integer> {
        match self {
            Self::I8(value) => Some(Integer::I8(Int8::new(*value))),
            Self::I16(value) => Some(Integer::I16(Int16::new(*value))),
            Self::I32(value) => Some(Integer::I32(Int32::new(*value))),
            Self::I64(value) => Some(Integer::I64(Int64::new(*value))),
            Self::I128(value) => Some(Integer::I128(Int128::new(*value))),
            Self::U8(value) => Some(Integer::U8(UInt8::new(*value))),
            Self::U16(value) => Some(Integer::U16(UInt16::new(*value))),
            Self::U32(value) => Some(Integer::U32(UInt32::new(*value))),
            Self::U64(value) => Some(Integer::U64(UInt64::new(*value))),
            Self::U128(value) => Some(Integer::U128(UInt128::new(*value))),
            _ => None,
        }
    }

    /// Return a signed integer when it fits `i128`.
    pub const fn as_i128(&self) -> Option<i128> {
        match self.as_integer() {
            Some(value) => value.as_i128(),
            None => None,
        }
    }

    /// Return an unsigned integer when it fits `u128`.
    pub const fn as_u128(&self) -> Option<u128> {
        match self.as_integer() {
            Some(value) => value.as_u128(),
            None => None,
        }
    }
}

impl Scalar {
    /// Return whether this is any integer, signed or unsigned, at any width.
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::I8(_)
                | Self::I16(_)
                | Self::I32(_)
                | Self::I64(_)
                | Self::U8(_)
                | Self::U16(_)
                | Self::U32(_)
                | Self::U64(_)
                | Self::I128(_)
                | Self::U128(_)
        )
    }
}

impl Scalar {
    /// Read this value as an `i64`, when it fits.
    ///
    /// A wider integer that does not fit returns `None` rather than wrapping,
    /// so a caller never silently loses magnitude.
    pub const fn as_i64(&self) -> Option<i64> {
        match self.as_i128() {
            Some(value) if value >= i64::MIN as i128 && value <= i64::MAX as i128 => {
                Some(value as i64)
            }
            _ => None,
        }
    }

    /// Read this value as a `u64`, when it fits.
    pub const fn as_u64(&self) -> Option<u64> {
        match self.as_u128() {
            Some(value) if value <= u64::MAX as u128 => Some(value as u64),
            _ => None,
        }
    }
}

// A native integer keeps its width: an `i32` is an `I32`, not an `I64` that
// happens to fit, because the width is what a column declaration reads back.
macro_rules! width_value_from {
    ($($type:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$type> for Scalar {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }
    )+};
}

width_value_from!(
    i8 => I8, i16 => I16, i32 => I32, i64 => I64,
    u8 => U8, u16 => U16, u32 => U32, u64 => U64,
);

impl From<i128> for Scalar {
    fn from(value: i128) -> Self {
        i64::try_from(value).map_or(Self::I128(value), Self::I64)
    }
}

impl From<u128> for Scalar {
    fn from(value: u128) -> Self {
        u64::try_from(value).map_or(Self::U128(value), Self::U64)
    }
}

impl From<Integer> for Scalar {
    fn from(value: Integer) -> Self {
        value.into_scalar()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct IntegerValueKind<'a> {
    pub(crate) value: &'a Scalar,
    pub(crate) signed: bool,
    pub(crate) bits: u16,
}

pub(crate) fn integer_value_kind(value: &Scalar) -> Option<IntegerValueKind<'_>> {
    let (signed, bits) = match value {
        Scalar::I8(_) => (true, 8),
        Scalar::I16(_) => (true, 16),
        Scalar::I32(_) => (true, 32),
        Scalar::I64(_) => (true, 64),
        Scalar::I128(_) => (true, 128),
        Scalar::U8(_) => (false, 8),
        Scalar::U16(_) => (false, 16),
        Scalar::U32(_) => (false, 32),
        Scalar::U64(_) => (false, 64),
        Scalar::U128(_) => (false, 128),
        _ => return None,
    };
    Some(IntegerValueKind {
        value,
        signed,
        bits,
    })
}

pub(crate) fn common_integer(
    left: IntegerValueKind<'_>,
    right: IntegerValueKind<'_>,
) -> Option<ArithmeticTarget> {
    if left.signed == right.signed {
        return Some(ArithmeticTarget::Integer {
            signed: left.signed,
            bits: left.bits.max(right.bits),
        });
    }
    let signed = if left.signed { left } else { right };
    let unsigned = if left.signed { right } else { left };
    let bits = [8, 16, 32, 64, 128]
        .into_iter()
        .find(|bits| *bits >= signed.bits && *bits > unsigned.bits)?;
    Some(ArithmeticTarget::Integer { signed: true, bits })
}

pub(crate) fn integer_kind(dtype: &DataType) -> Option<(bool, u16)> {
    Some(match dtype {
        DataType::Int8 => (true, 8),
        DataType::Int16 => (true, 16),
        DataType::Int32 => (true, 32),
        DataType::Int64 => (true, 64),
        DataType::UInt8 => (false, 8),
        DataType::UInt16 => (false, 16),
        DataType::UInt32 => (false, 32),
        DataType::UInt64 => (false, 64),
        _ => return None,
    })
}

pub(crate) fn integer_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    signed: bool,
    bits: u16,
) -> Result<Scalar> {
    let zero = if signed {
        right.as_i128() == Some(0)
    } else {
        right.as_u128() == Some(0)
    };
    if zero && matches!(operation, Arithmetic::Div | Arithmetic::Rem) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    let output = if signed {
        let left_number = left.as_i128().ok_or_else(|| {
            invalid_binary(operation, left, right, "left integer is out of range")
        })?;
        let right_number = right.as_i128().ok_or_else(|| {
            invalid_binary(operation, left, right, "right integer is out of range")
        })?;
        let held = match operation {
            Arithmetic::Add => left_number.checked_add(right_number),
            Arithmetic::Sub => left_number.checked_sub(right_number),
            Arithmetic::Mul => left_number.checked_mul(right_number),
            Arithmetic::Div => left_number.checked_div(right_number),
            Arithmetic::Rem => left_number.checked_rem(right_number),
        };
        held.and_then(|held| signed_value(bits, held))
    } else {
        let left_number = left.as_u128().ok_or_else(|| {
            invalid_binary(
                operation,
                left,
                right,
                "left integer is negative or out of range",
            )
        })?;
        let right_number = right.as_u128().ok_or_else(|| {
            invalid_binary(
                operation,
                left,
                right,
                "right integer is negative or out of range",
            )
        })?;
        let held = match operation {
            Arithmetic::Add => left_number.checked_add(right_number),
            Arithmetic::Sub => left_number.checked_sub(right_number),
            Arithmetic::Mul => left_number.checked_mul(right_number),
            Arithmetic::Div => left_number.checked_div(right_number),
            Arithmetic::Rem => left_number.checked_rem(right_number),
        };
        held.and_then(|held| unsigned_value_at(bits, held))
    };
    output.ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: integer_kind_name(signed, bits),
    })
}

fn signed_value(bits: u16, value: i128) -> Option<Scalar> {
    match bits {
        8 => i8::try_from(value).ok().map(Scalar::I8),
        16 => i16::try_from(value).ok().map(Scalar::I16),
        32 => i32::try_from(value).ok().map(Scalar::I32),
        64 => i64::try_from(value).ok().map(Scalar::I64),
        128 => Some(Scalar::I128(value)),
        _ => None,
    }
}

fn unsigned_value_at(bits: u16, value: u128) -> Option<Scalar> {
    match bits {
        8 => u8::try_from(value).ok().map(Scalar::U8),
        16 => u16::try_from(value).ok().map(Scalar::U16),
        32 => u32::try_from(value).ok().map(Scalar::U32),
        64 => u64::try_from(value).ok().map(Scalar::U64),
        128 => Some(Scalar::U128(value)),
        _ => None,
    }
}

const fn integer_kind_name(signed: bool, bits: u16) -> &'static str {
    match (signed, bits) {
        (true, 8) => "i8",
        (true, 16) => "i16",
        (true, 32) => "i32",
        (true, 64) => "i64",
        (true, 128) => "i128",
        (false, 8) => "u8",
        (false, 16) => "u16",
        (false, 32) => "u32",
        (false, 64) => "u64",
        (false, 128) => "u128",
        _ => "integer",
    }
}
