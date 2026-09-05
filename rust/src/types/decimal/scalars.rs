//! Exact coefficient-and-scale decimals matching Arrow storage.
//!
//! `from_decimal` selects `D128` or `D256`. Scale remains part of the stored
//! value, while equality, ordering, and hashing compare the represented number.
//!
//! ```
//! use yggdryl::{I256, Scalar};
//!
//! let price = Scalar::from_decimal(I256::from_i128(1_050), 2);
//!
//! assert_eq!(price.as_decimal(), Some((I256::from_i128(1_050), 2)));
//! assert_eq!(price, Scalar::from_decimal(I256::from_i128(105), 1));
//! assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::format_smolstr;

use crate::types::arithmetic::{Arithmetic, invalid_binary};
use crate::types::typed::define_scalar_type;
use crate::types::value::{ValidationFailure, expected};
use crate::{
    DataType, DataTypeId, DataTypeKind, Error, I256, Result, Scalar, ScalarFamily, ScalarValue,
};

/// Operations shared by every exact-decimal representation.
pub trait DecimalValue: crate::ScalarValue {
    /// Return the coefficient widened to 256 bits.
    fn coefficient(&self) -> I256;
    /// Return the decimal scale.
    fn scale(&self) -> i8;
    /// Return this value represented at `scale` without losing precision.
    fn rescale(self, scale: i8) -> Result<Self>;
}

trait IntoI256 {
    fn into_i256(self) -> I256;
}

macro_rules! into_i256 {
    ($($native:ty),+ $(,)?) => {$(
        impl IntoI256 for $native {
            fn into_i256(self) -> I256 {
                I256::from_i128(self as i128)
            }
        }
    )+};
}

into_i256!(i32, i64, i128);

impl IntoI256 for I256 {
    fn into_i256(self) -> I256 {
        self
    }
}

macro_rules! decimal_leaf {
    ($name:ident, $coefficient:ty) => {
        #[doc = concat!("One exact `", stringify!($coefficient), "` coefficient and scale.")]
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
        pub struct $name {
            coefficient: $coefficient,
            scale: i8,
        }

        impl $name {
            /// Construct an exact decimal representation.
            pub const fn new(coefficient: $coefficient, scale: i8) -> Self {
                Self { coefficient, scale }
            }

            /// Return the stored coefficient.
            pub const fn coefficient(&self) -> $coefficient {
                self.coefficient
            }

            /// Return the base-10 scale.
            pub const fn scale(&self) -> i8 {
                self.scale
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&decimal_text(self.coefficient.into_i256(), self.scale))
            }
        }
    };
}

decimal_leaf!(Decimal32, i32);
decimal_leaf!(Decimal64, i64);
decimal_leaf!(Decimal128, i128);
decimal_leaf!(Decimal256, I256);

/// One exact decimal coefficient width and scale.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Decimal {
    /// Signed 32-bit coefficient.
    D32(Decimal32),
    /// Signed 64-bit coefficient.
    D64(Decimal64),
    /// Signed 128-bit coefficient.
    D128(Decimal128),
    /// Signed 256-bit coefficient.
    D256(Decimal256),
}

impl Decimal {
    /// Return the coefficient widened losslessly to 256 bits.
    pub fn coefficient(self) -> I256 {
        match self {
            Self::D32(value) => value.coefficient().into_i256(),
            Self::D64(value) => value.coefficient().into_i256(),
            Self::D128(value) => value.coefficient().into_i256(),
            Self::D256(value) => value.coefficient(),
        }
    }

    /// Return the base-10 scale.
    pub const fn scale(self) -> i8 {
        match self {
            Self::D32(value) => value.scale(),
            Self::D64(value) => value.scale(),
            Self::D128(value) => value.scale(),
            Self::D256(value) => value.scale(),
        }
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&decimal_text(self.coefficient(), self.scale()))
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(
            self.coefficient(),
            self.scale(),
            other.coefficient(),
            other.scale(),
        )
    }
}

impl Hash for Decimal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        normalize(self.coefficient(), self.scale()).hash(state);
    }
}

const _: () = assert!(std::mem::size_of::<Decimal>() == 48);

macro_rules! decimal_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $native:ty) => {
        impl ScalarValue for $leaf {
            type Family = Decimal;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Decimal;

            fn dtype(&self) -> Result<DataType> {
                Scalar::Decimal(Decimal::$variant(*self)).dtype()
            }

            fn into_family(self) -> Self::Family {
                Decimal::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Decimal::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Decimal(Decimal::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Decimal(Decimal::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl DecimalValue for $leaf {
            fn coefficient(&self) -> I256 {
                self.coefficient().into_i256()
            }

            fn scale(&self) -> i8 {
                self.scale()
            }

            fn rescale(self, scale: i8) -> Result<Self> {
                let coefficient =
                    rescale_decimal(self.coefficient().into_i256(), self.scale(), scale)
                        .and_then(I256::as_i128)
                        .and_then(|value| <$native>::try_from(value).ok())
                        .ok_or(Error::InexactArithmetic {
                            operation: "rescale",
                            kind: stringify!($id),
                        })?;
                Ok(Self::new(coefficient, scale))
            }
        }
    };
}

decimal_value!(Decimal32, super::Decimal32Type, D32, Decimal32, i32);
decimal_value!(Decimal64, super::Decimal64Type, D64, Decimal64, i64);
decimal_value!(Decimal128, super::Decimal128Type, D128, Decimal128, i128);

impl ScalarValue for Decimal256 {
    type Family = Decimal;
    type Type = super::Decimal256Type;

    const ID: DataTypeId = DataTypeId::Decimal256;
    const KIND: DataTypeKind = DataTypeKind::Decimal;

    fn dtype(&self) -> Result<DataType> {
        Scalar::Decimal(Decimal::D256(*self)).dtype()
    }

    fn into_family(self) -> Self::Family {
        Decimal::D256(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            Decimal::D256(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Decimal(Decimal::D256(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Decimal(Decimal::D256(value)) => Some(value),
            _ => None,
        }
    }
}

impl DecimalValue for Decimal256 {
    fn coefficient(&self) -> I256 {
        self.coefficient()
    }

    fn scale(&self) -> i8 {
        self.scale()
    }

    fn rescale(self, scale: i8) -> Result<Self> {
        rescale_decimal(self.coefficient(), self.scale(), scale)
            .map(|coefficient| Self::new(coefficient, scale))
            .ok_or(Error::InexactArithmetic {
                operation: "rescale",
                kind: "Decimal256",
            })
    }
}

impl ScalarFamily for Decimal {
    const KIND: DataTypeKind = DataTypeKind::Decimal;

    fn id(&self) -> DataTypeId {
        match self {
            Self::D32(_) => DataTypeId::Decimal32,
            Self::D64(_) => DataTypeId::Decimal64,
            Self::D128(_) => DataTypeId::Decimal128,
            Self::D256(_) => DataTypeId::Decimal256,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        self.into_scalar().dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Decimal(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Decimal(value) => Some(value),
            _ => None,
        }
    }
}

define_scalar_type!(Decimal32Scalar, super::Decimal32Type, "decimal32");
define_scalar_type!(Decimal64Scalar, super::Decimal64Type, "decimal64");
define_scalar_type!(Decimal128Scalar, super::Decimal128Type, "decimal128");
define_scalar_type!(Decimal256Scalar, super::Decimal256Type, "decimal256");

impl Scalar {
    /// Build the narrowest exact decimal width that holds `unscaled`.
    pub fn from_decimal(unscaled: I256, scale: i8) -> Self {
        unscaled.as_i128().map_or_else(
            || Self::d256(unscaled, scale),
            |value| Self::d128(value, scale),
        )
    }

    /// Build an exact decimal from an unscaled integer and a scale.
    ///
    /// The value is `unscaled * 10^-scale`, so `Scalar::d128(1_050, 2)` is
    /// `10.50`. A negative scale multiplies instead, exactly as Arrow allows.
    pub const fn d128(unscaled: i128, scale: i8) -> Self {
        Self::Decimal(Decimal::D128(Decimal128::new(unscaled, scale)))
    }

    /// Build an exact decimal with a 256-bit coefficient.
    pub const fn d256(unscaled: I256, scale: i8) -> Self {
        Self::Decimal(Decimal::D256(Decimal256::new(unscaled, scale)))
    }

    /// Return the coefficient and scale when this is a 128-bit decimal.
    pub const fn as_d128(&self) -> Option<(i128, i8)> {
        match self {
            Self::Decimal(Decimal::D128(value)) => Some((value.coefficient(), value.scale())),
            _ => None,
        }
    }

    /// Return the coefficient and scale when this is a 256-bit decimal.
    pub const fn as_d256(&self) -> Option<(I256, i8)> {
        match self {
            Self::Decimal(Decimal::D256(value)) => Some((value.coefficient(), value.scale())),
            _ => None,
        }
    }

    /// Return this decimal's coefficient widened to 256 bits and its scale.
    pub fn as_decimal(&self) -> Option<(I256, i8)> {
        match self {
            Self::Decimal(value) => Some((value.coefficient(), value.scale())),
            _ => None,
        }
    }

    /// Return whether this value is an exact decimal.
    pub const fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(_))
    }

    /// Return this decimal's unscaled integer at `scale`, when it is exact.
    ///
    /// A column declares its own scale, so a decimal written into one has to be
    /// restated at that scale. Restating to fewer fractional digits would throw
    /// digits away, so it answers `None` rather than rounding.
    pub fn decimal_unscaled_at(&self, scale: i8) -> Option<i128> {
        let (unscaled, current) = self.as_decimal()?;
        let unscaled = unscaled.as_i128()?;
        let shift = i32::from(scale) - i32::from(current);
        match shift.cmp(&0) {
            Ordering::Equal => Some(unscaled),
            Ordering::Greater => scale_up_signed(unscaled, shift),
            // Removing digits is only exact when every digit removed is a zero.
            Ordering::Less => {
                let divisor = scale_up_signed(1, -shift)?;
                (unscaled % divisor == 0).then(|| unscaled / divisor)
            }
        }
    }

    /// Return this decimal's 256-bit coefficient at `scale`, when exact.
    pub fn decimal256_unscaled_at(&self, scale: i8) -> Option<I256> {
        let (unscaled, current) = self.as_decimal()?;
        let shift = i32::from(scale) - i32::from(current);
        match shift.cmp(&0) {
            Ordering::Equal => Some(unscaled),
            Ordering::Greater => (0..shift).try_fold(unscaled, |held, _| held.checked_mul_ten()),
            Ordering::Less => (0..-shift).try_fold(unscaled, |held, _| held.divided_by_ten()),
        }
    }

    /// Render an exact decimal without passing through a float.
    pub fn into_decimal_utf8(&self) -> Option<String> {
        let (coefficient, scale) = self.as_decimal()?;
        Some(decimal_text(coefficient, scale))
    }
}

/// Render a coefficient and scale in ordinary decimal notation.
pub(crate) fn decimal_text(coefficient: I256, scale: i8) -> String {
    let encoded = coefficient.to_string();
    if scale == 0 {
        return encoded;
    }
    let (sign, digits) = encoded
        .strip_prefix('-')
        .map_or(("", encoded.as_str()), |digits| ("-", digits));
    if scale < 0 {
        return format!(
            "{sign}{digits}{}",
            "0".repeat(usize::from(scale.unsigned_abs()))
        );
    }
    let scale = usize::from(scale.unsigned_abs());
    if digits.len() > scale {
        let split = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..split], &digits[split..])
    } else {
        format!("{sign}0.{}{digits}", "0".repeat(scale - digits.len()))
    }
}

/// Strip the trailing zeros a decimal's coefficient carries.
///
/// Equal numbers share exactly one normal form, which is what lets `Hash` agree
/// with the numeric `Ord` below without either of them widening the coefficient.
pub(crate) fn normalize(unscaled: I256, scale: i8) -> (I256, i8) {
    if unscaled.is_zero() {
        return (I256::ZERO, 0);
    }
    let mut unscaled = unscaled;
    let mut scale = scale;
    while scale > i8::MIN {
        let Some(reduced) = unscaled.divided_by_ten() else {
            break;
        };
        unscaled = reduced;
        scale -= 1;
    }
    (unscaled, scale)
}

/// Compare two decimals by the number each one names.
pub(crate) fn compare(
    left_unscaled: I256,
    left_scale: i8,
    right_unscaled: I256,
    right_scale: i8,
) -> Ordering {
    let (left_unscaled, left_scale) = normalize(left_unscaled, left_scale);
    let (right_unscaled, right_scale) = normalize(right_unscaled, right_scale);
    if left_scale == right_scale {
        return left_unscaled.cmp(&right_unscaled);
    }
    let sign = left_unscaled
        .cmp(&I256::ZERO)
        .cmp(&right_unscaled.cmp(&I256::ZERO));
    if sign != Ordering::Equal {
        return sign;
    }

    // Same sign, different scale: bring the coefficient with fewer fractional
    // digits up to the other's scale and compare magnitudes. A product that no
    // longer fits is by that fact the larger magnitude, because the coefficient
    // it is compared against did fit.
    if left_scale < right_scale {
        scale_up(
            left_unscaled,
            i32::from(right_scale) - i32::from(left_scale),
        )
        .map_or_else(
            || {
                if left_unscaled.is_negative() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            },
            |left| left.cmp(&right_unscaled),
        )
    } else {
        scale_up(
            right_unscaled,
            i32::from(left_scale) - i32::from(right_scale),
        )
        .map_or_else(
            || {
                if right_unscaled.is_negative() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            },
            |right| left_unscaled.cmp(&right),
        )
    }
}

/// Multiply a magnitude by ten `digits` times, or report that it overflowed.
fn scale_up(unscaled: I256, digits: i32) -> Option<I256> {
    (0..digits).try_fold(unscaled, |held, _| held.checked_mul_ten())
}

/// Multiply a signed coefficient by ten `digits` times, or report the overflow.
fn scale_up_signed(unscaled: i128, digits: i32) -> Option<i128> {
    (0..digits).try_fold(unscaled, |unscaled, _| unscaled.checked_mul(10))
}

pub(crate) fn validate_decimal_value(
    value: &Scalar,
    precision: u8,
    width: u16,
) -> std::result::Result<(), ValidationFailure> {
    let Some(integer) = value.as_i128() else {
        return Err(expected("unscaled decimal integer", value));
    };
    let fits_width = match width {
        32 => i32::try_from(integer).is_ok(),
        64 => i64::try_from(integer).is_ok(),
        _ => true,
    };
    if !fits_width || decimal_digits(integer.unsigned_abs()) > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal value exceeds precision {precision} or physical width {width}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_decimal256_value(
    value: &Scalar,
    precision: u8,
    scale: i8,
) -> std::result::Result<(), ValidationFailure> {
    let Some(coefficient) = (if value.is_decimal() {
        value.decimal256_unscaled_at(scale)
    } else {
        value.as_i128().map(I256::from_i128)
    }) else {
        return Err(expected("d256", value));
    };
    let encoded = coefficient.to_string();
    let digits = encoded.trim_start_matches('-');
    if digits.len() > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal256 value exceeds precision {precision}"
        )));
    }
    Ok(())
}

fn decimal_digits(value: u128) -> usize {
    let mut digits = 1;
    let mut remaining = value / 10;
    while remaining > 0 {
        digits += 1;
        remaining /= 10;
    }
    digits
}

#[cfg(test)]
mod tests;

pub(crate) fn is_exact_number(value: &Scalar) -> bool {
    value.as_integer().is_some() || value.as_decimal().is_some()
}

pub(crate) fn decimal_value_parts(value: &Scalar) -> Option<(I256, i8)> {
    value.as_decimal()
}

pub(crate) fn exact_value_parts(value: &Scalar) -> Option<(I256, i8)> {
    decimal_value_parts(value).or_else(|| {
        value
            .as_i128()
            .map(|value| (I256::from_i128(value), 0))
            .or_else(|| value.as_u128().map(|value| (I256::from_u128(value), 0)))
    })
}

pub(crate) fn decimal_target(dtype: &DataType) -> Option<(bool, i8)> {
    match dtype {
        DataType::Decimal32 { scale, .. }
        | DataType::Decimal64 { scale, .. }
        | DataType::Decimal128 { scale, .. } => Some((false, *scale)),
        DataType::Decimal256 { scale, .. } => Some((true, *scale)),
        _ => None,
    }
}

pub(crate) const fn result_decimal_scale(operation: Arithmetic, left: i8, right: i8) -> Option<i8> {
    match operation {
        Arithmetic::Add | Arithmetic::Sub | Arithmetic::Rem => {
            Some(if left > right { left } else { right })
        }
        Arithmetic::Mul => left.checked_add(right),
        Arithmetic::Div => None,
    }
}

/// Select the smallest non-negative scale that represents an inferred exact
/// quotient. After reducing the coefficients, only factors of two and five in
/// the denominator can terminate in base ten. Input scales shift the required
/// power of ten; the selected result never keeps arbitrary padding zeros.
pub(crate) fn inferred_decimal_division_scale(
    left: &Scalar,
    left_scale: i8,
    right: &Scalar,
    right_scale: i8,
    wide: bool,
) -> Result<i8> {
    let (left_number, _) = exact_value_parts(left).ok_or_else(|| {
        invalid_binary(
            Arithmetic::Div,
            left,
            right,
            "left operand is not an exact number",
        )
    })?;
    let (right_number, _) = exact_value_parts(right).ok_or_else(|| {
        invalid_binary(
            Arithmetic::Div,
            left,
            right,
            "right operand is not an exact number",
        )
    })?;
    if right_number.is_zero() || left_number.is_zero() {
        return Ok(0);
    }

    let divisor = signed_gcd(left_number, right_number)
        .ok_or_else(|| decimal_overflow(Arithmetic::Div, wide))?;
    let numerator = left_number
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(Arithmetic::Div, wide))?;
    let mut denominator = right_number
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(Arithmetic::Div, wide))?;
    let mut twos = 0_i16;
    while let Some(reduced) = divide_exactly(denominator, 2) {
        denominator = reduced;
        twos += 1;
    }
    let mut fives = 0_i16;
    while let Some(reduced) = divide_exactly(denominator, 5) {
        denominator = reduced;
        fives += 1;
    }
    if denominator != I256::from_i128(1) && denominator != I256::from_i128(-1) {
        return Err(inexact_decimal_division(wide));
    }

    let required = twos.max(fives);
    let scale = (required + i16::from(left_scale) - i16::from(right_scale)).max(0);
    let (numerator, numerator_twos) = factor_power(numerator, 2);
    let (_, numerator_fives) = factor_power(numerator, 5);
    let trailing_zeroes =
        (numerator_twos + required - twos).min(numerator_fives + required - fives);
    let scale = (scale - trailing_zeroes).max(0);
    let maximum = if wide { 76 } else { 38 };
    if scale > maximum {
        return Err(decimal_overflow(Arithmetic::Div, wide));
    }
    i8::try_from(scale).map_err(|_| decimal_overflow(Arithmetic::Div, wide))
}

fn factor_power(mut value: I256, factor: i128) -> (I256, i16) {
    let mut count = 0;
    while let Some(reduced) = divide_exactly(value, factor) {
        value = reduced;
        count += 1;
    }
    (value, count)
}

pub(crate) fn decimal_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    wide: bool,
    target_scale: i8,
) -> Result<Scalar> {
    let (left_number, left_scale) = exact_value_parts(left).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "left operand is not an exact number",
        )
    })?;
    let (right_number, right_scale) = exact_value_parts(right).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "right operand is not an exact number",
        )
    })?;
    if right_number.is_zero() && matches!(operation, Arithmetic::Div | Arithmetic::Rem) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    let held = match operation {
        Arithmetic::Add | Arithmetic::Sub | Arithmetic::Rem => {
            let left = rescale_decimal(left_number, left_scale, target_scale);
            let right = rescale_decimal(right_number, right_scale, target_scale);
            let (Some(left), Some(right)) = (left, right) else {
                return Err(decimal_overflow(operation, wide));
            };
            match operation {
                Arithmetic::Add => left.checked_add(right),
                Arithmetic::Sub => left.checked_sub(right),
                Arithmetic::Rem => left.checked_rem(right),
                _ => unreachable!(),
            }
        }
        Arithmetic::Mul => left_number.checked_mul(right_number).and_then(|held| {
            rescale_decimal(held, left_scale.checked_add(right_scale)?, target_scale)
        }),
        Arithmetic::Div => Some(exact_scaled_division(
            left_number,
            left_scale,
            right_number,
            right_scale,
            target_scale,
            wide,
        )?),
    }
    .ok_or_else(|| decimal_overflow(operation, wide))?;
    if wide {
        Ok(Scalar::d256(held, target_scale))
    } else {
        held.as_i128()
            .map(|held| Scalar::d128(held, target_scale))
            .ok_or_else(|| decimal_overflow(operation, false))
    }
}

/// Divide two scaled coefficients exactly without first multiplying either
/// full-width operand by a power of ten. Reducing first means `MAX / MAX`
/// reaches one instead of reporting overflow from an unnecessary intermediate.
fn exact_scaled_division(
    left: I256,
    left_scale: i8,
    right: I256,
    right_scale: i8,
    target_scale: i8,
    wide: bool,
) -> Result<I256> {
    let operation = Arithmetic::Div;
    let exponent = i16::from(target_scale) + i16::from(right_scale) - i16::from(left_scale);
    let divisor = signed_gcd(left, right).ok_or_else(|| decimal_overflow(operation, wide))?;
    let mut numerator = left
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(operation, wide))?;
    let mut denominator = right
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(operation, wide))?;

    if exponent >= 0 {
        let places = exponent.unsigned_abs();
        let mut twos = 0;
        let mut fives = 0;
        while twos < places {
            let Some(reduced) = divide_exactly(denominator, 2) else {
                break;
            };
            denominator = reduced;
            twos += 1;
        }
        while fives < places {
            let Some(reduced) = divide_exactly(denominator, 5) else {
                break;
            };
            denominator = reduced;
            fives += 1;
        }
        numerator = apply_denominator_sign(numerator, denominator)
            .ok_or_else(|| inexact_decimal_division(wide))?;
        for _ in twos..places {
            numerator = numerator
                .checked_mul(I256::from_i128(2))
                .ok_or_else(|| decimal_overflow(operation, wide))?;
        }
        for _ in fives..places {
            numerator = numerator
                .checked_mul(I256::from_i128(5))
                .ok_or_else(|| decimal_overflow(operation, wide))?;
        }
        return Ok(numerator);
    }

    numerator = apply_denominator_sign(numerator, denominator)
        .ok_or_else(|| inexact_decimal_division(wide))?;
    for _ in 0..exponent.unsigned_abs() {
        numerator = numerator
            .divided_by_ten()
            .ok_or_else(|| inexact_decimal_division(wide))?;
    }
    Ok(numerator)
}

/// Euclid's algorithm can stay signed, which also handles the I256 minimum:
/// only the final common factor is normalized, and the minimum is left signed.
fn signed_gcd(mut left: I256, mut right: I256) -> Option<I256> {
    while !right.is_zero() {
        let remainder = left.checked_rem(right)?;
        left = right;
        right = remainder;
    }
    if left.is_negative() {
        left.checked_neg().or(Some(left))
    } else {
        Some(left)
    }
}

fn divide_exactly(value: I256, divisor: i128) -> Option<I256> {
    let divisor = I256::from_i128(divisor);
    value
        .checked_rem(divisor)
        .filter(|remainder| remainder.is_zero())
        .and_then(|_| value.checked_div(divisor))
}

fn apply_denominator_sign(numerator: I256, denominator: I256) -> Option<I256> {
    if denominator == I256::from_i128(1) {
        Some(numerator)
    } else if denominator == I256::from_i128(-1) {
        numerator.checked_neg()
    } else {
        None
    }
}

const fn inexact_decimal_division(wide: bool) -> Error {
    Error::InexactArithmetic {
        operation: Arithmetic::Div.name(),
        kind: if wide { "d256" } else { "d128" },
    }
}

fn rescale_decimal(value: I256, from: i8, to: i8) -> Option<I256> {
    match to.cmp(&from) {
        std::cmp::Ordering::Greater => {
            scale_decimal_up(value, u16::try_from(i16::from(to) - i16::from(from)).ok()?)
        }
        std::cmp::Ordering::Less => (0..u16::try_from(i16::from(from) - i16::from(to)).ok()?)
            .try_fold(value, |held, _| held.divided_by_ten()),
        std::cmp::Ordering::Equal => Some(value),
    }
}

fn scale_decimal_up(value: I256, places: u16) -> Option<I256> {
    (0..places).try_fold(value, |held, _| held.checked_mul_ten())
}

const fn decimal_overflow(operation: Arithmetic, wide: bool) -> Error {
    Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: if wide { "d256" } else { "d128" },
    }
}
