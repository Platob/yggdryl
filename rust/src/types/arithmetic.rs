//! Checked arithmetic for exact native values.

use std::ops::{Add, Div, Mul, Neg, Rem, Sub};

use smol_str::SmolStr;

use super::decimal::Decimal;
use super::decimal::scalars::{
    decimal_arithmetic, decimal_target, decimal_value_parts, inferred_decimal_division_scale,
    is_exact_number, result_decimal_scale,
};
use super::floating::scalars::{Floating, float_arithmetic, float_value_width, float_width};
use super::integer::scalars::{
    Integer, common_integer, integer_arithmetic, integer_kind, integer_value_kind,
};
use super::scalar::Scalar;
use super::temporal::scalars::{
    Temporal, TemporalFamily, duration_integer_arithmetic, temporal_arithmetic,
    temporal_result_type, temporal_target, temporal_value_parts,
};
use crate::{DataType, Error, Result};

#[derive(Clone, Copy)]
pub(crate) enum Arithmetic {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone)]
pub(crate) enum ArithmeticTarget {
    Integer { signed: bool, bits: u16 },
    Float(u8),
    Decimal { wide: bool, scale: i8 },
    Temporal(DataType),
}

impl Arithmetic {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Add => "addition",
            Self::Sub => "subtraction",
            Self::Mul => "multiplication",
            Self::Div => "division",
            Self::Rem => "remainder",
        }
    }
}

impl Scalar {
    /// Add two numeric values using checked, width-aware promotion.
    pub fn checked_add(&self, other: &Self) -> Result<Self> {
        self.checked_arithmetic(other, Arithmetic::Add)
    }

    /// Subtract two numeric values using checked, width-aware promotion.
    pub fn checked_sub(&self, other: &Self) -> Result<Self> {
        self.checked_arithmetic(other, Arithmetic::Sub)
    }

    /// Multiply two numeric values using checked, width-aware promotion.
    pub fn checked_mul(&self, other: &Self) -> Result<Self> {
        self.checked_arithmetic(other, Arithmetic::Mul)
    }

    /// Divide two numeric values, refusing zero and inexact decimal quotients.
    pub fn checked_div(&self, other: &Self) -> Result<Self> {
        self.checked_arithmetic(other, Arithmetic::Div)
    }

    /// Return the checked numeric remainder, refusing a zero divisor.
    pub fn checked_rem(&self, other: &Self) -> Result<Self> {
        self.checked_arithmetic(other, Arithmetic::Rem)
    }

    /// Negate a signed numeric value or duration without overflow.
    ///
    /// Unsigned values promote to the next wider signed integer; `u128` has no
    /// lossless signed promotion and is therefore refused.
    pub fn checked_neg(&self) -> Result<Self> {
        if self.is_null() {
            return Ok(Self::Null);
        }
        let overflow = |kind| Error::ArithmeticOverflow {
            operation: "negation",
            kind,
        };
        Ok(match self {
            Self::Integer(Integer::I8(value)) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i8"))?)
            }
            Self::Integer(Integer::I16(value)) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i16"))?)
            }
            Self::Integer(Integer::I32(value)) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i32"))?)
            }
            Self::Integer(Integer::I64(value)) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i64"))?)
            }
            Self::Integer(Integer::I128(value)) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i128"))?)
            }
            Self::Integer(Integer::U8(value)) => Self::from(-i16::from(value.get())),
            Self::Integer(Integer::U16(value)) => Self::from(-i32::from(value.get())),
            Self::Integer(Integer::U32(value)) => Self::from(-i64::from(value.get())),
            Self::Integer(Integer::U64(value)) => Self::from(-i128::from(value.get())),
            Self::Integer(Integer::U128(_)) => {
                return Err(invalid_unary(
                    "negation",
                    self,
                    "u128 has no lossless signed promotion",
                ));
            }
            Self::Floating(Floating::F16(value)) => Self::Floating(Floating::F16(-*value)),
            Self::Floating(Floating::F32(value)) => Self::Floating(Floating::F32(-*value)),
            Self::Floating(Floating::F64(value)) => Self::Floating(Floating::F64(-*value)),
            Self::Decimal(Decimal::D32(value)) => {
                Self::Decimal(Decimal::D32(super::decimal::Decimal32::new(
                    value
                        .coefficient()
                        .checked_neg()
                        .ok_or_else(|| overflow("d32"))?,
                    value.scale(),
                )))
            }
            Self::Decimal(Decimal::D64(value)) => {
                Self::Decimal(Decimal::D64(super::decimal::Decimal64::new(
                    value
                        .coefficient()
                        .checked_neg()
                        .ok_or_else(|| overflow("d64"))?,
                    value.scale(),
                )))
            }
            Self::Decimal(Decimal::D128(value)) => Self::d128(
                value
                    .coefficient()
                    .checked_neg()
                    .ok_or_else(|| overflow("d128"))?,
                value.scale(),
            ),
            Self::Decimal(Decimal::D256(value)) => Self::d256(
                value
                    .coefficient()
                    .checked_neg()
                    .ok_or_else(|| overflow("d256"))?,
                value.scale(),
            ),
            Self::Temporal(Temporal::Duration32(value)) => Self::duration32(
                value
                    .count()
                    .checked_neg()
                    .ok_or_else(|| overflow("duration32"))?,
                value.unit(),
            )?,
            Self::Temporal(Temporal::Duration64(value)) => Self::duration64(
                value
                    .count()
                    .checked_neg()
                    .ok_or_else(|| overflow("duration64"))?,
                value.unit(),
            )?,
            _ => {
                return Err(invalid_unary(
                    "negation",
                    self,
                    "expected a signed number or duration",
                ));
            }
        })
    }

    /// Return the non-negative magnitude without changing its native width.
    pub fn checked_abs(&self) -> Result<Self> {
        if self.is_null() {
            return Ok(Self::Null);
        }
        let overflow = |kind| Error::ArithmeticOverflow {
            operation: "absolute value",
            kind,
        };
        Ok(match self {
            Self::Integer(Integer::I8(value)) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i8"))?)
            }
            Self::Integer(Integer::I16(value)) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i16"))?)
            }
            Self::Integer(Integer::I32(value)) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i32"))?)
            }
            Self::Integer(Integer::I64(value)) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i64"))?)
            }
            Self::Integer(Integer::I128(value)) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i128"))?)
            }
            Self::Integer(
                Integer::U8(_)
                | Integer::U16(_)
                | Integer::U32(_)
                | Integer::U64(_)
                | Integer::U128(_),
            ) => self.clone(),
            Self::Floating(Floating::F16(value)) => Self::Floating(Floating::F16(value.abs())),
            Self::Floating(Floating::F32(value)) => Self::Floating(Floating::F32(value.abs())),
            Self::Floating(Floating::F64(value)) => Self::Floating(Floating::F64(value.abs())),
            Self::Decimal(Decimal::D32(value)) => {
                Self::Decimal(Decimal::D32(super::decimal::Decimal32::new(
                    value
                        .coefficient()
                        .checked_abs()
                        .ok_or_else(|| overflow("d32"))?,
                    value.scale(),
                )))
            }
            Self::Decimal(Decimal::D64(value)) => {
                Self::Decimal(Decimal::D64(super::decimal::Decimal64::new(
                    value
                        .coefficient()
                        .checked_abs()
                        .ok_or_else(|| overflow("d64"))?,
                    value.scale(),
                )))
            }
            Self::Decimal(Decimal::D128(value)) => Self::d128(
                value
                    .coefficient()
                    .checked_abs()
                    .ok_or_else(|| overflow("d128"))?,
                value.scale(),
            ),
            Self::Decimal(Decimal::D256(value)) => Self::d256(
                if value.coefficient().is_negative() {
                    value
                        .coefficient()
                        .checked_neg()
                        .ok_or_else(|| overflow("d256"))?
                } else {
                    value.coefficient()
                },
                value.scale(),
            ),
            Self::Temporal(Temporal::Duration32(value)) => Self::duration32(
                value
                    .count()
                    .checked_abs()
                    .ok_or_else(|| overflow("duration32"))?,
                value.unit(),
            )?,
            Self::Temporal(Temporal::Duration64(value)) => Self::duration64(
                value
                    .count()
                    .checked_abs()
                    .ok_or_else(|| overflow("duration64"))?,
                value.unit(),
            )?,
            _ => {
                return Err(invalid_unary(
                    "absolute value",
                    self,
                    "expected a number or duration",
                ));
            }
        })
    }

    fn checked_arithmetic(&self, other: &Self, operation: Arithmetic) -> Result<Self> {
        if self.is_null() || other.is_null() {
            return Ok(Self::Null);
        }
        let target = inferred_target(self, operation, other)?;
        checked_arithmetic_target(self, operation, other, &target)
    }

    pub(crate) fn checked_arithmetic_as(
        &self,
        other: &Self,
        operation: Arithmetic,
        target: &DataType,
    ) -> Result<Self> {
        if self.is_null() || other.is_null() {
            return Ok(Self::Null);
        }
        let target = target_from_dtype(target).ok_or_else(|| {
            invalid_binary(
                operation,
                self,
                other,
                "the promoted datatype is not arithmetic",
            )
        })?;
        checked_arithmetic_target(self, operation, other, &target)
    }
}

fn checked_arithmetic_target(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &ArithmeticTarget,
) -> Result<Scalar> {
    match target {
        ArithmeticTarget::Integer { signed, bits } => {
            integer_arithmetic(left, operation, right, *signed, *bits)
        }
        ArithmeticTarget::Float(width) => float_arithmetic(left, operation, right, *width),
        ArithmeticTarget::Decimal { wide, scale } => {
            decimal_arithmetic(left, operation, right, *wide, *scale)
        }
        ArithmeticTarget::Temporal(dtype) => {
            if matches!(temporal_target(dtype), Some((TemporalFamily::Duration, _)))
                && ((temporal_value_parts(left)
                    .is_some_and(|parts| parts.family == TemporalFamily::Duration)
                    && right.as_integer().is_some())
                    || (left.as_integer().is_some()
                        && temporal_value_parts(right)
                            .is_some_and(|parts| parts.family == TemporalFamily::Duration)))
            {
                duration_integer_arithmetic(left, operation, right, dtype)
            } else {
                temporal_arithmetic(left, operation, right, dtype)
            }
        }
    }
}

fn target_from_dtype(dtype: &DataType) -> Option<ArithmeticTarget> {
    if let Some((signed, bits)) = integer_kind(dtype) {
        return Some(ArithmeticTarget::Integer { signed, bits });
    }
    if let Some(width) = float_width(dtype) {
        return Some(ArithmeticTarget::Float(width));
    }
    if let Some((wide, scale)) = decimal_target(dtype) {
        return Some(ArithmeticTarget::Decimal { wide, scale });
    }
    temporal_target(dtype).map(|_| ArithmeticTarget::Temporal(dtype.clone()))
}

fn inferred_target(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
) -> Result<ArithmeticTarget> {
    if let (Some(left), Some(right)) = (integer_value_kind(left), integer_value_kind(right)) {
        return common_integer(left, right).ok_or_else(|| {
            invalid_binary(
                operation,
                left.value,
                right.value,
                "the integer widths have no lossless common promotion",
            )
        });
    }

    match (
        temporal_value_parts(left),
        integer_value_kind(right),
        operation,
    ) {
        (Some(parts), Some(_), Arithmetic::Mul | Arithmetic::Div)
            if parts.family == TemporalFamily::Duration =>
        {
            return Ok(ArithmeticTarget::Temporal(parts.dtype));
        }
        _ => {}
    }
    match (
        integer_value_kind(left),
        temporal_value_parts(right),
        operation,
    ) {
        (Some(_), Some(parts), Arithmetic::Mul) if parts.family == TemporalFamily::Duration => {
            return Ok(ArithmeticTarget::Temporal(parts.dtype));
        }
        _ => {}
    }

    let left_decimal = decimal_value_parts(left);
    let right_decimal = decimal_value_parts(right);
    if left_decimal.is_some() || right_decimal.is_some() {
        if !is_exact_number(left) || !is_exact_number(right) {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "exact decimals cannot mix with approximate or non-numeric values",
            ));
        }
        let left_scale = left_decimal.map_or(0, |parts| parts.1);
        let right_scale = right_decimal.map_or(0, |parts| parts.1);
        let wide = matches!(
            left,
            Scalar::Decimal(Decimal::D256(_))
                | Scalar::Integer(Integer::I128(_) | Integer::U128(_))
        ) || matches!(
            right,
            Scalar::Decimal(Decimal::D256(_))
                | Scalar::Integer(Integer::I128(_) | Integer::U128(_))
        );
        let scale = match operation {
            Arithmetic::Div => {
                inferred_decimal_division_scale(left, left_scale, right, right_scale, wide)?
            }
            _ => result_decimal_scale(operation, left_scale, right_scale).ok_or_else(|| {
                Error::ArithmeticOverflow {
                    operation: operation.name(),
                    kind: if wide { "d256" } else { "d128" },
                }
            })?,
        };
        let valid = if wide {
            DataType::decimal256(76, scale)
        } else {
            DataType::decimal128(38, scale)
        };
        valid.map_err(|error| invalid_binary(operation, left, right, error.to_string()))?;
        return Ok(ArithmeticTarget::Decimal { wide, scale });
    }

    let left_float = float_value_width(left);
    let right_float = float_value_width(right);
    if left_float.is_some() || right_float.is_some() {
        if !(left_float.is_some() || left.as_integer().is_some())
            || !(right_float.is_some() || right.as_integer().is_some())
        {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "floats combine only with floats or integers",
            ));
        }
        let width = if left.as_integer().is_some() || right.as_integer().is_some() {
            64
        } else {
            left_float.unwrap_or(16).max(right_float.unwrap_or(16))
        };
        return Ok(ArithmeticTarget::Float(width));
    }

    if let (Some(left_temporal), Some(right_temporal)) =
        (temporal_value_parts(left), temporal_value_parts(right))
    {
        return temporal_result_type(left, left_temporal, operation, right, right_temporal)
            .map(ArithmeticTarget::Temporal);
    }

    Err(invalid_binary(
        operation,
        left,
        right,
        "expected numeric or compatible temporal operands",
    ))
}

pub(crate) fn invalid_binary(
    operation: Arithmetic,
    left: &Scalar,
    right: &Scalar,
    reason: impl Into<SmolStr>,
) -> Error {
    Error::InvalidArithmetic {
        operation: operation.name(),
        left: left.kind(),
        right: Some(right.kind()),
        reason: reason.into(),
    }
}

fn invalid_unary(operation: &'static str, value: &Scalar, reason: &'static str) -> Error {
    Error::InvalidArithmetic {
        operation,
        left: value.kind(),
        right: None,
        reason: reason.into(),
    }
}

macro_rules! value_binary_operator {
    ($trait:ident, $method:ident, $checked:ident) => {
        impl $trait for Scalar {
            type Output = Result<Scalar>;

            fn $method(self, other: Self) -> Self::Output {
                self.$checked(&other)
            }
        }

        impl $trait<&Scalar> for Scalar {
            type Output = Result<Scalar>;

            fn $method(self, other: &Scalar) -> Self::Output {
                self.$checked(other)
            }
        }

        impl $trait<Scalar> for &Scalar {
            type Output = Result<Scalar>;

            fn $method(self, other: Scalar) -> Self::Output {
                self.$checked(&other)
            }
        }

        impl $trait<&Scalar> for &Scalar {
            type Output = Result<Scalar>;

            fn $method(self, other: &Scalar) -> Self::Output {
                self.$checked(other)
            }
        }
    };
}

value_binary_operator!(Add, add, checked_add);
value_binary_operator!(Sub, sub, checked_sub);
value_binary_operator!(Mul, mul, checked_mul);
value_binary_operator!(Div, div, checked_div);
value_binary_operator!(Rem, rem, checked_rem);

impl Neg for Scalar {
    type Output = Result<Self>;

    fn neg(self) -> Self::Output {
        self.checked_neg()
    }
}

impl Neg for &Scalar {
    type Output = Result<Scalar>;

    fn neg(self) -> Self::Output {
        self.checked_neg()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Float16, Float32, Float64, I256, TimeUnit, Timezone};

    #[test]
    fn integer_arithmetic_preserves_width_and_promotes_without_loss() {
        assert_eq!(
            Scalar::from(3_i8).checked_add(&Scalar::from(4_i8)).unwrap(),
            Scalar::from(7_i8)
        );
        assert_eq!(
            Scalar::from(-1_i8)
                .checked_add(&Scalar::from(2_u8))
                .unwrap(),
            Scalar::from(1_i16)
        );
        assert!(matches!(
            Scalar::from(127_i8).checked_add(&Scalar::from(1_i8)),
            Err(Error::ArithmeticOverflow { kind: "i8", .. })
        ));
        assert!(matches!(
            Scalar::from(1_i128).checked_add(&Scalar::from(1_u128)),
            Err(Error::InvalidArithmetic { .. })
        ));
    }

    #[test]
    fn float_arithmetic_promotes_width_and_uses_checked_zero() {
        let left = Scalar::from(Float16::from_f16(half::f16::from_f32(1.5)));
        let right = Scalar::from(Float32::from_f32(2.0));
        assert_eq!(
            left.checked_mul(&right).unwrap(),
            Scalar::from(Float32::from_f32(3.0))
        );
        assert!(matches!(
            right.checked_div(&Scalar::from(Float32::from_f32(0.0))),
            Err(Error::DivisionByZero { .. })
        ));
        assert!(matches!(
            right.checked_rem(&Scalar::from(Float32::from_f32(-0.0))),
            Err(Error::DivisionByZero { .. })
        ));
    }

    #[test]
    fn float_wrappers_keep_their_width_for_native_arithmetic() {
        let mut half = Float16::from_f16(half::f16::from_f32(1.5));
        half += Float16::from_f16(half::f16::from_f32(0.5));
        assert_eq!(half, Float16::from_f16(half::f16::from_f32(2.0)));
        assert_eq!(
            Float32::from_f32(7.0) % Float32::from_f32(4.0),
            Float32::from_f32(3.0)
        );
        assert_eq!(-Float64::from_f64(2.5), Float64::from_f64(-2.5));
        assert_eq!(Float32::from_f32(-0.0).abs(), Float32::from_f32(0.0));
    }

    #[test]
    fn decimal_arithmetic_is_scale_exact() {
        assert_eq!(
            Scalar::d128(105, 2)
                .checked_add(&Scalar::d128(2, 1))
                .unwrap(),
            Scalar::d128(125, 2)
        );
        assert_eq!(
            Scalar::d128(1, 0).checked_div(&Scalar::d128(2, 0)).unwrap(),
            Scalar::d128(5, 1)
        );
        assert_eq!(
            Scalar::d128(100, 2)
                .checked_div(&Scalar::d128(2, 0))
                .unwrap(),
            Scalar::d128(5, 1)
        );
        assert_eq!(
            Scalar::d128(1, 0)
                .checked_div(&Scalar::d128(128, 0))
                .unwrap(),
            Scalar::d128(78_125, 7)
        );
        assert!(matches!(
            Scalar::d128(1, 0).checked_div(&Scalar::d128(3, 0)),
            Err(Error::InexactArithmetic { .. })
        ));

        let maximum = Scalar::d128(i128::MAX, 0);
        assert_eq!(maximum.checked_div(&maximum).unwrap(), Scalar::d128(1, 0));
        let wide: I256 =
            "9999999999999999999999999999999999999999999999999999999999999999999999999999"
                .parse()
                .unwrap();
        let maximum = Scalar::d256(wide, 0);
        assert_eq!(
            maximum.checked_div(&maximum).unwrap(),
            Scalar::d256(I256::from_i128(1), 0)
        );
        assert_eq!(
            Scalar::d128(3, 0).checked_div(&Scalar::d128(6, 0)).unwrap(),
            Scalar::d128(5, 1)
        );

        let denominator = (0..75).fold(I256::from_i128(1), |value, _| {
            value.checked_mul(I256::from_i128(2)).unwrap()
        });
        let coefficient = (0..75).fold(I256::from_i128(1), |value, _| {
            value.checked_mul(I256::from_i128(5)).unwrap()
        });
        assert_eq!(
            Scalar::d256(I256::from_i128(1), 0)
                .checked_div(&Scalar::d256(denominator, 0))
                .unwrap(),
            Scalar::d256(coefficient, 75)
        );
    }

    #[test]
    fn temporal_arithmetic_uses_durations_and_preserves_zone() {
        let at = Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC).unwrap();
        let elapsed = Scalar::duration64(2, TimeUnit::Second).unwrap();
        assert_eq!(
            at.checked_add(&elapsed).unwrap(),
            Scalar::datetime64(3_000, TimeUnit::Millisecond, Timezone::UTC).unwrap()
        );
        assert_eq!(
            at.checked_sub(&Scalar::datetime64(500, TimeUnit::Millisecond, Timezone::UTC).unwrap())
                .unwrap(),
            Scalar::duration64(500, TimeUnit::Millisecond).unwrap()
        );
        assert_eq!(
            Scalar::date32(2).checked_sub(&Scalar::date32(1)).unwrap(),
            Scalar::duration64(1, TimeUnit::Day).unwrap()
        );
        assert_eq!(
            Scalar::date64(86_400_001)
                .checked_sub(&Scalar::date32(1))
                .unwrap(),
            Scalar::duration64(1, TimeUnit::Millisecond).unwrap()
        );
        assert_eq!(
            Scalar::datetime64(2, TimeUnit::Second, Timezone::NAIVE)
                .unwrap()
                .checked_sub(
                    &Scalar::datetime64(1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE,)
                        .unwrap(),
                )
                .unwrap(),
            Scalar::duration64(1_000_000_000, TimeUnit::Nanosecond).unwrap()
        );
    }

    #[test]
    fn durations_scale_only_by_exact_integers() {
        let duration = Scalar::duration32(12, TimeUnit::Second).unwrap();
        assert_eq!(
            duration.checked_mul(&Scalar::from(-3_i16)).unwrap(),
            Scalar::duration32(-36, TimeUnit::Second).unwrap()
        );
        assert_eq!(
            Scalar::from(3_u8).checked_mul(&duration).unwrap(),
            Scalar::duration32(36, TimeUnit::Second).unwrap()
        );
        assert_eq!(
            duration.checked_div(&Scalar::from(3_i8)).unwrap(),
            Scalar::duration32(4, TimeUnit::Second).unwrap()
        );
        assert!(matches!(
            duration.checked_div(&Scalar::from(5_i8)),
            Err(Error::InexactArithmetic { .. })
        ));
        assert!(matches!(
            duration.checked_div(&Scalar::from(0_i8)),
            Err(Error::DivisionByZero { .. })
        ));
        assert!(Scalar::date32(1).checked_mul(&Scalar::from(2_i8)).is_err());
        assert!(Scalar::from(2_i8).checked_div(&duration).is_err());
    }

    #[test]
    fn null_propagates_through_every_binary_operation() {
        for operation in [
            Arithmetic::Add,
            Arithmetic::Sub,
            Arithmetic::Mul,
            Arithmetic::Div,
            Arithmetic::Rem,
        ] {
            assert_eq!(
                Scalar::Null
                    .checked_arithmetic(&Scalar::from(0_i8), operation)
                    .unwrap(),
                Scalar::Null
            );
            assert_eq!(
                Scalar::from(0_i8)
                    .checked_arithmetic(&Scalar::Null, operation)
                    .unwrap(),
                Scalar::Null
            );
        }
        assert_eq!(Scalar::Null.checked_neg().unwrap(), Scalar::Null);
        assert_eq!(Scalar::Null.checked_abs().unwrap(), Scalar::Null);
    }

    #[test]
    fn operator_traits_are_checked_and_do_not_concatenate() {
        assert_eq!(
            (&Scalar::from(7_i16) + &Scalar::from(5_i16)).unwrap(),
            Scalar::from(12_i16)
        );
        assert_eq!((-Scalar::from(7_u8)).unwrap(), Scalar::from(-7_i16));
        assert_eq!(
            Scalar::from(-7_i16).checked_abs().unwrap(),
            Scalar::from(7_i16)
        );
        assert_eq!(
            Scalar::from(u128::MAX).checked_abs().unwrap(),
            Scalar::from(u128::MAX)
        );
        assert!(matches!(
            Scalar::from(i8::MIN).checked_abs(),
            Err(Error::ArithmeticOverflow { .. })
        ));
        assert!((Scalar::from("a") + Scalar::from("b")).is_err());
    }
}
