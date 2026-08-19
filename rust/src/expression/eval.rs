//! Scalar evaluation: one resolved tree, one row at a time, over [`Value`].
//!
//! This tier compiles with no Arrow at all. It is the definition of what every
//! operator *means*, and the vectorized tier in [`arrow`](super::arrow) is an
//! optimization of it - which is why the property test asserts the two agree
//! rather than testing them separately.
//!
//! # Three-valued logic
//!
//! Null is unknown, not false. `and` is false when any operand is false even
//! if another is unknown; `or` is true when any operand is true even if
//! another is unknown; `not unknown` is unknown. A comparison with a null
//! operand is unknown, and the two distinctness tests are the only comparisons
//! that answer about a null rather than through it. A row is kept when the
//! predicate is *true*, so unknown filters the row out - and that is a
//! separate decision from what the predicate evaluated to.
//!
//! # Floats
//!
//! Comparison uses IEEE 754 totalOrder, which is what Arrow's kernels use:
//! `nan` equals `nan`, `-nan` sorts below everything, and `nan` sorts above
//! it. Two tiers that disagree about `nan` disagree about a filter, so this is
//! stated rather than inherited.

use std::cmp::Ordering;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::bind::{Kind, Node};
use super::selector::Attributes;
use super::typing::{
    decimal_parts, is_binary, is_text, step_field, temporal_parts, unwrap_dictionary,
};
use super::{Comparison, Function, Operator, Safety, Segment};
use crate::{DataType, Error, Field, Float, Float32, Result, TimeUnit, Value};

/// One row's worth of context: its column values and its holder.
///
/// Either half may be absent. A constant subtree needs neither, which is what
/// lets [`bind`](super::bind) fold by evaluating; a listing filter has a
/// holder and no row, which is what lets it prune before opening anything.
pub(crate) struct Row<'context> {
    values: Option<&'context [Value]>,
    holder: Option<&'context dyn Attributes>,
}

impl<'context> Row<'context> {
    pub(crate) const fn new(
        values: Option<&'context [Value]>,
        holder: Option<&'context dyn Attributes>,
    ) -> Self {
        Self { values, holder }
    }
}

fn missing(what: &str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("expected {what}"),
    }
}

impl Node {
    /// Evaluate this node for one row.
    ///
    /// # Errors
    ///
    /// Returns an error when a strict cast refuses a value, when a column is
    /// asked for and no row was supplied, or when a holder attribute fails.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn eval(&self, row: &Row<'_>) -> Result<Value> {
        match &self.kind {
            Kind::Literal(value) => Ok(value.clone()),
            Kind::Column(index) => {
                let values = row
                    .values
                    .ok_or_else(|| missing("a row to read a column from"))?;
                values
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| missing("a row with every bound column"))
            }
            Kind::Path(base, steps) => {
                let mut field = base.field.clone();
                let mut value = base.eval(row)?;
                for step in steps.iter() {
                    let next = step_field(&field, step)?;
                    value = apply_step(&field, &value, step);
                    field = next;
                    if value.is_null() {
                        break;
                    }
                }
                Ok(value)
            }
            Kind::Attribute(selector) => match row.holder {
                Some(holder) => holder.attribute(selector),
                None => Ok(Value::Null),
            },
            Kind::And(operands) => {
                let mut unknown = false;
                for operand in operands {
                    match operand.eval(row)?.as_bool() {
                        Some(false) => return Ok(Value::Bool(false)),
                        Some(true) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    Value::Null
                } else {
                    Value::Bool(true)
                })
            }
            Kind::Or(operands) => {
                let mut unknown = false;
                for operand in operands {
                    match operand.eval(row)?.as_bool() {
                        Some(true) => return Ok(Value::Bool(true)),
                        Some(false) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    Value::Null
                } else {
                    Value::Bool(false)
                })
            }
            Kind::Not(inner) => Ok(match inner.eval(row)?.as_bool() {
                Some(held) => Value::Bool(!held),
                None => Value::Null,
            }),
            Kind::Compare(left, comparison, right) => {
                let left_value = left.eval(row)?;
                let right_value = right.eval(row)?;
                Ok(compare(
                    left.field.data_type(),
                    &left_value,
                    *comparison,
                    &right_value,
                ))
            }
            Kind::In(value, list) => {
                let held = value.eval(row)?;
                if held.is_null() {
                    return Ok(Value::Null);
                }
                let mut unknown = false;
                for item in list {
                    let item_value = item.eval(row)?;
                    match compare(value.field.data_type(), &held, Comparison::Eq, &item_value)
                        .as_bool()
                    {
                        Some(true) => return Ok(Value::Bool(true)),
                        Some(false) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    Value::Null
                } else {
                    Value::Bool(false)
                })
            }
            Kind::Between(value, low, high) => {
                let data_type = value.field.data_type();
                let held = value.eval(row)?;
                let above = compare(data_type, &held, Comparison::GtEq, &low.eval(row)?);
                let below = compare(data_type, &held, Comparison::LtEq, &high.eval(row)?);
                Ok(kleene_and(&above, &below))
            }
            Kind::IsNull(inner) => Ok(Value::Bool(inner.eval(row)?.is_null())),
            Kind::IsNotNull(inner) => Ok(Value::Bool(!inner.eval(row)?.is_null())),
            Kind::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => {
                let held = value.eval(row)?;
                let Some(text) = held.as_str() else {
                    return Ok(Value::Null);
                };
                Ok(Value::Bool(like_matches(
                    text,
                    pattern,
                    *case_insensitive,
                    *escape,
                )))
            }
            Kind::Glob(value, pattern) => {
                let held = value.eval(row)?;
                let Some(text) = held.as_str() else {
                    return Ok(Value::Null);
                };
                Ok(Value::Bool(crate::uri::pattern::matches_glob_text(
                    text, pattern,
                )))
            }
            Kind::Arithmetic(left, operator, right) => arithmetic(
                self.field.data_type(),
                &left.eval(row)?,
                *operator,
                &right.eval(row)?,
            ),
            Kind::Negate(inner) => {
                let held = inner.eval(row)?;
                Ok(super::negate_value(&held).unwrap_or(Value::Null))
            }
            Kind::Function(function, arguments) => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(argument.eval(row)?);
                }
                call(*function, arguments, &values, self.field.data_type())
            }
            Kind::Cast(inner, safety) => {
                let held = inner.eval(row)?;
                match convert(self.field.data_type(), &held, *safety) {
                    Ok(value) => Ok(value),
                    Err(error) if matches!(safety, Safety::Safe) => {
                        let _ = error;
                        Ok(Value::Null)
                    }
                    Err(error) => Err(error),
                }
            }
            Kind::Case {
                branches,
                otherwise,
            } => {
                for (when, then) in branches {
                    if when.eval(row)?.as_bool() == Some(true) {
                        return then.eval(row);
                    }
                }
                match otherwise {
                    Some(otherwise) => otherwise.eval(row),
                    None => Ok(Value::Null),
                }
            }
            Kind::Struct(children) => {
                let mut values = Vec::with_capacity(children.len());
                for child in children {
                    values.push(child.eval(row)?);
                }
                Value::record(self.field.data_type().clone(), values)
            }
            Kind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(item.eval(row)?);
                }
                Ok(Value::from_sequence(values))
            }
            Kind::Map(entries) => {
                let mut pairs = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    pairs.push((key.eval(row)?, value.eval(row)?));
                }
                Value::from_mapping(pairs)
            }
        }
    }
}

/// Reach one step into a value, answering null for anything absent.
fn apply_step(field: &Field, value: &Value, segment: &Segment) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    match segment {
        Segment::Field(name) => struct_child(field, value, name),
        Segment::Index(position) => {
            let Some(items) = value.as_sequence() else {
                return Value::Null;
            };
            let length = i64::try_from(items.len()).unwrap_or(i64::MAX);
            // A negative index counts from the end, and either end may miss.
            let resolved = if *position < 0 {
                length + position
            } else {
                *position
            };
            usize::try_from(resolved)
                .ok()
                .and_then(|index| items.get(index))
                .cloned()
                .unwrap_or(Value::Null)
        }
        Segment::Key(key) => {
            if let Some(entries) = value.as_mapping() {
                let data_type = key.data_type();
                return entries
                    .iter()
                    .find(|(held, _)| {
                        compare(data_type, held, Comparison::IsNotDistinctFrom, key.value())
                            .as_bool()
                            == Some(true)
                    })
                    .map_or(Value::Null, |(_, held)| held.clone());
            }
            match key.value().as_str() {
                Some(name) => struct_child(field, value, name),
                None => Value::Null,
            }
        }
    }
}

/// Read one struct child, whichever way the row spells a struct.
fn struct_child(field: &Field, value: &Value, name: &str) -> Value {
    if let Value::Record(data_type, values) = value {
        return data_type
            .as_fields()
            .and_then(|fields| {
                fields
                    .iter()
                    .position(|child| child.name().eq_ignore_ascii_case(name))
            })
            .and_then(|index| values.get(index))
            .cloned()
            .unwrap_or(Value::Null);
    }
    if let Some(entries) = value.as_mapping() {
        return entries
            .iter()
            .find(|(key, _)| {
                key.as_str()
                    .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map_or(Value::Null, |(_, held)| held.clone());
    }
    // A struct spelled as a bare sequence takes its order from the schema.
    if let (Some(values), DataType::Struct(fields)) =
        (value.as_sequence(), unwrap_dictionary(field.data_type()))
    {
        return fields
            .as_fields()
            .iter()
            .position(|child| child.name().eq_ignore_ascii_case(name))
            .and_then(|index| values.get(index))
            .cloned()
            .unwrap_or(Value::Null);
    }
    Value::Null
}

/// Kleene conjunction of two already-evaluated booleans.
fn kleene_and(left: &Value, right: &Value) -> Value {
    match (left.as_bool(), right.as_bool()) {
        (Some(false), _) | (_, Some(false)) => Value::Bool(false),
        (Some(true), Some(true)) => Value::Bool(true),
        _ => Value::Null,
    }
}

/// Answer one comparison, three-valued except for the two distinctness tests.
pub(crate) fn compare(
    data_type: &DataType,
    left: &Value,
    comparison: Comparison,
    right: &Value,
) -> Value {
    if comparison.is_two_valued() {
        let same = match (left.is_null(), right.is_null()) {
            (true, true) => true,
            (true, false) | (false, true) => false,
            (false, false) => order(data_type, left, right) == Some(Ordering::Equal),
        };
        return Value::Bool(match comparison {
            Comparison::IsDistinctFrom => !same,
            _ => same,
        });
    }
    if left.is_null() || right.is_null() {
        return Value::Null;
    }
    match order(data_type, left, right) {
        Some(ordering) => Value::Bool(comparison.answers(ordering)),
        // Two values that share a declared type but no ordering - a struct
        // against a struct - answer unknown rather than an arbitrary yes.
        None => Value::Null,
    }
}

/// Order two non-null values that share one declared datatype.
pub(crate) fn order(data_type: &DataType, left: &Value, right: &Value) -> Option<Ordering> {
    let data_type = unwrap_dictionary(data_type);
    if let Some((_, scale)) = decimal_parts(data_type) {
        return Some(unscaled_at(left, scale)?.cmp(&unscaled_at(right, scale)?));
    }
    if let Some((family, unit)) = temporal_parts(data_type) {
        return Some(temporal_at(left, family, unit)?.cmp(&temporal_at(right, family, unit)?));
    }
    match data_type {
        DataType::Boolean => Some(left.as_bool()?.cmp(&right.as_bool()?)),
        DataType::Float16 | DataType::Float32 | DataType::Float64 => {
            // IEEE 754 totalOrder, the same predicate Arrow's kernels use.
            Some(left.as_f64()?.total_cmp(&right.as_f64()?))
        }
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            Some(left.as_u128()?.cmp(&right.as_u128()?))
        }
        other if is_text(other) => Some(left.as_str()?.cmp(right.as_str()?)),
        other if is_binary(other) => Some(left.as_bytes()?.cmp(right.as_bytes()?)),
        _ => Some(left.as_i128()?.cmp(&right.as_i128()?)),
    }
}

/// Answer one arithmetic node in the type its output was typed as.
fn arithmetic(
    data_type: &DataType,
    left: &Value,
    operator: Operator,
    right: &Value,
) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    let data_type = unwrap_dictionary(data_type);
    if let Some((_, scale)) = decimal_parts(data_type) {
        let left_unscaled = unscaled_at(left, scale);
        let right_unscaled = unscaled_at(right, scale);
        let (Some(left_unscaled), Some(right_unscaled)) = (left_unscaled, right_unscaled) else {
            return Ok(Value::Null);
        };
        // Multiplication is the one operation whose operands are at a
        // different scale from the result, so it divides the extra places
        // back out exactly.
        let held = match operator {
            Operator::Add => left_unscaled.checked_add(right_unscaled),
            Operator::Sub => left_unscaled.checked_sub(right_unscaled),
            Operator::Mul => left_unscaled
                .checked_mul(right_unscaled)
                .and_then(|held| rescale(held, scale)),
            Operator::Div => (right_unscaled != 0)
                .then(|| {
                    left_unscaled
                        .checked_mul(pow10(scale)?)?
                        .checked_div(right_unscaled)
                })
                .flatten(),
            Operator::Rem => (right_unscaled != 0).then(|| left_unscaled % right_unscaled),
        };
        return Ok(held.map_or(Value::Null, |held| Value::Decimal(held, scale)));
    }
    if matches!(
        data_type,
        DataType::Float16 | DataType::Float32 | DataType::Float64
    ) {
        let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) else {
            return Ok(Value::Null);
        };
        let held = match operator {
            Operator::Add => left + right,
            Operator::Sub => left - right,
            Operator::Mul => left * right,
            Operator::Div => left / right,
            Operator::Rem => left % right,
        };
        return Ok(if matches!(data_type, DataType::Float32) {
            Value::F32(Float32::from_f32(held as f32))
        } else {
            Value::F64(Float::from_f64(held))
        });
    }
    if let Some((family, unit)) = temporal_parts(data_type) {
        let (Some(left), Some(right)) = (
            temporal_at(left, family, unit),
            temporal_at(right, family, unit),
        ) else {
            return Ok(Value::Null);
        };
        let held = match operator {
            Operator::Add => left.checked_add(right),
            Operator::Sub => left.checked_sub(right),
            _ => None,
        };
        let Some(held) = held else {
            return Ok(Value::Null);
        };
        return Ok(match family {
            0 => Value::Date(i32::try_from(held).unwrap_or_default()),
            1 => Value::Time(held, unit),
            2 => match data_type {
                DataType::Timestamp(_, Some(zone)) => Value::Timestamp(held, unit, zone.clone()),
                _ => Value::DateTime(held, unit),
            },
            _ => Value::Duration(held, unit),
        });
    }
    let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) else {
        return Ok(Value::Null);
    };
    let held = match operator {
        Operator::Add => left.checked_add(right),
        Operator::Sub => left.checked_sub(right),
        Operator::Mul => left.checked_mul(right),
        Operator::Div => left.checked_div(right),
        Operator::Rem => left.checked_rem(right),
    };
    Ok(held.map_or(Value::Null, |held| narrow(data_type, held)))
}

/// This value's unscaled coefficient at `scale`, whatever kind of number it is.
///
/// [`Value::decimal_unscaled_at`] restates one exact decimal at another scale;
/// this widens the question to every whole number, because `price > 100` writes
/// the bound as an integer and means it as a decimal.
pub(crate) fn unscaled_at(value: &Value, scale: i8) -> Option<i128> {
    // A decimal answers through its own restatement and never through
    // `as_i128`, which would hand back the raw coefficient and read `1.50` as
    // one hundred and fifty.
    if value.is_decimal() {
        return value.decimal_unscaled_at(scale);
    }
    let held = value.as_i128()?;
    match scale.cmp(&0) {
        Ordering::Equal => Some(held),
        Ordering::Greater => held.checked_mul(pow10(scale)?),
        // A negative scale multiplies, so restating into one only keeps a
        // number whose trailing zeros are already there.
        Ordering::Less => {
            let divisor = pow10(-scale)?;
            (held % divisor == 0).then(|| held / divisor)
        }
    }
}

/// This value's temporal count in one family's unit, dates included.
///
/// A date carries no unit of its own, so [`Value::temporal_count_at`] declines
/// it; here the family says it is a day count and the answer is the day.
pub(crate) fn temporal_at(value: &Value, family: u8, unit: TimeUnit) -> Option<i64> {
    if family == 0 {
        return match value {
            Value::Date(days) => Some(i64::from(*days)),
            other => other.as_i64(),
        };
    }
    value.temporal_count_at(unit).or_else(|| value.as_i64())
}

/// Ten to a non-negative power, as the multiplier a rescale needs.
fn pow10(scale: i8) -> Option<i128> {
    let places = u32::try_from(scale.max(0)).ok()?;
    10_i128.checked_pow(places)
}

/// Divide out the extra places a decimal multiplication produced.
fn rescale(unscaled: i128, scale: i8) -> Option<i128> {
    Some(unscaled / pow10(scale)?)
}

/// Put a whole number back into the width its datatype declares.
fn narrow(data_type: &DataType, held: i128) -> Value {
    match data_type {
        DataType::Int8 => i8::try_from(held).map_or(Value::Null, Value::I8),
        DataType::Int16 => i16::try_from(held).map_or(Value::Null, Value::I16),
        DataType::Int32 => i32::try_from(held).map_or(Value::Null, Value::I32),
        DataType::UInt8 => u8::try_from(held).map_or(Value::Null, Value::U8),
        DataType::UInt16 => u16::try_from(held).map_or(Value::Null, Value::U16),
        DataType::UInt32 => u32::try_from(held).map_or(Value::Null, Value::U32),
        DataType::UInt64 => u64::try_from(held).map_or(Value::Null, Value::U64),
        _ => i64::try_from(held).map_or(Value::Null, Value::I64),
    }
}

/// SQL `like`, with `_` for one character and `%` for any run.
///
/// Written here rather than delegated because `arrow-string` is not a
/// dependency of this workspace and adding one for a pattern match this small
/// would be a poor trade.
fn like_matches(text: &str, pattern: &str, case_insensitive: bool, escape: Option<char>) -> bool {
    let subject: Vec<char> = if case_insensitive {
        text.to_lowercase().chars().collect()
    } else {
        text.chars().collect()
    };
    let mut steps: Vec<(char, bool)> = Vec::new();
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        if Some(character) == escape {
            if let Some(escaped) = characters.next() {
                steps.push((fold_case(escaped, case_insensitive), true));
            }
            continue;
        }
        steps.push((fold_case(character, case_insensitive), false));
    }
    like_walk(&subject, &steps)
}

fn fold_case(character: char, case_insensitive: bool) -> char {
    if case_insensitive {
        character.to_lowercase().next().unwrap_or(character)
    } else {
        character
    }
}

/// Match a folded subject against folded pattern steps, iteratively.
///
/// The `%` backtrack is kept as one remembered position rather than as
/// recursion, so a pathological pattern costs time and never the stack.
fn like_walk(subject: &[char], steps: &[(char, bool)]) -> bool {
    let (mut index, mut step) = (0_usize, 0_usize);
    let (mut star_step, mut star_index) = (None, 0_usize);
    while index < subject.len() {
        match steps.get(step) {
            Some(('%', false)) => {
                star_step = Some(step);
                star_index = index;
                step += 1;
            }
            Some(('_', false)) => {
                step += 1;
                index += 1;
            }
            Some((character, _)) if *character == subject[index] => {
                step += 1;
                index += 1;
            }
            _ => match star_step {
                Some(held) => {
                    step = held + 1;
                    star_index += 1;
                    index = star_index;
                }
                None => return false,
            },
        }
    }
    steps[step..]
        .iter()
        .all(|(held, escaped)| *held == '%' && !*escaped)
}

/// Answer one function call.
#[allow(clippy::too_many_lines)]
fn call(
    function: Function,
    arguments: &[Node],
    values: &[Value],
    data_type: &DataType,
) -> Result<Value> {
    let first = values.first().unwrap_or(&Value::Null);
    // Coalesce and its two-argument spelling are the only functions that mean
    // something when an argument is null.
    if !matches!(function, Function::Coalesce | Function::IfNull)
        && values.iter().any(Value::is_null)
    {
        return Ok(Value::Null);
    }
    Ok(match function {
        Function::Lower => text_value(first, str::to_lowercase),
        Function::Upper => text_value(first, str::to_uppercase),
        Function::Trim => text_value(first, |text| text.trim().to_owned()),
        Function::Length => match first {
            Value::Bytes(bytes) => Value::I64(i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
            other => other.as_str().map_or(Value::Null, |text| {
                Value::I64(i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
            }),
        },
        Function::Substring => {
            let Some(text) = first.as_str() else {
                return Ok(Value::Null);
            };
            // SQL counts from one here, deliberately unlike the zero-based `[]`
            // path step, because both spellings are what their own notation
            // means everywhere else. The window is the standard's: the
            // half-open interval [start, start + length) intersected with the
            // characters that exist, which is why `substring(s, 0, 5)` yields
            // four characters and not five - the window starts before the
            // string and the part before it is not there to take.
            let characters: Vec<char> = text.chars().collect();
            let length = i64::try_from(characters.len()).unwrap_or(i64::MAX);
            let written = values.get(1).and_then(Value::as_i64).unwrap_or(1);
            // A negative start counts back from the end before the window is
            // taken, which is what a caller who writes one always means.
            let start = if written < 0 {
                length + written + 1
            } else {
                written
            };
            let end = match values.get(2).and_then(Value::as_i64) {
                Some(count) if count < 0 => {
                    return Err(missing("a substring length that is not negative"));
                }
                Some(count) => start.saturating_add(count),
                None => length.saturating_add(1),
            };
            let from = usize::try_from(start.max(1) - 1).unwrap_or(0);
            let until = usize::try_from(end.max(1) - 1)
                .unwrap_or(0)
                .min(characters.len());
            Value::String(SmolStr::new(
                characters
                    .get(from..until.max(from))
                    .unwrap_or_default()
                    .iter()
                    .collect::<String>(),
            ))
        }
        Function::StartsWith => text_pair(values, |text, other| text.starts_with(other)),
        Function::EndsWith => text_pair(values, |text, other| text.ends_with(other)),
        Function::Contains => text_pair(values, |text, other| text.contains(other)),
        Function::Concat => {
            let mut joined = String::new();
            for value in values {
                let Some(text) = value.as_str() else {
                    return Ok(Value::Null);
                };
                joined.push_str(text);
            }
            Value::String(SmolStr::new(joined))
        }
        Function::Year | Function::Month | Function::Day | Function::Hour => {
            calendar_part(first, function)
        }
        Function::Truncate => truncate(first, values.get(1).unwrap_or(&Value::Null), data_type)?,
        Function::Coalesce | Function::IfNull => values
            .iter()
            .find(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null),
        Function::Size => match first {
            other if other.is_container() => {
                Value::I64(i64::try_from(other.len()).unwrap_or(i64::MAX))
            }
            _ => Value::Null,
        },
        Function::Get => {
            let container = arguments
                .first()
                .ok_or_else(|| missing("a container for get"))?;
            let key = values.get(1).cloned().unwrap_or(Value::Null);
            let segment = match key.as_i64() {
                Some(index) if !matches!(key, Value::String(_)) => Segment::Index(index),
                _ => Segment::Key(crate::TypedValue::from_value(key)?),
            };
            apply_step(&container.field, first, &segment)
        }
    })
}

fn text_value(value: &Value, rewrite: impl Fn(&str) -> String) -> Value {
    value.as_str().map_or(Value::Null, |text| {
        Value::String(SmolStr::new(rewrite(text)))
    })
}

fn text_pair(values: &[Value], answer: impl Fn(&str, &str) -> bool) -> Value {
    match (
        values.first().and_then(Value::as_str),
        values.get(1).and_then(Value::as_str),
    ) {
        (Some(text), Some(other)) => Value::Bool(answer(text, other)),
        _ => Value::Null,
    }
}

/// Read one calendar field off a temporal, through the one ISO formatter.
///
/// Rendering and slicing rather than reimplementing civil-from-days keeps this
/// crate's calendar in exactly one place; the cost is a small allocation per
/// row, which the vectorized tier does not pay.
fn calendar_part(value: &Value, function: Function) -> Value {
    use crate::generic::iso;

    let text = match value {
        Value::Date(days) => iso::format_date(*days),
        Value::DateTime(count, unit) => iso::format_datetime(*count, *unit),
        Value::Timestamp(count, unit, zone) => iso::format_timestamp(*count, *unit, zone),
        _ => None,
    };
    let Some(text) = text else {
        return Value::Null;
    };
    let bytes = text.as_bytes();
    let read = |from: usize, to: usize| -> Option<i32> {
        text.get(from..to)
            .and_then(|slice| slice.parse::<i32>().ok())
    };
    let parsed = match function {
        Function::Year => read(0, 4),
        Function::Month => read(5, 7),
        Function::Day => read(8, 10),
        // A date has no clock, and midnight is the honest reading of one.
        Function::Hour => {
            if bytes.len() > 12 {
                read(11, 13)
            } else {
                Some(0)
            }
        }
        _ => None,
    };
    parsed.map_or(Value::Null, Value::I32)
}

/// Floor a value to a unit or to a multiple.
fn truncate(value: &Value, unit: &Value, data_type: &DataType) -> Result<Value> {
    if let Some((family, held_unit)) = temporal_parts(unwrap_dictionary(data_type)) {
        let Some(name) = unit.as_str() else {
            return Err(missing(
                "a unit name such as 'hour' for a temporal truncate",
            ));
        };
        let Some(count) = value.temporal_count_at(held_unit) else {
            return Ok(Value::Null);
        };
        let per_second = match held_unit {
            TimeUnit::Second => 1_i64,
            TimeUnit::Millisecond => 1_000,
            TimeUnit::Microsecond => 1_000_000,
            TimeUnit::Nanosecond => 1_000_000_000,
            _ => return Ok(Value::Null),
        };
        let seconds = match name.to_ascii_lowercase().as_str() {
            "second" => 1_i64,
            "minute" => 60,
            "hour" => 3_600,
            "day" => 86_400,
            other => {
                return Err(missing(&format!(
                    "one of the fixed-length units second, minute, hour, day for truncate; \
                     got {other:?}, and a calendar month or year is not a fixed length - \
                     read it with year() or month() instead"
                )));
            }
        };
        // A date already counts in days, so it truncates to itself.
        let step = if family == 0 { 1 } else { seconds * per_second };
        let floored = count.div_euclid(step) * step;
        return Ok(match family {
            0 => Value::Date(i32::try_from(floored).unwrap_or_default()),
            1 => Value::Time(floored, held_unit),
            2 => match unwrap_dictionary(data_type) {
                DataType::Timestamp(_, Some(zone)) => {
                    Value::Timestamp(floored, held_unit, zone.clone())
                }
                _ => Value::DateTime(floored, held_unit),
            },
            _ => Value::Duration(floored, held_unit),
        });
    }
    let Some(width) = unit.as_i64() else {
        return Err(missing("a whole multiple to truncate a number to"));
    };
    if width == 0 {
        return Ok(Value::Null);
    }
    let Some(held) = value.as_i128() else {
        return Ok(Value::Null);
    };
    let width = i128::from(width);
    Ok(narrow(data_type, held.div_euclid(width) * width))
}

/// Convert one value into a datatype, logically rather than physically.
///
/// This is the crate's one value-level conversion, and both the bind-time
/// literal coercion and the `cast` operator run through it, so a folded
/// constant and a per-row cast can never disagree.
///
/// # Errors
///
/// Returns an error when the target cannot hold the value and the caller asked
/// for [`Safety::Strict`].
#[allow(clippy::too_many_lines)]
pub(crate) fn convert(target: &DataType, value: &Value, safety: Safety) -> Result<Value> {
    use crate::generic::iso;

    if value.is_null() || matches!(target, DataType::Null) {
        return Ok(Value::Null);
    }
    let target = unwrap_dictionary(target);
    let refuse = |reason: &str| -> Result<Value> {
        if safety.is_safe() {
            return Ok(Value::Null);
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("expected {reason}, got {}", value.kind()),
        })
    };
    if let Some((precision, scale)) = decimal_parts(target) {
        let Some(unscaled) = unscaled_at(value, scale).or_else(|| {
            value
                .as_f64()
                .and_then(|held| pow10(scale).map(|factor| (held * held_f64(factor)) as i128))
        }) else {
            return refuse("a number an exact decimal can hold");
        };
        if digits(unscaled) > u32::from(precision) {
            return refuse("a number within the declared precision");
        }
        return Ok(Value::Decimal(unscaled, scale));
    }
    if let Some((family, unit)) = temporal_parts(target) {
        if let Some(text) = value.as_str() {
            let parsed = match family {
                0 => iso::parse_date(text).map(Value::Date),
                1 => iso::parse_time(text).map(|(count, unit)| Value::Time(count, unit)),
                2 => match target {
                    DataType::Timestamp(_, Some(_)) => iso::parse_timestamp(text)
                        .map(|(count, unit, zone)| Value::Timestamp(count, unit, zone)),
                    _ => {
                        iso::parse_datetime(text).map(|(count, unit)| Value::DateTime(count, unit))
                    }
                },
                _ => iso::parse_duration(text).map(|(count, unit)| Value::Duration(count, unit)),
            };
            return match parsed {
                Ok(parsed) => convert(target, &parsed, safety),
                Err(_) => refuse("an ISO 8601 temporal"),
            };
        }
        let Some(count) = temporal_at(value, family, unit) else {
            return refuse("a temporal of the same family");
        };
        return Ok(match family {
            0 => match i32::try_from(count) {
                Ok(days) => Value::Date(days),
                Err(_) => return refuse("a date within 32 bits"),
            },
            1 => Value::Time(count, unit),
            2 => match target {
                DataType::Timestamp(_, Some(zone)) => Value::Timestamp(count, unit, zone.clone()),
                _ => Value::DateTime(count, unit),
            },
            _ => Value::Duration(count, unit),
        });
    }
    match target {
        DataType::Boolean => match value {
            Value::Bool(_) => Ok(value.clone()),
            other => match other.as_i128() {
                Some(held) => Ok(Value::Bool(held != 0)),
                None => refuse("a boolean"),
            },
        },
        DataType::Float32 => match value.as_f64() {
            Some(held) => Ok(Value::F32(Float32::from_f32(held as f32))),
            None => refuse("a number"),
        },
        DataType::Float16 | DataType::Float64 => match value.as_f64() {
            Some(held) => Ok(Value::F64(Float::from_f64(held))),
            None => refuse("a number"),
        },
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => {
            let held = unscaled_at(value, 0)
                .or_else(|| value.as_u128().and_then(|held| i128::try_from(held).ok()))
                .or_else(|| value.as_f64().map(|held| held.trunc() as i128))
                .or_else(|| value.as_str().and_then(|text| text.parse::<i128>().ok()));
            let Some(held) = held else {
                return refuse("a whole number");
            };
            match narrow(target, held) {
                Value::Null => refuse("a whole number the declared width can hold"),
                narrowed => Ok(narrowed),
            }
        }
        other if is_text(other) => {
            if let Some(text) = value.as_str() {
                return Ok(Value::String(SmolStr::new(text)));
            }
            let inferred = crate::TypedValue::from_value(value.clone())
                .map(|held| held.data_type().clone())
                .unwrap_or(DataType::Null);
            match super::display::literal_text(&inferred, value) {
                Some(text) => Ok(Value::String(text)),
                None => refuse("a value with a text form"),
            }
        }
        other if is_binary(other) => match value {
            Value::Bytes(_) => Ok(value.clone()),
            Value::String(text) => Ok(Value::Bytes(Arc::from(text.as_bytes()))),
            _ => refuse("bytes"),
        },
        _ => {
            // A nested target keeps the value it was given: the schema-directed
            // walk that reshapes a container lives in the record layer, and
            // duplicating it here would be the second cast this module refuses
            // to grow.
            if value.is_container() {
                return Ok(value.clone());
            }
            refuse("a value the target datatype can hold")
        }
    }
}

/// How many decimal digits an unscaled coefficient has.
fn digits(unscaled: i128) -> u32 {
    let mut magnitude = unscaled.unsigned_abs();
    let mut counted = 1;
    while magnitude >= 10 {
        magnitude /= 10;
        counted += 1;
    }
    counted
}

/// A power of ten as a float, for the one conversion that needs it.
#[allow(clippy::cast_precision_loss)]
fn held_f64(value: i128) -> f64 {
    value as f64
}
