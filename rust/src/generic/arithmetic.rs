//! Checked arithmetic for exact native values.

use std::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};

use smol_str::SmolStr;

use super::{Float16, Float32, Float64, Scalar, TemporalFamily};
use crate::{DataType, Error, I256, Result, TimeUnit, Timezone};

#[derive(Clone, Copy)]
pub(crate) enum Arithmetic {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone)]
enum ArithmeticTarget {
    Integer { signed: bool, bits: u16 },
    Float(u8),
    Decimal { wide: bool, scale: i8 },
    Temporal(DataType),
}

impl Arithmetic {
    const fn name(self) -> &'static str {
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
            Self::I8(value) => Self::I8(value.checked_neg().ok_or_else(|| overflow("i8"))?),
            Self::I16(value) => Self::I16(value.checked_neg().ok_or_else(|| overflow("i16"))?),
            Self::I32(value) => Self::I32(value.checked_neg().ok_or_else(|| overflow("i32"))?),
            Self::I64(value) => Self::I64(value.checked_neg().ok_or_else(|| overflow("i64"))?),
            Self::I128(value) => Self::I128(value.checked_neg().ok_or_else(|| overflow("i128"))?),
            Self::U8(value) => Self::I16(-i16::from(*value)),
            Self::U16(value) => Self::I32(-i32::from(*value)),
            Self::U32(value) => Self::I64(-i64::from(*value)),
            Self::U64(value) => Self::I128(-i128::from(*value)),
            Self::U128(_) => {
                return Err(invalid_unary(
                    "negation",
                    self,
                    "u128 has no lossless signed promotion",
                ));
            }
            Self::F16(value) => Self::F16(-*value),
            Self::F32(value) => Self::F32(-*value),
            Self::F64(value) => Self::F64(-*value),
            Self::D128(value, scale) => {
                Self::d128(value.checked_neg().ok_or_else(|| overflow("d128"))?, *scale)
            }
            Self::D256(value, scale) => {
                Self::d256(value.checked_neg().ok_or_else(|| overflow("d256"))?, *scale)
            }
            Self::Duration32(count, unit, _) => Self::duration32(
                count.checked_neg().ok_or_else(|| overflow("duration32"))?,
                *unit,
            )?,
            Self::Duration64(count, unit, _) => Self::duration64(
                count.checked_neg().ok_or_else(|| overflow("duration64"))?,
                *unit,
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
            Self::I8(value) => Self::I8(value.checked_abs().ok_or_else(|| overflow("i8"))?),
            Self::I16(value) => Self::I16(value.checked_abs().ok_or_else(|| overflow("i16"))?),
            Self::I32(value) => Self::I32(value.checked_abs().ok_or_else(|| overflow("i32"))?),
            Self::I64(value) => Self::I64(value.checked_abs().ok_or_else(|| overflow("i64"))?),
            Self::I128(value) => Self::I128(value.checked_abs().ok_or_else(|| overflow("i128"))?),
            Self::U8(_) | Self::U16(_) | Self::U32(_) | Self::U64(_) | Self::U128(_) => {
                self.clone()
            }
            Self::F16(value) => Self::F16(value.abs()),
            Self::F32(value) => Self::F32(value.abs()),
            Self::F64(value) => Self::F64(value.abs()),
            Self::D128(value, scale) => {
                Self::d128(value.checked_abs().ok_or_else(|| overflow("d128"))?, *scale)
            }
            Self::D256(value, scale) => Self::d256(
                if value.is_negative() {
                    value.checked_neg().ok_or_else(|| overflow("d256"))?
                } else {
                    *value
                },
                *scale,
            ),
            Self::Duration32(count, unit, _) => Self::duration32(
                count.checked_abs().ok_or_else(|| overflow("duration32"))?,
                *unit,
            )?,
            Self::Duration64(count, unit, _) => Self::duration64(
                count.checked_abs().ok_or_else(|| overflow("duration64"))?,
                *unit,
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
        let wide = matches!(left, Scalar::D256(..) | Scalar::I128(_) | Scalar::U128(_))
            || matches!(right, Scalar::D256(..) | Scalar::I128(_) | Scalar::U128(_));
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

#[derive(Clone, Copy)]
struct IntegerValueKind<'a> {
    value: &'a Scalar,
    signed: bool,
    bits: u16,
}

fn integer_value_kind(value: &Scalar) -> Option<IntegerValueKind<'_>> {
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

fn common_integer(
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

fn integer_kind(dtype: &DataType) -> Option<(bool, u16)> {
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

fn integer_arithmetic(
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

fn float_value_width(value: &Scalar) -> Option<u8> {
    value.as_float().map(|value| value.bit_width())
}

fn float_width(dtype: &DataType) -> Option<u8> {
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

fn float_arithmetic(
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
            Scalar::F16(Float16::from_f16(half::f16::from_f32(held)))
        }
        32 => Scalar::F32(Float32::from_f32(float_operation(
            left_number as f32,
            operation,
            right_number as f32,
        ))),
        _ => Scalar::F64(Float64::from_f64(float_operation(
            left_number,
            operation,
            right_number,
        ))),
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

fn is_exact_number(value: &Scalar) -> bool {
    value.as_integer().is_some() || value.as_decimal().is_some()
}

fn decimal_value_parts(value: &Scalar) -> Option<(I256, i8)> {
    value.as_decimal()
}

fn exact_value_parts(value: &Scalar) -> Option<(I256, i8)> {
    decimal_value_parts(value).or_else(|| {
        value
            .as_i128()
            .map(|value| (I256::from_i128(value), 0))
            .or_else(|| value.as_u128().map(|value| (I256::from_u128(value), 0)))
    })
}

fn decimal_target(dtype: &DataType) -> Option<(bool, i8)> {
    match dtype {
        DataType::Decimal32 { scale, .. }
        | DataType::Decimal64 { scale, .. }
        | DataType::Decimal128 { scale, .. } => Some((false, *scale)),
        DataType::Decimal256 { scale, .. } => Some((true, *scale)),
        _ => None,
    }
}

const fn result_decimal_scale(operation: Arithmetic, left: i8, right: i8) -> Option<i8> {
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
fn inferred_decimal_division_scale(
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

fn decimal_arithmetic(
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

#[derive(Clone)]
struct TemporalParts {
    family: TemporalFamily,
    unit: TimeUnit,
    zone: Timezone,
    dtype: DataType,
}

fn temporal_value_parts(value: &Scalar) -> Option<TemporalParts> {
    let temporal = value.as_temporal()?;
    let unit = temporal.unit();
    let zone = temporal.timezone().clone();
    let dtype = match (temporal.family(), temporal.bit_width()) {
        (TemporalFamily::Date, 32) => DataType::Date32,
        (TemporalFamily::Date, 64) => DataType::Date64,
        (TemporalFamily::Time, 32) => DataType::Time32(unit),
        (TemporalFamily::Time, 64) => DataType::Time64(unit),
        (TemporalFamily::DateTime, 64) => {
            DataType::Timestamp(unit, (!zone.is_naive()).then(|| zone.clone()))
        }
        (TemporalFamily::Duration, 32) => DataType::Duration32(unit),
        (TemporalFamily::Duration, 64) => DataType::Duration64(unit),
        _ => return None,
    };
    Some(TemporalParts {
        family: temporal.family(),
        unit,
        zone,
        dtype,
    })
}

fn temporal_target(dtype: &DataType) -> Option<(TemporalFamily, TimeUnit)> {
    match dtype {
        DataType::Date32 => Some((TemporalFamily::Date, TimeUnit::Day)),
        DataType::Date64 => Some((TemporalFamily::Date, TimeUnit::Millisecond)),
        DataType::Time32(unit) | DataType::Time64(unit) => Some((TemporalFamily::Time, *unit)),
        DataType::Timestamp(unit, _) => Some((TemporalFamily::DateTime, *unit)),
        DataType::Duration32(unit) | DataType::Duration64(unit) => {
            Some((TemporalFamily::Duration, *unit))
        }
        _ => None,
    }
}

fn temporal_result_type(
    left: &Scalar,
    left_parts: TemporalParts,
    operation: Arithmetic,
    right: &Scalar,
    right_parts: TemporalParts,
) -> Result<DataType> {
    match (left_parts.family, right_parts.family, operation) {
        (family, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if family != TemporalFamily::Duration =>
        {
            Ok(left_parts.dtype)
        }
        (TemporalFamily::Duration, family, Arithmetic::Add)
            if family != TemporalFamily::Duration =>
        {
            Ok(right_parts.dtype)
        }
        (family, other, Arithmetic::Sub)
            if family == other && family != TemporalFamily::Duration =>
        {
            if left_parts.zone.is_naive() != right_parts.zone.is_naive() {
                return Err(invalid_binary(
                    operation,
                    left,
                    right,
                    "zoned and timezone-naive temporal values cannot be subtracted",
                ));
            }
            let unit = finer_unit(left_parts.unit, right_parts.unit);
            DataType::duration64(unit)
                .map_err(|error| invalid_binary(operation, left, right, error.to_string()))
        }
        (TemporalFamily::Duration, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub) => {
            let unit = finer_unit(left_parts.unit, right_parts.unit);
            let wide = matches!(left_parts.dtype, DataType::Duration64(_))
                || matches!(right_parts.dtype, DataType::Duration64(_));
            if wide {
                DataType::duration64(unit)
            } else {
                DataType::duration32(unit)
            }
            .map_err(|error| invalid_binary(operation, left, right, error.to_string()))
        }
        _ => Err(invalid_binary(
            operation,
            left,
            right,
            "temporal arithmetic supports temporal +/- duration, temporal subtraction, and duration +/- duration",
        )),
    }
}

fn finer_unit(left: TimeUnit, right: TimeUnit) -> TimeUnit {
    if unit_rank(left) >= unit_rank(right) {
        left
    } else {
        right
    }
}

const fn unit_rank(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Day => 0,
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 2,
        TimeUnit::Microsecond => 3,
        TimeUnit::Nanosecond => 4,
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => 5,
    }
}

fn temporal_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &DataType,
) -> Result<Scalar> {
    if !matches!(operation, Arithmetic::Add | Arithmetic::Sub) {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "temporal multiplication, division, and remainder are undefined",
        ));
    }
    let left_parts = temporal_value_parts(left)
        .ok_or_else(|| invalid_binary(operation, left, right, "left operand is not temporal"))?;
    let right_parts = temporal_value_parts(right)
        .ok_or_else(|| invalid_binary(operation, left, right, "right operand is not temporal"))?;
    let (target_family, unit) = temporal_target(target).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "the promoted datatype is not temporal",
        )
    })?;
    let (left_count, right_count) = match (left_parts.family, right_parts.family, operation) {
        (family, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if family == target_family =>
        {
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        (TemporalFamily::Duration, family, Arithmetic::Add) if family == target_family => (
            temporal_at(right, unit, operation, left)?,
            temporal_at(left, unit, operation, right)?,
        ),
        (family, other, Arithmetic::Sub)
            if family == other
                && target_family == TemporalFamily::Duration
                && family != TemporalFamily::Duration =>
        {
            if left_parts.zone.is_naive() != right_parts.zone.is_naive() {
                return Err(invalid_binary(
                    operation,
                    left,
                    right,
                    "zoned and timezone-naive temporal values cannot be subtracted",
                ));
            }
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        (TemporalFamily::Duration, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if target_family == TemporalFamily::Duration =>
        {
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "operands do not match the promoted temporal result",
            ));
        }
    };
    let held = match operation {
        Arithmetic::Add => left_count.checked_add(right_count),
        Arithmetic::Sub => left_count.checked_sub(right_count),
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "temporal multiplication, division, and remainder are undefined",
            ));
        }
    }
    .ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })?;
    temporal_value(target, held, unit).map_err(|_| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })
}

fn duration_integer_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &DataType,
) -> Result<Scalar> {
    let (duration, integer, duration_first) = if temporal_value_parts(left)
        .is_some_and(|parts| parts.family == TemporalFamily::Duration)
        && right.as_integer().is_some()
    {
        (left, right, true)
    } else if left.as_integer().is_some()
        && temporal_value_parts(right).is_some_and(|parts| parts.family == TemporalFamily::Duration)
    {
        (right, left, false)
    } else {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "expected one duration and one integer",
        ));
    };
    if !(matches!(operation, Arithmetic::Mul)
        || duration_first && matches!(operation, Arithmetic::Div))
    {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "durations support multiplication by an integer and exact division by an integer",
        ));
    }

    let parts = temporal_value_parts(duration).ok_or_else(|| {
        invalid_binary(operation, left, right, "duration operand is not temporal")
    })?;
    let count = duration
        .temporal_count_at(parts.unit)
        .map(|value| I256::from_i128(i128::from(value)))
        .ok_or_else(|| invalid_binary(operation, left, right, "invalid duration count"))?;
    let scalar = exact_value_parts(integer)
        .map(|parts| parts.0)
        .ok_or_else(|| invalid_binary(operation, left, right, "integer is out of range"))?;
    if scalar.is_zero() && matches!(operation, Arithmetic::Div) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    let held = match operation {
        Arithmetic::Mul => count.checked_mul(scalar),
        Arithmetic::Div => {
            if !count
                .checked_rem(scalar)
                .is_some_and(|remainder| remainder.is_zero())
            {
                return Err(Error::InexactArithmetic {
                    operation: operation.name(),
                    kind: temporal_kind_name(target),
                });
            }
            count.checked_div(scalar)
        }
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "durations support multiplication or exact division by an integer",
            ));
        }
    }
    .and_then(I256::as_i128)
    .and_then(|value| i64::try_from(value).ok())
    .ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })?;
    temporal_value(target, held, parts.unit).map_err(|_| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })
}

fn temporal_at(
    value: &Scalar,
    unit: TimeUnit,
    operation: Arithmetic,
    other: &Scalar,
) -> Result<i64> {
    value.temporal_count_at(unit).ok_or_else(|| {
        invalid_binary(
            operation,
            value,
            other,
            "temporal unit conversion is inexact or out of range",
        )
    })
}

fn temporal_value(dtype: &DataType, count: i64, unit: TimeUnit) -> Result<Scalar> {
    match dtype {
        DataType::Date32 => Scalar::date32_in(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "date32",
            })?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Date64 => Scalar::date64_in(count, unit, Timezone::NAIVE),
        DataType::Time32(expected) if *expected == unit => Scalar::time32(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "time32",
            })?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Time64(expected) if *expected == unit => {
            Scalar::time64(count, unit, Timezone::NAIVE)
        }
        DataType::Timestamp(expected, zone) if *expected == unit => {
            Scalar::datetime64(count, unit, zone.clone().unwrap_or(Timezone::NAIVE))
        }
        DataType::Duration32(expected) if *expected == unit => Scalar::duration32(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "duration32",
            })?,
            unit,
        ),
        DataType::Duration64(expected) if *expected == unit => Scalar::duration64(count, unit),
        _ => Err(Error::InvalidArithmetic {
            operation: "temporal arithmetic",
            left: temporal_kind_name(dtype),
            right: None,
            reason: "invalid result unit".into(),
        }),
    }
}

const fn temporal_kind_name(dtype: &DataType) -> &'static str {
    match dtype {
        DataType::Date32 => "date32",
        DataType::Date64 => "date64",
        DataType::Time32(_) => "time32",
        DataType::Time64(_) => "time64",
        DataType::Timestamp(_, _) => "datetime64",
        DataType::Duration32(_) => "duration32",
        DataType::Duration64(_) => "duration64",
        _ => "temporal",
    }
}

fn invalid_binary(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_arithmetic_preserves_width_and_promotes_without_loss() {
        assert_eq!(
            Scalar::I8(3).checked_add(&Scalar::I8(4)).unwrap(),
            Scalar::I8(7)
        );
        assert_eq!(
            Scalar::I8(-1).checked_add(&Scalar::U8(2)).unwrap(),
            Scalar::I16(1)
        );
        assert!(matches!(
            Scalar::I8(127).checked_add(&Scalar::I8(1)),
            Err(Error::ArithmeticOverflow { kind: "i8", .. })
        ));
        assert!(matches!(
            Scalar::I128(1).checked_add(&Scalar::U128(1)),
            Err(Error::InvalidArithmetic { .. })
        ));
    }

    #[test]
    fn float_arithmetic_promotes_width_and_uses_checked_zero() {
        let left = Scalar::F16(Float16::from_f16(half::f16::from_f32(1.5)));
        let right = Scalar::F32(Float32::from_f32(2.0));
        assert_eq!(
            left.checked_mul(&right).unwrap(),
            Scalar::F32(Float32::from_f32(3.0))
        );
        assert!(matches!(
            right.checked_div(&Scalar::F32(Float32::from_f32(0.0))),
            Err(Error::DivisionByZero { .. })
        ));
        assert!(matches!(
            right.checked_rem(&Scalar::F32(Float32::from_f32(-0.0))),
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
            duration.checked_mul(&Scalar::I16(-3)).unwrap(),
            Scalar::duration32(-36, TimeUnit::Second).unwrap()
        );
        assert_eq!(
            Scalar::U8(3).checked_mul(&duration).unwrap(),
            Scalar::duration32(36, TimeUnit::Second).unwrap()
        );
        assert_eq!(
            duration.checked_div(&Scalar::I8(3)).unwrap(),
            Scalar::duration32(4, TimeUnit::Second).unwrap()
        );
        assert!(matches!(
            duration.checked_div(&Scalar::I8(5)),
            Err(Error::InexactArithmetic { .. })
        ));
        assert!(matches!(
            duration.checked_div(&Scalar::I8(0)),
            Err(Error::DivisionByZero { .. })
        ));
        assert!(Scalar::date32(1).checked_mul(&Scalar::I8(2)).is_err());
        assert!(Scalar::I8(2).checked_div(&duration).is_err());
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
                    .checked_arithmetic(&Scalar::I8(0), operation)
                    .unwrap(),
                Scalar::Null
            );
            assert_eq!(
                Scalar::I8(0)
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
            (&Scalar::I16(7) + &Scalar::I16(5)).unwrap(),
            Scalar::I16(12)
        );
        assert_eq!((-Scalar::U8(7)).unwrap(), Scalar::I16(-7));
        assert_eq!(Scalar::I16(-7).checked_abs().unwrap(), Scalar::I16(7));
        assert_eq!(
            Scalar::U128(u128::MAX).checked_abs().unwrap(),
            Scalar::U128(u128::MAX)
        );
        assert!(matches!(
            Scalar::I8(i8::MIN).checked_abs(),
            Err(Error::ArithmeticOverflow { .. })
        ));
        assert!((Scalar::from("a") + Scalar::from("b")).is_err());
    }
}
