//! Row evaluation: one [`Value`] at a time, in SQL three-valued logic.
//!
//! A row is one [`Value::Record`], one [`Value::Sequence`] in field order, or
//! one [`Value::Mapping`] by name - all three, because all three are what this
//! project calls a row. Reading a column costs a slot index and a borrow: no
//! name lookup, no allocation, and no intermediate vector per row, because the
//! names were resolved once when the plan was bound.
//!
//! # Three-valued logic
//!
//! A comparison with a null operand is *unknown*, not false. `matches` keeps a
//! row only on `true`, `AND`/`OR` follow the standard tables, and `IS NULL` is
//! the only way to select absence. `venue <> 'XNAS'` therefore leaves the rows
//! whose venue is null behind.

use smol_str::{SmolStr, format_smolstr};

use super::bound::{Bound, BoundColumn, BoundPredicate, Step, coerce_value, compare};
use super::graph::{Node, NodeId, Plan};
use super::{ArithOp, Function};
use crate::{Error, Result, TimeUnit, Value};

impl Bound {
    /// Evaluate this plan over one row.
    ///
    /// # Errors
    ///
    /// Returns an error when arithmetic overflows, when a strict cast meets a
    /// value the target cannot hold, or when the row disagrees with the schema
    /// this plan was bound to.
    pub fn evaluate(&self, row: &Value) -> Result<Value> {
        evaluate(self.plan(), self.root(), row)
    }
}

impl BoundPredicate {
    /// Return whether one row matches.
    ///
    /// Only `true` keeps a row: unknown - which is what a comparison against a
    /// null answers - does not.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Bound::evaluate`] returns.
    pub fn matches(&self, row: &Value) -> Result<bool> {
        Ok(matches!(self.bound().evaluate(row)?, Value::Bool(true)))
    }
}

/// Evaluate one node over one row.
pub(super) fn evaluate(plan: &Plan, id: NodeId, row: &Value) -> Result<Value> {
    let Some(node) = plan.get(id) else {
        return Ok(Value::Null);
    };
    match node {
        Node::Literal(value) => Ok(value.clone()),
        Node::Column(column) => match column.bound() {
            Some(bound) => Ok(read_column(row, bound)),
            // A column the tolerant binding could not resolve reads as
            // absence, which makes every comparison on it unknown.
            None => Ok(Value::Null),
        },
        Node::Alias { child, .. } => evaluate(plan, *child, row),
        Node::Cast {
            child,
            data_type,
            safe,
        } => {
            let value = evaluate(plan, *child, row)?;
            match coerce_value(&value, data_type) {
                Some(converted) => Ok(converted),
                None if *safe => Ok(Value::Null),
                None => Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        format_smolstr!("a value a {data_type} can hold"),
                        crate::text::elide_display(&super::Literal(&value)),
                    ),
                }),
            }
        }
        Node::Compare { op, left, right } => {
            let left = evaluate(plan, *left, row)?;
            let right = evaluate(plan, *right, row)?;
            Ok(unknown_or(compare(*op, &left, &right)))
        }
        Node::And(operands) => {
            // Three-valued conjunction: one false settles it, and an unknown
            // survives only until a false appears.
            let mut certain = true;
            for operand in operands {
                match evaluate(plan, *operand, row)? {
                    Value::Bool(false) => return Ok(Value::Bool(false)),
                    Value::Bool(true) => {}
                    _ => certain = false,
                }
            }
            Ok(if certain {
                Value::Bool(true)
            } else {
                Value::Null
            })
        }
        Node::Or(operands) => {
            let mut certain = true;
            for operand in operands {
                match evaluate(plan, *operand, row)? {
                    Value::Bool(true) => return Ok(Value::Bool(true)),
                    Value::Bool(false) => {}
                    _ => certain = false,
                }
            }
            Ok(if certain {
                Value::Bool(false)
            } else {
                Value::Null
            })
        }
        Node::Not(child) => Ok(match evaluate(plan, *child, row)? {
            Value::Bool(known) => Value::Bool(!known),
            _ => Value::Null,
        }),
        Node::IsNull(child) => Ok(Value::Bool(evaluate(plan, *child, row)?.is_null())),
        Node::IsNotNull(child) => Ok(Value::Bool(!evaluate(plan, *child, row)?.is_null())),
        Node::In {
            child,
            list,
            negated,
        } => {
            let value = evaluate(plan, *child, row)?;
            if value.is_null() {
                return Ok(Value::Null);
            }
            // SQL's `IN` is a disjunction of equalities, so a null in the list
            // makes a non-match unknown rather than false.
            let mut certain = true;
            for item in list {
                let item = evaluate(plan, *item, row)?;
                match compare(super::CompareOp::Eq, &value, &item) {
                    Some(true) => return Ok(Value::Bool(!*negated)),
                    Some(false) => {}
                    None => certain = false,
                }
            }
            Ok(if certain {
                Value::Bool(*negated)
            } else {
                Value::Null
            })
        }
        Node::Between {
            child,
            low,
            high,
            negated,
        } => {
            let value = evaluate(plan, *child, row)?;
            let low = evaluate(plan, *low, row)?;
            let high = evaluate(plan, *high, row)?;
            let inside = match (
                compare(super::CompareOp::GtEq, &value, &low),
                compare(super::CompareOp::LtEq, &value, &high),
            ) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            };
            Ok(unknown_or(inside.map(|inside| inside != *negated)))
        }
        Node::Like {
            child,
            pattern,
            escape,
            negated,
            case_insensitive,
        } => {
            let value = evaluate(plan, *child, row)?;
            let pattern = evaluate(plan, *pattern, row)?;
            let (Some(text), Some(pattern)) = (value.as_str(), pattern.as_str()) else {
                return Ok(Value::Null);
            };
            let matched = like_matches(text, pattern, *escape, *case_insensitive);
            Ok(Value::Bool(matched != *negated))
        }
        Node::StartsWith { child, prefix } => {
            let value = evaluate(plan, *child, row)?;
            Ok(value.as_str().map_or(Value::Null, |text| {
                Value::Bool(text.starts_with(prefix.as_str()))
            }))
        }
        Node::Arithmetic { op, left, right } => {
            let left = evaluate(plan, *left, row)?;
            let right = evaluate(plan, *right, row)?;
            arithmetic(*op, &left, &right)
        }
        Node::Neg(child) => {
            let value = evaluate(plan, *child, row)?;
            arithmetic(ArithOp::Sub, &Value::I64(0), &value)
        }
        Node::Function { name, args } => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(evaluate(plan, *arg, row)?);
            }
            call(*name, &values)
        }
        Node::Case {
            branches,
            otherwise,
        } => {
            for (when, then) in branches {
                if matches!(evaluate(plan, *when, row)?, Value::Bool(true)) {
                    return evaluate(plan, *then, row);
                }
            }
            match otherwise {
                Some(otherwise) => evaluate(plan, *otherwise, row),
                None => Ok(Value::Null),
            }
        }
    }
}

/// Read one bound column out of a row, following its resolved slot chain.
pub(super) fn read_column(row: &Value, column: &BoundColumn) -> Value {
    let mut held = match row {
        // A record and a sequence are both positional, which is why the
        // ordinal resolved at bind time is all a read costs.
        Value::Record(_, values) | Value::Sequence(values) => {
            values.get(column.root_index()).cloned()
        }
        // A mapping is by name, which is the shape a decoded document has.
        Value::Mapping(_) => row.get_key_str(column.name()).cloned().or_else(|| {
            row.entries()
                .find(|(key, _)| {
                    key.as_str()
                        .is_some_and(|key| key.eq_ignore_ascii_case(column.name()))
                })
                .map(|(_, value)| value.clone())
        }),
        _ => None,
    }
    .unwrap_or(Value::Null);
    for step in column.steps() {
        held = apply_step(&held, step);
        if held.is_null() {
            // Absence propagates: nothing below a missing value exists either.
            return Value::Null;
        }
    }
    held
}

/// Apply one resolved accessor to one value.
///
/// Out of range is null and an out-of-range range clamps, never an error:
/// absence is not a failure on the read path anywhere else in this project,
/// and a predicate over a ragged list must not abort a scan.
fn apply_step(value: &Value, step: &Step) -> Value {
    match step {
        Step::Child { index, name } => match value {
            Value::Record(_, values) | Value::Sequence(values) => {
                values.get(*index).cloned().unwrap_or(Value::Null)
            }
            Value::Mapping(_) => value.get_key_str(name).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        },
        Step::Key(key) => match value {
            Value::Mapping(_) => value.get_key(key).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        },
        Step::Index(index) => match value {
            Value::Sequence(items) => resolve_index(*index, items.len())
                .and_then(|index| items.get(index).cloned())
                .unwrap_or(Value::Null),
            Value::String(text) => {
                // Text slices Unicode scalar values, so an index never splits
                // a character; binary below slices bytes.
                let characters: Vec<char> = text.chars().collect();
                resolve_index(*index, characters.len())
                    .and_then(|index| characters.get(index))
                    .map_or(Value::Null, |character| {
                        Value::String(SmolStr::new(character.to_string()))
                    })
            }
            Value::Bytes(bytes) => resolve_index(*index, bytes.len())
                .and_then(|index| bytes.get(index))
                .map_or(Value::Null, |byte| {
                    Value::Bytes(std::sync::Arc::from([*byte].as_slice()))
                }),
            _ => Value::Null,
        },
        Step::Range { start, end } => match value {
            Value::Sequence(items) => {
                let (from, to) = resolve_range(*start, *end, items.len());
                Value::from_sequence(items[from..to].iter().cloned())
            }
            Value::String(text) => {
                let characters: Vec<char> = text.chars().collect();
                let (from, to) = resolve_range(*start, *end, characters.len());
                Value::String(SmolStr::new(
                    characters[from..to].iter().collect::<String>(),
                ))
            }
            Value::Bytes(bytes) => {
                let (from, to) = resolve_range(*start, *end, bytes.len());
                Value::Bytes(std::sync::Arc::from(&bytes[from..to]))
            }
            _ => Value::Null,
        },
    }
}

/// Turn a possibly negative index into a position inside a value.
fn resolve_index(index: i64, length: usize) -> Option<usize> {
    let length = i64::try_from(length).ok()?;
    let resolved = if index < 0 { length + index } else { index };
    (resolved >= 0 && resolved < length).then(|| usize::try_from(resolved).unwrap_or(0))
}

/// Turn a half-open range into a clamped pair of positions.
fn resolve_range(start: Option<i64>, end: Option<i64>, length: usize) -> (usize, usize) {
    let span = i64::try_from(length).unwrap_or(i64::MAX);
    let clamp = |bound: i64| {
        let resolved = if bound < 0 { span + bound } else { bound };
        usize::try_from(resolved.clamp(0, span)).unwrap_or(0)
    };
    let from = start.map_or(0, clamp);
    let to = end.map_or(length, clamp);
    // An inverted range is empty rather than an error, for the same reason an
    // out-of-range index is null.
    (from, to.max(from))
}

/// Read a three-valued answer back as a value.
const fn unknown_or(answer: Option<bool>) -> Value {
    match answer {
        Some(known) => Value::Bool(known),
        None => Value::Null,
    }
}

/// Evaluate one arithmetic node, refusing an overflow rather than wrapping.
fn arithmetic(op: ArithOp, left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    // An elapsed count is what a difference of two instants is, and it is the
    // one pairing whose result leaves the family its operands are in.
    if op == ArithOp::Sub {
        if let (Some(left), Some(right)) = (millis_of(left), millis_of(right)) {
            return left
                .checked_sub(right)
                .map(|count| Value::Duration(count, TimeUnit::Millisecond))
                .ok_or_else(|| overflow(op));
        }
    }
    if let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) {
        return integer_arithmetic(op, left, right);
    }
    if let (
        Value::Decimal(..) | Value::I8(..) | Value::I16(..) | Value::I32(..) | Value::I64(..),
        _,
    )
    | (_, Value::Decimal(..)) = (left, right)
    {
        if let (Some(left), Some(right)) = (as_decimal_parts(left), as_decimal_parts(right)) {
            return decimal_arithmetic(op, left, right);
        }
    }
    let (Some(left), Some(right)) = (as_float(left), as_float(right)) else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("expected two numbers to apply {op} to, got a non-number"),
        });
    };
    Ok(Value::from(match op {
        ArithOp::Add => left + right,
        ArithOp::Sub => left - right,
        ArithOp::Mul => left * right,
        ArithOp::Div => left / right,
        ArithOp::Mod => left % right,
    }))
}

/// The exact integer arithmetic path, checked at every step.
fn integer_arithmetic(op: ArithOp, left: i128, right: i128) -> Result<Value> {
    let computed = match op {
        ArithOp::Add => left.checked_add(right),
        ArithOp::Sub => left.checked_sub(right),
        ArithOp::Mul => left.checked_mul(right),
        // A division of two integers is not an integer in general, so it
        // answers as a double, exactly as the bound type says it does.
        #[allow(clippy::cast_precision_loss)]
        ArithOp::Div => return Ok(Value::from(left as f64 / right as f64)),
        ArithOp::Mod => left.checked_rem(right),
    };
    let computed = computed.ok_or_else(|| overflow(op))?;
    Ok(match i64::try_from(computed) {
        Ok(narrow) => Value::I64(narrow),
        Err(_) => Value::I128(computed),
    })
}

/// The exact decimal arithmetic path: coefficients, never a float.
fn decimal_arithmetic(op: ArithOp, left: (i128, i8), right: (i128, i8)) -> Result<Value> {
    let (left_unscaled, left_scale) = left;
    let (right_unscaled, right_scale) = right;
    match op {
        ArithOp::Add | ArithOp::Sub | ArithOp::Mod => {
            let scale = left_scale.max(right_scale);
            let left = rescale(left_unscaled, left_scale, scale).ok_or_else(|| overflow(op))?;
            let right = rescale(right_unscaled, right_scale, scale).ok_or_else(|| overflow(op))?;
            let computed = match op {
                ArithOp::Add => left.checked_add(right),
                ArithOp::Sub => left.checked_sub(right),
                _ => left.checked_rem(right),
            }
            .ok_or_else(|| overflow(op))?;
            Ok(Value::Decimal(computed, scale))
        }
        ArithOp::Mul => {
            let scale = left_scale
                .checked_add(right_scale)
                .ok_or_else(|| overflow(op))?;
            let computed = left_unscaled
                .checked_mul(right_unscaled)
                .ok_or_else(|| overflow(op))?;
            Ok(Value::Decimal(computed, scale))
        }
        #[allow(clippy::cast_precision_loss)]
        ArithOp::Div => {
            let left = left_unscaled as f64 / 10_f64.powi(i32::from(left_scale));
            let right = right_unscaled as f64 / 10_f64.powi(i32::from(right_scale));
            Ok(Value::from(left / right))
        }
    }
}

/// Restate an unscaled coefficient at a wider scale, exactly.
fn rescale(unscaled: i128, from: i8, to: i8) -> Option<i128> {
    let steps = i32::from(to) - i32::from(from);
    if steps == 0 {
        return Some(unscaled);
    }
    if steps < 0 {
        return None;
    }
    unscaled.checked_mul(10_i128.checked_pow(u32::try_from(steps).ok()?)?)
}

/// Read a value as a decimal coefficient and scale.
const fn as_decimal_parts(value: &Value) -> Option<(i128, i8)> {
    match value {
        Value::Decimal(unscaled, scale) => Some((*unscaled, *scale)),
        Value::I8(inner) => Some((*inner as i128, 0)),
        Value::I16(inner) => Some((*inner as i128, 0)),
        Value::I32(inner) => Some((*inner as i128, 0)),
        Value::I64(inner) => Some((*inner as i128, 0)),
        Value::I128(inner) => Some((*inner, 0)),
        _ => None,
    }
}

/// Read a value as an `f64`, for the inexact arithmetic path.
fn as_float(value: &Value) -> Option<f64> {
    match value {
        Value::F32(_) | Value::F64(_) => value.as_f64(),
        #[allow(clippy::cast_precision_loss)]
        Value::Decimal(unscaled, scale) => Some(*unscaled as f64 / 10_f64.powi(i32::from(*scale))),
        #[allow(clippy::cast_precision_loss)]
        other => other.as_i128().map(|integer| integer as f64),
    }
}

/// The millisecond count an instant-like value names.
fn millis_of(value: &Value) -> Option<i64> {
    match value {
        Value::Date(days) => i64::from(*days).checked_mul(86_400_000),
        Value::Time(..) | Value::Timestamp(..) | Value::DateTime(..) => {
            value.temporal_count_at(TimeUnit::Millisecond)
        }
        _ => None,
    }
}

/// The failure an overflowing computation reports.
fn overflow(op: ArithOp) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("{op} overflowed the widest exact number this value model holds"),
    }
}

/// Evaluate one call from the closed function vocabulary.
fn call(name: Function, args: &[Value]) -> Result<Value> {
    let first = args.first().cloned().unwrap_or(Value::Null);
    if name == Function::Coalesce {
        return Ok(args
            .iter()
            .find(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null));
    }
    if first.is_null() {
        return Ok(Value::Null);
    }
    Ok(match name {
        Function::Coalesce => first,
        Function::Length => match &first {
            // Text counts characters and binary counts bytes, which is the
            // same split the range accessor makes.
            Value::String(text) => {
                Value::I64(i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
            }
            Value::Bytes(bytes) => Value::I64(i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
            other => Value::I64(i64::try_from(other.len()).unwrap_or(i64::MAX)),
        },
        Function::Lower => text_of(&first).map_or(Value::Null, |text| {
            Value::String(SmolStr::new(text.to_lowercase()))
        }),
        Function::Upper => text_of(&first).map_or(Value::Null, |text| {
            Value::String(SmolStr::new(text.to_uppercase()))
        }),
        Function::Trim => {
            text_of(&first).map_or(Value::Null, |text| Value::String(SmolStr::new(text.trim())))
        }
        Function::Substring => substring(&first, args),
        Function::Abs => absolute(&first)?,
        Function::Truncate => truncate(&first, args.get(1))?,
        calendar => calendar_field(calendar, &first),
    })
}

/// The text of a value, for the string functions.
fn text_of(value: &Value) -> Option<&str> {
    value.as_str()
}

/// SQL `substring`, which is 1-based - deliberately unlike the `[]` accessor.
fn substring(value: &Value, args: &[Value]) -> Value {
    let Some(text) = value.as_str() else {
        return Value::Null;
    };
    let characters: Vec<char> = text.chars().collect();
    let Some(start) = args.get(1).and_then(Value::as_i128) else {
        return Value::Null;
    };
    // SQL counts from one, and a start below one is clamped rather than
    // wrapped - the standard's own behavior.
    let from = usize::try_from((start - 1).max(0))
        .unwrap_or(0)
        .min(characters.len());
    let to = match args.get(2).and_then(Value::as_i128) {
        Some(length) => usize::try_from(i128::try_from(from).unwrap_or(0) + length.max(0))
            .unwrap_or(0)
            .min(characters.len()),
        None => characters.len(),
    };
    Value::String(SmolStr::new(
        characters[from..to.max(from)].iter().collect::<String>(),
    ))
}

/// The absolute value, exact where the value is exact.
fn absolute(value: &Value) -> Result<Value> {
    Ok(match value {
        Value::Decimal(unscaled, scale) => Value::Decimal(
            unscaled
                .checked_abs()
                .ok_or_else(|| overflow(ArithOp::Sub))?,
            *scale,
        ),
        Value::F32(_) | Value::F64(_) => Value::from(value.as_f64().unwrap_or(f64::NAN).abs()),
        other => match other.as_i128() {
            Some(integer) => {
                let absolute = integer
                    .checked_abs()
                    .ok_or_else(|| overflow(ArithOp::Sub))?;
                match i64::try_from(absolute) {
                    Ok(narrow) => Value::I64(narrow),
                    Err(_) => Value::I128(absolute),
                }
            }
            None => Value::Null,
        },
    })
}

/// The largest multiple of `width` at or below `value`.
fn truncate(value: &Value, width: Option<&Value>) -> Result<Value> {
    let Some(width) = width.and_then(Value::as_i128) else {
        return Ok(Value::Null);
    };
    if width <= 0 {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got("a positive truncation width", width),
        });
    }
    Ok(match value {
        // A decimal truncates on its own coefficient, so the scale survives.
        Value::Decimal(unscaled, scale) => {
            Value::Decimal(unscaled - unscaled.rem_euclid(width), *scale)
        }
        other => match other.as_i128() {
            Some(integer) => {
                let truncated = integer - integer.rem_euclid(width);
                match i64::try_from(truncated) {
                    Ok(narrow) => Value::I64(narrow),
                    Err(_) => Value::I128(truncated),
                }
            }
            None => Value::Null,
        },
    })
}

/// Read one calendar field off a temporal value.
///
/// This is SQL's meaning - `year(DATE '2024-03-01')` is 2024 - and
/// deliberately not Iceberg's `years` transform, which counts from 1970. A
/// zoned instant is read in its own zone, which is the only reading that makes
/// "the day this happened" answerable.
fn calendar_field(field: Function, value: &Value) -> Value {
    let (days, seconds_of_day) = match value {
        Value::Date(days) => (i64::from(*days), 0),
        Value::Time(count, unit) => {
            let seconds = Value::Duration(*count, *unit)
                .temporal_count_at(TimeUnit::Second)
                .unwrap_or(0);
            (0, seconds)
        }
        Value::Timestamp(count, unit, zone) => {
            let utc = Value::Duration(*count, *unit)
                .temporal_count_at(TimeUnit::Second)
                .unwrap_or(0);
            let local = zone.to_local(utc).unwrap_or(utc);
            (local.div_euclid(86_400), local.rem_euclid(86_400))
        }
        Value::DateTime(count, unit) => {
            let seconds = Value::Duration(*count, *unit)
                .temporal_count_at(TimeUnit::Second)
                .unwrap_or(0);
            (seconds.div_euclid(86_400), seconds.rem_euclid(86_400))
        }
        _ => return Value::Null,
    };
    let (year, month, day) = crate::enums::timezone::civil_from_days(days);
    let counted = match field {
        Function::Year => i64::from(year),
        Function::Month => i64::from(month),
        Function::Day => i64::from(day),
        Function::Hour => seconds_of_day / 3_600,
        Function::Minute => (seconds_of_day / 60) % 60,
        Function::Second => seconds_of_day % 60,
        _ => return Value::Null,
    };
    Value::I32(i32::try_from(counted).unwrap_or(i32::MAX))
}

/// Match SQL's `LIKE`, with `%` for any run and `_` for one character.
///
/// The matcher is iterative with one backtrack point, which is the standard
/// linear-time wildcard algorithm: it never recurses, so a pathological
/// pattern costs time rather than stack.
pub(super) fn like_matches(
    text: &str,
    pattern: &str,
    escape: Option<char>,
    case_insensitive: bool,
) -> bool {
    let fold = |character: char| {
        if case_insensitive {
            character.to_ascii_lowercase()
        } else {
            character
        }
    };
    let text: Vec<char> = text.chars().map(fold).collect();
    // The escape character is consumed while the pattern is read, so a
    // literal `%` and a wildcard `%` become different tokens here.
    let mut tokens: Vec<(char, bool)> = Vec::with_capacity(pattern.chars().count());
    let mut escaped = false;
    for character in pattern.chars() {
        if escaped {
            tokens.push((fold(character), true));
            escaped = false;
            continue;
        }
        if Some(character) == escape {
            escaped = true;
            continue;
        }
        tokens.push((fold(character), false));
    }
    let mut text_at = 0;
    let mut pattern_at = 0;
    let mut star_at: Option<usize> = None;
    let mut star_text = 0;
    while text_at < text.len() {
        match tokens.get(pattern_at) {
            Some(('%', false)) => {
                star_at = Some(pattern_at);
                star_text = text_at;
                pattern_at += 1;
            }
            Some(('_', false)) => {
                text_at += 1;
                pattern_at += 1;
            }
            Some((expected, _)) if *expected == text[text_at] => {
                text_at += 1;
                pattern_at += 1;
            }
            _ => match star_at {
                Some(star) => {
                    pattern_at = star + 1;
                    star_text += 1;
                    text_at = star_text;
                }
                None => return false,
            },
        }
    }
    tokens[pattern_at..]
        .iter()
        .all(|(character, literal)| *character == '%' && !*literal)
}

/// Apply a run of resolved accessor steps to one value.
///
/// Only the vectorized path calls this, so a schema-only build does not carry
/// it - the row path walks the steps as it reads the column.
///
/// The vectorized path calls this for the accessors that have no columnar
/// kernel, so both paths reach inside a value through exactly one traversal.
#[cfg(feature = "arrow")]
pub(super) fn apply_steps(value: &Value, steps: &[Step]) -> Value {
    let mut held = value.clone();
    for step in steps {
        held = apply_step(&held, step);
        if held.is_null() {
            return Value::Null;
        }
    }
    held
}

/// Evaluate one node whose children are already materialized as values.
///
/// This is the row-at-a-time fallback the vectorized evaluator uses for the
/// nodes with no kernel: the children were read once into columns of values,
/// so the node itself is the only thing paying per row.
///
/// # Errors
///
/// Returns whatever the equivalent [`evaluate`] step returns.
#[cfg(feature = "arrow")]
pub(super) fn evaluate_with(
    plan: &Plan,
    id: NodeId,
    child: &dyn Fn(NodeId) -> Value,
) -> Result<Value> {
    let Some(node) = plan.get(id) else {
        return Ok(Value::Null);
    };
    Ok(match node {
        Node::Literal(value) => value.clone(),
        Node::Column(_) => child(id),
        Node::Alias { child: inner, .. } | Node::Cast { child: inner, .. } => {
            let value = child(*inner);
            match node {
                Node::Cast {
                    data_type, safe, ..
                } => match coerce_value(&value, data_type) {
                    Some(converted) => converted,
                    None if *safe => Value::Null,
                    None => {
                        return Err(Error::InvalidRecord {
                            path: SmolStr::new_static("$"),
                            reason: crate::text::expected_got(
                                format_smolstr!("a value a {data_type} can hold"),
                                crate::text::elide_display(&super::Literal(&value)),
                            ),
                        });
                    }
                },
                _ => value,
            }
        }
        Node::Compare { op, left, right } => {
            unknown_or(compare(*op, &child(*left), &child(*right)))
        }
        Node::And(operands) => {
            let mut certain = true;
            let mut answer = Value::Bool(true);
            for operand in operands {
                match child(*operand) {
                    Value::Bool(false) => {
                        answer = Value::Bool(false);
                        certain = true;
                        break;
                    }
                    Value::Bool(true) => {}
                    _ => certain = false,
                }
            }
            if certain { answer } else { Value::Null }
        }
        Node::Or(operands) => {
            let mut certain = true;
            let mut answer = Value::Bool(false);
            for operand in operands {
                match child(*operand) {
                    Value::Bool(true) => {
                        answer = Value::Bool(true);
                        certain = true;
                        break;
                    }
                    Value::Bool(false) => {}
                    _ => certain = false,
                }
            }
            if certain { answer } else { Value::Null }
        }
        Node::Not(inner) => match child(*inner) {
            Value::Bool(known) => Value::Bool(!known),
            _ => Value::Null,
        },
        Node::IsNull(inner) => Value::Bool(child(*inner).is_null()),
        Node::IsNotNull(inner) => Value::Bool(!child(*inner).is_null()),
        Node::In {
            child: inner,
            list,
            negated,
        } => {
            let value = child(*inner);
            if value.is_null() {
                return Ok(Value::Null);
            }
            let mut certain = true;
            let mut found = false;
            for item in list {
                match compare(super::CompareOp::Eq, &value, &child(*item)) {
                    Some(true) => {
                        found = true;
                        break;
                    }
                    Some(false) => {}
                    None => certain = false,
                }
            }
            if found {
                Value::Bool(!*negated)
            } else if certain {
                Value::Bool(*negated)
            } else {
                Value::Null
            }
        }
        Node::Between {
            child: inner,
            low,
            high,
            negated,
        } => {
            let value = child(*inner);
            let inside = match (
                compare(super::CompareOp::GtEq, &value, &child(*low)),
                compare(super::CompareOp::LtEq, &value, &child(*high)),
            ) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            };
            unknown_or(inside.map(|inside| inside != *negated))
        }
        Node::Like {
            child: inner,
            pattern,
            escape,
            negated,
            case_insensitive,
        } => {
            let value = child(*inner);
            let pattern = child(*pattern);
            match (value.as_str(), pattern.as_str()) {
                (Some(text), Some(pattern)) => {
                    Value::Bool(like_matches(text, pattern, *escape, *case_insensitive) != *negated)
                }
                _ => Value::Null,
            }
        }
        Node::StartsWith {
            child: inner,
            prefix,
        } => child(*inner).as_str().map_or(Value::Null, |text| {
            Value::Bool(text.starts_with(prefix.as_str()))
        }),
        Node::Arithmetic { op, left, right } => arithmetic(*op, &child(*left), &child(*right))?,
        Node::Neg(inner) => arithmetic(ArithOp::Sub, &Value::I64(0), &child(*inner))?,
        Node::Function { name, args } => {
            let values: Vec<Value> = args.iter().map(|arg| child(*arg)).collect();
            call(*name, &values)?
        }
        Node::Case {
            branches,
            otherwise,
        } => {
            let mut answer = otherwise.map_or(Value::Null, child);
            for (when, then) in branches {
                if matches!(child(*when), Value::Bool(true)) {
                    answer = child(*then);
                    break;
                }
            }
            answer
        }
    })
}
