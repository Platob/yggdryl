//! Arithmetic over [`ScalarValue`] — the `add` / `sub` / `mul` / `div` / `neg` engine the
//! [`Scalar`](crate::Scalar) trait exposes by default, plus the value-level
//! [`cast`](ScalarValue::cast). Numeric operands **promote** (an integer widens to a float
//! when mixed, a decimal carries through its scale); a few **temporal** combinations are
//! defined — durations scale (`*` / `/` by an integer) and add, a duration shifts a date /
//! time / timestamp, and two instants subtract to a duration — and everything else is a
//! [`ScalarError::Unsupported`], so every value type either computes or says why it can't.
//!
//! A `null` operand propagates: the result is a typed `null` of the operation's result
//! type. The whole engine works in the widened representations [`ScalarValue`] already
//! holds (`i128` integers, `f64` floats, `i256` decimals, the core temporal types), so it
//! never narrows silently — an integer overflow is an error, not a wraparound.

use arrow_buffer::i256;
use yggdryl_schema::DataType;

use crate::error::{ScalarError, ScalarResult};
use crate::value::ScalarValue;

/// One of the four binary arithmetic operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
        }
    }
}

impl ScalarValue {
    /// `self + rhs`, promoting / combining types per the module rules (raises
    /// [`ScalarError::Unsupported`] for a combination with no defined sum).
    pub fn add(&self, rhs: &ScalarValue) -> ScalarResult<ScalarValue> {
        self.arith(rhs, Op::Add)
    }

    /// `self - rhs` (see [`add`](ScalarValue::add)).
    pub fn sub(&self, rhs: &ScalarValue) -> ScalarResult<ScalarValue> {
        self.arith(rhs, Op::Sub)
    }

    /// `self * rhs` (see [`add`](ScalarValue::add)).
    pub fn mul(&self, rhs: &ScalarValue) -> ScalarResult<ScalarValue> {
        self.arith(rhs, Op::Mul)
    }

    /// `self / rhs` — integer / float / duration division (raises on a zero divisor, and is
    /// [`Unsupported`](ScalarError::Unsupported) for decimals, which need an explicit
    /// rounding scale).
    pub fn div(&self, rhs: &ScalarValue) -> ScalarResult<ScalarValue> {
        self.arith(rhs, Op::Div)
    }

    /// `-self` — negates an integer / float / decimal / duration; raises for the rest.
    pub fn neg(&self) -> ScalarResult<ScalarValue> {
        use ScalarValue::*;
        if self.is_null() {
            return Ok(self.clone());
        }
        match self {
            Int {
                value,
                bits,
                signed,
            } => {
                // Negating a non-zero unsigned value cannot be represented as that unsigned
                // type — surface it rather than wrap.
                if !*signed && *value != 0 {
                    return Err(ScalarError::Invalid(format!(
                        "cannot negate the unsigned value {value}; use a signed type"
                    )));
                }
                Ok(ScalarValue::int(
                    value.checked_neg().ok_or_else(|| overflow(Op::Sub))?,
                    *bits,
                    *signed,
                ))
            }
            Float { value, bits } => Ok(ScalarValue::float(-value.0, *bits)),
            Decimal {
                value,
                precision,
                scale,
                bits,
            } => Ok(ScalarValue::decimal(
                i256::ZERO - *value,
                *precision,
                *scale,
                *bits,
            )),
            Duration { value, unit } => Ok(ScalarValue::Duration {
                value: value.checked_neg().ok_or_else(|| overflow(Op::Sub))?,
                unit: *unit,
            }),
            other => Err(ScalarError::Unsupported(format!(
                "cannot negate a '{}' value",
                other.data_type().to_str()
            ))),
        }
    }

    /// Casts this value to `dtype` by running Arrow's cast kernel over its length-1 array
    /// (the value-level companion to [`Serie::cast`](yggdryl_serie); lossy / narrowing
    /// casts yield a typed `null`). Casting to the **same** type is a no-op.
    pub fn cast(&self, dtype: &DataType) -> ScalarResult<ScalarValue> {
        if self.data_type() == *dtype {
            return Ok(self.clone());
        }
        let target = dtype.to_arrow()?;
        let array = arrow_cast::cast(self.to_array()?.as_ref(), &target)?;
        ScalarValue::from_array(array.as_ref(), 0)
    }

    /// The shared dispatcher behind [`add`](ScalarValue::add) / [`sub`](ScalarValue::sub) /
    /// [`mul`](ScalarValue::mul) / [`div`](ScalarValue::div).
    fn arith(&self, rhs: &ScalarValue, op: Op) -> ScalarResult<ScalarValue> {
        let (lt, rt) = (self.data_type(), rhs.data_type());
        let numeric_pair = is_numeric_type(&lt) && is_numeric_type(&rt);
        // The op must be defined for these *types* — so a null operand does not turn an
        // otherwise-unsupported combination into a (wrong) successful null.
        if !numeric_pair && !temporal_defined(&lt, &rt, op) {
            return Err(ScalarError::Unsupported(unsupported_msg(self, rhs, op)));
        }
        // A null operand then propagates as a typed null of the result type.
        if self.is_null() || rhs.is_null() {
            return Ok(ScalarValue::Null(lt.common_type(&rt).unwrap_or(lt)));
        }
        if numeric_pair {
            return numeric(self, rhs, op);
        }
        // Defined temporal combination: `temporal` returns `Some` (its own error inside).
        temporal(self, rhs, op).expect("temporal_defined implies a temporal result")
    }
}

/// Numeric arithmetic with int → float → decimal promotion.
fn numeric(lhs: &ScalarValue, rhs: &ScalarValue, op: Op) -> ScalarResult<ScalarValue> {
    use ScalarValue::*;
    let is_float = |v: &ScalarValue| matches!(v, Float { .. });
    let is_dec = |v: &ScalarValue| matches!(v, Decimal { .. });

    // A float anywhere widens the whole operation to f64.
    if is_float(lhs) || is_float(rhs) {
        let (a, b) = (to_f64(lhs), to_f64(rhs));
        let bits = float_bits(lhs).max(float_bits(rhs));
        let r = match op {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            Op::Div => {
                if b == 0.0 {
                    return Err(div_by_zero());
                }
                a / b
            }
        };
        return Ok(ScalarValue::float(r, bits));
    }

    // A decimal (with no float) keeps decimal precision.
    if is_dec(lhs) || is_dec(rhs) {
        return decimal(lhs, rhs, op);
    }

    // Both integers — compute in i128, promoting width and signedness.
    let (av, ab, asg) = int_parts(lhs);
    let (bv, bb, bsg) = int_parts(rhs);
    let bits = ab.max(bb);
    let signed = asg || bsg;
    let value = match op {
        Op::Add => av.checked_add(bv),
        Op::Sub => av.checked_sub(bv),
        Op::Mul => av.checked_mul(bv),
        Op::Div => {
            if bv == 0 {
                return Err(div_by_zero());
            }
            av.checked_div(bv)
        }
    }
    .ok_or_else(|| overflow(op))?;
    // An unsigned result must not go negative — that would silently wrap on Arrow
    // conversion (a uint cell of `2^bits + value`). Surface it instead.
    if !signed && value < 0 {
        return Err(ScalarError::Invalid(format!(
            "unsigned '{}' underflowed to {value}; use a signed type",
            op.symbol()
        )));
    }
    Ok(ScalarValue::int(value, bits, signed))
}

/// Decimal add / sub (scale-aligned) and mul (scales add); div needs a rounding scale and
/// is unsupported. The result is sized (128 vs 256-bit) to hold its value losslessly, or an
/// error if it exceeds the 256-bit maximum — never a silent truncation.
fn decimal(lhs: &ScalarValue, rhs: &ScalarValue, op: Op) -> ScalarResult<ScalarValue> {
    let (av, asc, _) = dec_parts(lhs);
    let (bv, bsc, _) = dec_parts(rhs);
    let (value, scale) = match op {
        Op::Add | Op::Sub => {
            let scale = asc.max(bsc);
            let a = scale_up(av, (scale - asc).max(0) as u32)?;
            let b = scale_up(bv, (scale - bsc).max(0) as u32)?;
            let v = if op == Op::Add {
                a.checked_add(b)
            } else {
                a.checked_sub(b)
            }
            .ok_or_else(|| overflow(op))?;
            (v, scale)
        }
        Op::Mul => {
            let v = av.checked_mul(bv).ok_or_else(|| overflow(op))?;
            (v, asc + bsc)
        }
        Op::Div => {
            return Err(ScalarError::Unsupported(
                "decimal division needs an explicit result scale; cast to float first".into(),
            ))
        }
    };
    fit_decimal(value, scale)
}

/// Sizes a decimal `(value, scale)` into the narrowest storage that holds it losslessly
/// (128- or 256-bit), erroring if it needs more than the 256-bit maximum of 76 digits.
fn fit_decimal(value: i256, scale: i8) -> ScalarResult<ScalarValue> {
    let needed = i256_digits(value).max(scale.max(0) as u32);
    let (bits, precision) = if needed <= 38 {
        (128u16, 38u8)
    } else if needed <= 76 {
        (256, 76)
    } else {
        return Err(ScalarError::Invalid(format!(
            "decimal result needs {needed} digits, exceeding the 256-bit maximum of 76; \
             cast to float first"
        )));
    };
    Ok(ScalarValue::decimal(value, precision, scale, bits))
}

/// The number of decimal digits in `|value|` (`0` has one digit).
fn i256_digits(value: i256) -> u32 {
    value.to_string().trim_start_matches('-').len() as u32
}

/// Temporal arithmetic — `Some(result)` for a defined combination (the result carries its
/// own error, e.g. division by zero), `None` for a combination this engine does not define.
fn temporal(lhs: &ScalarValue, rhs: &ScalarValue, op: Op) -> Option<ScalarResult<ScalarValue>> {
    use ScalarValue::*;
    let result: ScalarResult<ScalarValue> = match (lhs, rhs) {
        // duration ± duration
        (Duration { .. }, Duration { .. }) if matches!(op, Op::Add | Op::Sub) => {
            let (a, b) = (lhs.as_duration().unwrap(), rhs.as_duration().unwrap());
            Ok(ScalarValue::from_duration(
                &(if op == Op::Add { a.add(&b) } else { a.sub(&b) }),
            ))
        }
        // duration * / int  (and int * duration)
        (Duration { .. }, Int { value, .. }) if matches!(op, Op::Mul | Op::Div) => {
            scale_duration(lhs.as_duration().unwrap(), *value, op)
        }
        (Int { value, .. }, Duration { .. }) if op == Op::Mul => {
            scale_duration(rhs.as_duration().unwrap(), *value, Op::Mul)
        }
        // date ± duration
        (Date { .. }, Duration { .. }) if matches!(op, Op::Add | Op::Sub) => {
            let (d, span) = (lhs.as_date().unwrap(), rhs.as_duration().unwrap());
            Ok(ScalarValue::from_date(
                &(if op == Op::Add {
                    d.add(&span)
                } else {
                    d.sub(&span)
                }),
            ))
        }
        // timestamp ± duration
        (Timestamp { .. }, Duration { .. }) if matches!(op, Op::Add | Op::Sub) => {
            let (t, span) = (lhs.as_datetime().unwrap(), rhs.as_duration().unwrap());
            Ok(ScalarValue::from_datetime(
                &(if op == Op::Add {
                    t.add(&span)
                } else {
                    t.sub(&span)
                }),
            ))
        }
        // time ± duration (wraps within the day)
        (Time { .. }, Duration { .. }) if matches!(op, Op::Add | Op::Sub) => {
            let (t, span) = (lhs.as_time().unwrap(), rhs.as_duration().unwrap());
            Ok(ScalarValue::from_time(
                &(if op == Op::Add {
                    t.add(&span)
                } else {
                    t.sub(&span)
                }),
            ))
        }
        // instant - instant -> duration
        (Date { .. }, Date { .. }) if op == Op::Sub => Ok(ScalarValue::from_duration(
            &(lhs.as_date().unwrap() - rhs.as_date().unwrap()),
        )),
        (Timestamp { .. }, Timestamp { .. }) if op == Op::Sub => Ok(ScalarValue::from_duration(
            &(lhs.as_datetime().unwrap() - rhs.as_datetime().unwrap()),
        )),
        (Time { .. }, Time { .. }) if op == Op::Sub => Ok(ScalarValue::from_duration(
            &(lhs.as_time().unwrap() - rhs.as_time().unwrap()),
        )),
        _ => return None,
    };
    Some(result)
}

/// `duration * factor` or `duration / factor`, surfacing a zero divisor (and a factor that
/// exceeds the `i64` range) as an actionable error.
fn scale_duration(d: yggdryl_core::Duration, factor: i128, op: Op) -> ScalarResult<ScalarValue> {
    let factor = i64::try_from(factor)
        .map_err(|_| ScalarError::Invalid("duration scale factor exceeds the i64 range".into()))?;
    let r = match op {
        Op::Mul => d.mul(factor),
        Op::Div => {
            if factor == 0 {
                return Err(div_by_zero());
            }
            d.div(factor)
        }
        _ => unreachable!("scale_duration is only called for * and /"),
    };
    Ok(ScalarValue::from_duration(&r))
}

/// Whether a [`DataType`] participates in the numeric promotion lattice (int / float /
/// decimal).
fn is_numeric_type(dt: &DataType) -> bool {
    dt.is_integer() || dt.is_floating() || dt.is_decimal()
}

/// Whether `op` is a defined temporal combination for these types (mirrors [`temporal`]).
fn temporal_defined(lt: &DataType, rt: &DataType, op: Op) -> bool {
    use DataType::*;
    let add_sub = matches!(op, Op::Add | Op::Sub);
    match (lt, rt) {
        (Duration { .. }, Duration { .. }) => add_sub,
        (Duration { .. }, Int { .. }) => matches!(op, Op::Mul | Op::Div),
        (Int { .. }, Duration { .. }) => op == Op::Mul,
        (Date { .. }, Duration { .. })
        | (Timestamp { .. }, Duration { .. })
        | (Time { .. }, Duration { .. }) => add_sub,
        (Date { .. }, Date { .. })
        | (Timestamp { .. }, Timestamp { .. })
        | (Time { .. }, Time { .. }) => op == Op::Sub,
        _ => false,
    }
}

/// The `(value, bits, signed)` of an integer value.
fn int_parts(v: &ScalarValue) -> (i128, u16, bool) {
    match v {
        ScalarValue::Int {
            value,
            bits,
            signed,
        } => (*value, *bits, *signed),
        _ => (0, 64, true),
    }
}

/// A numeric value as `f64` (a decimal divides by `10^scale`).
fn to_f64(v: &ScalarValue) -> f64 {
    match v {
        ScalarValue::Int { value, .. } => *value as f64,
        ScalarValue::Float { value, .. } => value.0,
        ScalarValue::Decimal { value, scale, .. } => {
            // Parse the full-width unscaled value (`i256` can exceed `i128`) directly to f64.
            let unscaled: f64 = value.to_string().parse().unwrap_or(f64::NAN);
            unscaled / 10f64.powi(*scale as i32)
        }
        _ => f64::NAN,
    }
}

/// The float bit width of a value (defaulting to 64 for a non-float operand).
fn float_bits(v: &ScalarValue) -> u16 {
    match v {
        ScalarValue::Float { bits, .. } => *bits,
        _ => 64,
    }
}

/// The `(unscaled, scale, bits)` of a decimal, treating an integer as a scale-0 decimal.
fn dec_parts(v: &ScalarValue) -> (i256, i8, u16) {
    match v {
        ScalarValue::Decimal {
            value, scale, bits, ..
        } => (*value, *scale, *bits),
        ScalarValue::Int { value, bits, .. } => (i256::from_i128(*value), 0, (*bits).max(128)),
        _ => (i256::ZERO, 0, 128),
    }
}

/// Multiplies an `i256` by `10^by` (used to align decimal scales). The power is built in
/// `i256` (not `i128`), so a shift of up to ~76 digits does not spuriously overflow.
fn scale_up(value: i256, by: u32) -> ScalarResult<i256> {
    if by == 0 {
        return Ok(value);
    }
    let ten = i256::from_i128(10);
    let mut factor = i256::from_i128(1);
    for _ in 0..by {
        factor = factor.checked_mul(ten).ok_or_else(|| overflow(Op::Mul))?;
    }
    value.checked_mul(factor).ok_or_else(|| overflow(Op::Mul))
}

/// The error for a divisor of zero.
fn div_by_zero() -> ScalarError {
    ScalarError::Invalid("division by zero".into())
}

/// The error for an arithmetic overflow.
fn overflow(op: Op) -> ScalarError {
    ScalarError::Invalid(format!("arithmetic overflow in '{}'", op.symbol()))
}

/// The message for an undefined type combination.
fn unsupported_msg(lhs: &ScalarValue, rhs: &ScalarValue, op: Op) -> String {
    format!(
        "cannot compute '{} {} {}'",
        lhs.data_type().to_str(),
        op.symbol(),
        rhs.data_type().to_str()
    )
}

#[cfg(test)]
mod _repro_clamp_test {
    use crate::value::ScalarValue;
    use yggdryl_core::{Duration, TimeUnit};

    #[test]
    fn repro_timestamp_add_clamps() {
        let ts = ScalarValue::timestamp(i64::MAX / 2, TimeUnit::Second, None);
        let r = ts
            .add(&ScalarValue::from_duration(&Duration::from_secs(0)))
            .unwrap();
        eprintln!("ORIGINAL: {:?}", ts);
        eprintln!("RESULT:   {:?}", r);
        match r {
            ScalarValue::Timestamp { value, unit, .. } => {
                eprintln!("value={} unit={:?}", value, unit);
                // original instant in nanos:
                let orig_nanos = (i64::MAX / 2) as i128 * 1_000_000_000i128;
                let res_nanos = value as i128 * unit.nanos() as i128;
                eprintln!(
                    "orig_nanos={} res_nanos={} equal={}",
                    orig_nanos,
                    res_nanos,
                    orig_nanos == res_nanos
                );
            }
            other => panic!("not a timestamp: {:?}", other),
        }
    }
}
