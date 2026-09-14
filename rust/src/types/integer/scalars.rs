//! Integer scalar canonicalization and validation.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::types::arithmetic::{Arithmetic, ArithmeticTarget, invalid_binary};
use crate::types::value::{PathSegment, ValidationFailure, canonical_error, expected};
use crate::{
    DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, ScalarFamily, ScalarValue, TimeUnit,
};

/// Operations shared by every signed and unsigned integer representation.
pub trait IntegerValue: crate::ScalarValue {
    /// Whether this representation is signed.
    const SIGNED: bool;
    /// The physical width in bits.
    const BIT_WIDTH: u8;

    /// Return this integer as a signed 128-bit value when it fits.
    fn as_i128(&self) -> Option<i128>;
    /// Return this integer as an unsigned 128-bit value when it is non-negative.
    fn as_u128(&self) -> Option<u128>;
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
        DataType::Int8 => Scalar::from(i8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int16 => Scalar::from(i16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int32 => Scalar::from(i32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int64 | DataType::Interval(TimeUnit::YearMonth) => {
            Scalar::from(i64::try_from(integer).map_err(canonical_integer_error)?)
        }
        _ => unreachable!("signed canonicalization requires a signed datatype"),
    };
    let changed = !same_integer_representation(value, &canonical);
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
        DataType::UInt8 => Scalar::from(u8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt16 => Scalar::from(u16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt32 => Scalar::from(u32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt64 => Scalar::from(u64::try_from(integer).map_err(canonical_integer_error)?),
        _ => unreachable!("unsigned canonicalization requires an unsigned datatype"),
    };
    let changed = !same_integer_representation(value, &canonical);
    Ok((canonical, changed))
}

fn same_integer_representation(left: &Scalar, right: &Scalar) -> bool {
    match (left, right) {
        (Scalar::Int8(left), Scalar::Int8(right)) => left == right,
        (Scalar::Int16(left), Scalar::Int16(right)) => left == right,
        (Scalar::Int32(left), Scalar::Int32(right)) => left == right,
        (Scalar::Int64(left), Scalar::Int64(right)) => left == right,
        (Scalar::Int128(left), Scalar::Int128(right)) => left == right,
        (Scalar::UInt8(left), Scalar::UInt8(right)) => left == right,
        (Scalar::UInt16(left), Scalar::UInt16(right)) => left == right,
        (Scalar::UInt32(left), Scalar::UInt32(right)) => left == right,
        (Scalar::UInt64(left), Scalar::UInt64(right)) => left == right,
        (Scalar::UInt128(left), Scalar::UInt128(right)) => left == right,
        _ => false,
    }
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

// Each width is its own family, as `Boolean` is: the `Scalar` variant holds
// the leaf directly, so there is no grouping enum to widen into.
macro_rules! integer_scalar_value {
    ($leaf:ident, $dtype:expr) => {
        impl ScalarValue for $leaf {
            type Family = Self;

            const ID: DataTypeId = DataTypeId::$leaf;
            const KIND: DataTypeKind = DataTypeKind::Integer;

            fn dtype(&self) -> Result<DataType> {
                Ok(($dtype)(self))
            }

            fn into_family(self) -> Self::Family {
                self
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                Some(family)
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$leaf(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                <Self as ScalarFamily>::from_scalar(value)
            }
        }

        impl ScalarFamily for $leaf {
            const KIND: DataTypeKind = DataTypeKind::Integer;

            fn id(&self) -> DataTypeId {
                DataTypeId::$leaf
            }

            fn dtype(&self) -> Result<DataType> {
                <Self as ScalarValue>::dtype(self)
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$leaf(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$leaf(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

integer_scalar_value!(Int8, |_: &Int8| DataType::Int8);
integer_scalar_value!(Int16, |_: &Int16| { DataType::Int16 });
integer_scalar_value!(Int32, |_: &Int32| { DataType::Int32 });
integer_scalar_value!(Int64, |_: &Int64| { DataType::Int64 });
integer_scalar_value!(UInt8, |_: &UInt8| { DataType::UInt8 });
integer_scalar_value!(UInt16, |_: &UInt16| { DataType::UInt16 });
integer_scalar_value!(UInt32, |_: &UInt32| { DataType::UInt32 });
integer_scalar_value!(UInt64, |_: &UInt64| { DataType::UInt64 });
integer_scalar_value!(Int128, |value: &Int128| {
    wide_integer_dtype(value.get().unsigned_abs())
});
integer_scalar_value!(UInt128, |value: &UInt128| {
    wide_integer_dtype(value.get())
});

macro_rules! signed_integer_value {
    ($leaf:ident, $native:ty, $bits:literal) => {
        impl IntegerValue for $leaf {
            const SIGNED: bool = true;
            const BIT_WIDTH: u8 = $bits;

            fn as_i128(&self) -> Option<i128> {
                Some(i128::from(self.get()))
            }

            fn as_u128(&self) -> Option<u128> {
                u128::try_from(self.get()).ok()
            }

            fn from_i128(value: i128) -> Result<Self> {
                <$native>::try_from(value)
                    .map(Self::new)
                    .map_err(|_| integer_leaf_range(stringify!($native)))
            }
        }
    };
}

signed_integer_value!(Int8, i8, 8);
signed_integer_value!(Int16, i16, 16);
signed_integer_value!(Int32, i32, 32);
signed_integer_value!(Int64, i64, 64);

impl IntegerValue for Int128 {
    const SIGNED: bool = true;
    const BIT_WIDTH: u8 = 128;

    fn as_i128(&self) -> Option<i128> {
        Some(self.get())
    }

    fn as_u128(&self) -> Option<u128> {
        u128::try_from(self.get()).ok()
    }

    fn from_i128(value: i128) -> Result<Self> {
        Ok(Self::new(value))
    }
}

macro_rules! unsigned_integer_value {
    ($leaf:ident, $native:ty, $bits:literal) => {
        impl IntegerValue for $leaf {
            const SIGNED: bool = false;
            const BIT_WIDTH: u8 = $bits;

            fn as_i128(&self) -> Option<i128> {
                i128::try_from(self.get()).ok()
            }

            fn as_u128(&self) -> Option<u128> {
                Some(u128::from(self.get()))
            }

            fn from_i128(value: i128) -> Result<Self> {
                <$native>::try_from(value)
                    .map(Self::new)
                    .map_err(|_| integer_leaf_range(stringify!($native)))
            }
        }
    };
}

unsigned_integer_value!(UInt8, u8, 8);
unsigned_integer_value!(UInt16, u16, 16);
unsigned_integer_value!(UInt32, u32, 32);
unsigned_integer_value!(UInt64, u64, 64);

impl IntegerValue for UInt128 {
    const SIGNED: bool = false;
    const BIT_WIDTH: u8 = 128;

    fn as_i128(&self) -> Option<i128> {
        i128::try_from(self.get()).ok()
    }

    fn as_u128(&self) -> Option<u128> {
        Some(self.get())
    }

    fn from_i128(value: i128) -> Result<Self> {
        u128::try_from(value)
            .map(Self::new)
            .map_err(|_| integer_leaf_range("u128"))
    }
}

fn integer_leaf_range(kind: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("integer does not fit {kind}"),
    }
}

fn wide_integer_dtype(magnitude: u128) -> DataType {
    let mut precision = 1_u8;
    let mut remaining = magnitude / 10;
    while remaining != 0 {
        precision += 1;
        remaining /= 10;
    }
    DataType::decimal(precision, 0).expect("a u128 always fits Arrow decimal256")
}

/// Read the sign and magnitude of any exact integer width.
///
/// The pair is `(is_negative, magnitude)`, so every width holding the same
/// number answers the same pair: it is what cross-width integer equality,
/// order and hashing read. A value that is not an integer answers `None`.
pub(crate) const fn integer_parts(value: &Scalar) -> Option<(bool, u128)> {
    Some(match value {
        Scalar::Int8(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
        Scalar::Int16(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
        Scalar::Int32(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
        Scalar::Int64(value) => (value.get() < 0, (value.get() as i128).unsigned_abs()),
        Scalar::Int128(value) => (value.get() < 0, value.get().unsigned_abs()),
        Scalar::UInt8(value) => (false, value.get() as u128),
        Scalar::UInt16(value) => (false, value.get() as u128),
        Scalar::UInt32(value) => (false, value.get() as u128),
        Scalar::UInt64(value) => (false, value.get() as u128),
        Scalar::UInt128(value) => (false, value.get()),
        _ => return None,
    })
}

/// Order two [`integer_parts`] readings as the numbers they spell.
///
/// A negative number sorts before a non-negative one; two negatives order by
/// reversed magnitude, two non-negatives by magnitude.
pub(crate) const fn compare_integer_parts(left: (bool, u128), right: (bool, u128)) -> Ordering {
    const fn magnitudes(left: u128, right: u128) -> Ordering {
        if left < right {
            Ordering::Less
        } else if left > right {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }

    match (left.0, right.0) {
        (true, true) => magnitudes(right.1, left.1),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => magnitudes(left.1, right.1),
    }
}

impl Scalar {
    /// Return a signed integer when it fits `i128`.
    pub const fn as_i128(&self) -> Option<i128> {
        match integer_parts(self) {
            Some((true, magnitude)) => {
                if magnitude == (i128::MAX as u128) + 1 {
                    Some(i128::MIN)
                } else {
                    Some(-(magnitude as i128))
                }
            }
            Some((false, magnitude)) => {
                if magnitude <= i128::MAX as u128 {
                    Some(magnitude as i128)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    /// Return an unsigned integer when it fits `u128`.
    pub const fn as_u128(&self) -> Option<u128> {
        match integer_parts(self) {
            Some((false, magnitude)) => Some(magnitude),
            _ => None,
        }
    }
}

impl Scalar {
    /// Return whether this is any integer, signed or unsigned, at any width.
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::Int8(_)
                | Self::Int16(_)
                | Self::Int32(_)
                | Self::Int64(_)
                | Self::UInt8(_)
                | Self::UInt16(_)
                | Self::UInt32(_)
                | Self::UInt64(_)
                | Self::Int128(_)
                | Self::UInt128(_)
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
    ($($type:ty => $leaf:ident),+ $(,)?) => {$(
        impl From<$type> for Scalar {
            fn from(value: $type) -> Self {
                Self::$leaf($leaf::new(value))
            }
        }
    )+};
}

width_value_from!(
    i8 => Int8, i16 => Int16, i32 => Int32, i64 => Int64,
    u8 => UInt8, u16 => UInt16, u32 => UInt32, u64 => UInt64,
);

impl From<i128> for Scalar {
    fn from(value: i128) -> Self {
        Self::Int128(Int128::new(value))
    }
}

impl From<u128> for Scalar {
    fn from(value: u128) -> Self {
        Self::UInt128(UInt128::new(value))
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
        Scalar::Int8(_) => (true, 8),
        Scalar::Int16(_) => (true, 16),
        Scalar::Int32(_) => (true, 32),
        Scalar::Int64(_) => (true, 64),
        Scalar::Int128(_) => (true, 128),
        Scalar::UInt8(_) => (false, 8),
        Scalar::UInt16(_) => (false, 16),
        Scalar::UInt32(_) => (false, 32),
        Scalar::UInt64(_) => (false, 64),
        Scalar::UInt128(_) => (false, 128),
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
        8 => i8::try_from(value).ok().map(Scalar::from),
        16 => i16::try_from(value).ok().map(Scalar::from),
        32 => i32::try_from(value).ok().map(Scalar::from),
        64 => i64::try_from(value).ok().map(Scalar::from),
        128 => Some(Scalar::from(value)),
        _ => None,
    }
}

fn unsigned_value_at(bits: u16, value: u128) -> Option<Scalar> {
    match bits {
        8 => u8::try_from(value).ok().map(Scalar::from),
        16 => u16::try_from(value).ok().map(Scalar::from),
        32 => u32::try_from(value).ok().map(Scalar::from),
        64 => u64::try_from(value).ok().map(Scalar::from),
        128 => Some(Scalar::from(value)),
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

/// Read an integer out of its canonical spelling.
///
/// The spelling is the one every integer width prints, at the widest signed
/// and unsigned storage this crate holds; the declared width then narrows it,
/// so a magnitude the column cannot hold is refused by the width rather than
/// wrapped here. Surrounding space is not part of the number.
pub(crate) fn integer_from_text(text: &str) -> Option<Scalar> {
    let text = text.trim();
    text.parse::<i128>()
        .ok()
        .map(Scalar::from)
        .or_else(|| text.parse::<u128>().ok().map(Scalar::from))
}
