//! Checked arithmetic for exact native values.

use std::ops::{Add, Div, Mul, Neg, Rem, Sub};

use smol_str::SmolStr;

use crate::code_scalars;
use crate::decimal::{
    decimal_arithmetic, decimal_target, decimal_value_parts, inferred_decimal_division_scale,
    is_exact_number, result_decimal_scale,
};
use crate::floating::{float_arithmetic, float_value_width, float_width};
use crate::integer::{common_integer, integer_arithmetic, integer_kind, integer_value_kind};
use crate::scalar::Scalar;
use crate::temporal::scalars::{
    TemporalKind, duration_integer_arithmetic, temporal_arithmetic, temporal_result_type,
    temporal_target, temporal_value_parts,
};
use crate::{DataType, Error, Result};

#[derive(Clone, Copy)]
pub enum Arithmetic {
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
            Self::Int8(value) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i8"))?)
            }
            Self::Int16(value) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i16"))?)
            }
            Self::Int32(value) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i32"))?)
            }
            Self::Int64(value) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i64"))?)
            }
            Self::Int128(value) => {
                Self::from(value.get().checked_neg().ok_or_else(|| overflow("i128"))?)
            }
            Self::UInt8(value) => Self::from(-i16::from(value.get())),
            Self::UInt16(value) => Self::from(-i32::from(value.get())),
            Self::UInt32(value) => Self::from(-i64::from(value.get())),
            Self::UInt64(value) => Self::from(-i128::from(value.get())),
            Self::UInt128(_) => {
                return Err(invalid_unary(
                    "negation",
                    self,
                    "u128 has no lossless signed promotion",
                ));
            }
            Self::Float16(value) => Self::Float16(-*value),
            Self::Float32(value) => Self::Float32(-*value),
            Self::Float64(value) => Self::Float64(-*value),
            Self::Decimal32(value) => Self::Decimal32(crate::decimal::Decimal32::new(
                value
                    .coefficient()
                    .checked_neg()
                    .ok_or_else(|| overflow("d32"))?,
                value.scale(),
            )),
            Self::Decimal64(value) => Self::Decimal64(crate::decimal::Decimal64::new(
                value
                    .coefficient()
                    .checked_neg()
                    .ok_or_else(|| overflow("d64"))?,
                value.scale(),
            )),
            Self::Decimal128(value) => Self::d128(
                value
                    .coefficient()
                    .checked_neg()
                    .ok_or_else(|| overflow("d128"))?,
                value.scale(),
            ),
            Self::Decimal256(value) => Self::d256(
                value
                    .coefficient()
                    .checked_neg()
                    .ok_or_else(|| overflow("d256"))?,
                value.scale(),
            ),
            Self::Duration32(value) => Self::duration32(
                value
                    .count()
                    .checked_neg()
                    .ok_or_else(|| overflow("duration32"))?,
                value.unit(),
            )?,
            Self::Duration64(value) => Self::duration64(
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
            Self::Int8(value) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i8"))?)
            }
            Self::Int16(value) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i16"))?)
            }
            Self::Int32(value) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i32"))?)
            }
            Self::Int64(value) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i64"))?)
            }
            Self::Int128(value) => {
                Self::from(value.get().checked_abs().ok_or_else(|| overflow("i128"))?)
            }
            Self::UInt8(_)
            | Self::UInt16(_)
            | Self::UInt32(_)
            | Self::UInt64(_)
            | Self::UInt128(_) => self.clone(),
            Self::Float16(value) => Self::Float16(value.abs()),
            Self::Float32(value) => Self::Float32(value.abs()),
            Self::Float64(value) => Self::Float64(value.abs()),
            Self::Decimal32(value) => Self::Decimal32(crate::decimal::Decimal32::new(
                value
                    .coefficient()
                    .checked_abs()
                    .ok_or_else(|| overflow("d32"))?,
                value.scale(),
            )),
            Self::Decimal64(value) => Self::Decimal64(crate::decimal::Decimal64::new(
                value
                    .coefficient()
                    .checked_abs()
                    .ok_or_else(|| overflow("d64"))?,
                value.scale(),
            )),
            Self::Decimal128(value) => Self::d128(
                value
                    .coefficient()
                    .checked_abs()
                    .ok_or_else(|| overflow("d128"))?,
                value.scale(),
            ),
            Self::Decimal256(value) => Self::d256(
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
            Self::Duration32(value) => Self::duration32(
                value
                    .count()
                    .checked_abs()
                    .ok_or_else(|| overflow("duration32"))?,
                value.unit(),
            )?,
            Self::Duration64(value) => Self::duration64(
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
        if let Some(joined) = concatenated(self, operation, other) {
            return Ok(joined);
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
            if matches!(temporal_target(dtype), Some((TemporalKind::Duration, _)))
                && ((temporal_value_parts(left)
                    .is_some_and(|parts| parts.family == TemporalKind::Duration)
                    && right.is_integer())
                    || (left.is_integer()
                        && temporal_value_parts(right)
                            .is_some_and(|parts| parts.family == TemporalKind::Duration)))
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

/// Join two values of one repertoire under `+`, or answer `None`.
///
/// Addition over text, bytes and sequences is concatenation: joining is what
/// `+` means for a repertoire that has no sum.
///
/// This is deliberately not the expression language's `concat`, which stays
/// where it is. That one is a variadic *text* function that also renders a
/// version and answers null for anything it cannot read; this one is a binary
/// join over one repertoire that refuses what it does not know. Same English
/// word, two verbs - folding them together would drag version rendering into
/// the operator or turn `concat`'s null into an error.
///
/// Only `+`. Subtraction, multiplication, division and remainder over text
/// stay refusals, because none of them names anything a reader would agree
/// on. The two sides must be the same repertoire: text joins text, bytes join
/// bytes, a sequence extends a sequence.
///
/// A code joins as the text it is, and the result is a plain string rather
/// than a code - `FR` and `X` concatenated are not a country. Geospatial
/// values are deliberately absent even though they read as bytes: two WKB
/// payloads laid end to end are not a geometry.
fn concatenated(left: &Scalar, operation: Arithmetic, right: &Scalar) -> Option<Scalar> {
    if !matches!(operation, Arithmetic::Add) {
        return None;
    }
    match (left, right) {
        (Scalar::String(_) | code_scalars!(), Scalar::String(_) | code_scalars!()) => {
            let (left, right) = (left.as_str()?, right.as_str()?);
            let mut joined = String::with_capacity(left.len() + right.len());
            joined.push_str(left);
            joined.push_str(right);
            Some(Scalar::from(SmolStr::new(joined)))
        }
        (Scalar::Bytes(left), Scalar::Bytes(right)) => {
            let (left, right) = (left.as_bytes(), right.as_bytes());
            let mut joined = Vec::with_capacity(left.len() + right.len());
            joined.extend_from_slice(left);
            joined.extend_from_slice(right);
            Some(Scalar::from(std::sync::Arc::<[u8]>::from(joined)))
        }
        (Scalar::Sequence(left), Scalar::Sequence(right)) => Some(Scalar::from_sequence(
            left.rows().iter().chain(right.rows().iter()).cloned(),
        )),
        _ => None,
    }
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
            if parts.family == TemporalKind::Duration =>
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
        (Some(_), Some(parts), Arithmetic::Mul) if parts.family == TemporalKind::Duration => {
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
            Scalar::Decimal256(_) | Scalar::Int128(_) | Scalar::UInt128(_)
        ) || matches!(
            right,
            Scalar::Decimal256(_) | Scalar::Int128(_) | Scalar::UInt128(_)
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
        if !(left_float.is_some() || left.is_integer())
            || !(right_float.is_some() || right.is_integer())
        {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "floats combine only with floats or integers",
            ));
        }
        let width = if left.is_integer() || right.is_integer() {
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/arithmetic.rs` pins and a caller cannot reach.
    //!
    //! [`Arithmetic`] names which operation a binary walk is performing, and
    //! a caller reaches it only through the operator traits, so null
    //! propagation has to be asked for one operation at a time. The crate
    //! root declares `arithmetic` privately, so the name reaches nobody
    //! without the feature; the dispatcher itself stays behind a forwarder.

    pub use super::Arithmetic;
    use crate::{Result, Scalar};

    /// Perform one binary operation over two values.
    pub fn checked_arithmetic(
        left: &Scalar,
        right: &Scalar,
        operation: Arithmetic,
    ) -> Result<Scalar> {
        left.checked_arithmetic(right, operation)
    }
}
