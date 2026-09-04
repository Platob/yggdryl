//! Scalar evaluation: one resolved tree, one row at a time, over [`Scalar`].
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
use crate::{
    DataType, Error, Field, Float16, Float32, Float64, I256, Result, Scalar, TimeUnit, Timezone,
};

/// One row's worth of context: its column values and its holder.
///
/// Either half may be absent. A constant subtree needs neither, which is what
/// lets [`bind`](super::bind) fold by evaluating; a listing filter has a
/// holder and no row, which is what lets it prune before opening anything.
pub(crate) struct Row<'context> {
    values: Option<&'context [Scalar]>,
    holder: Option<&'context dyn Attributes>,
}

impl<'context> Row<'context> {
    pub(crate) const fn new(
        values: Option<&'context [Scalar]>,
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
    /// Returns an error when a strict cast or checked arithmetic refuses a
    /// value, when a column is asked for and no row was supplied, or when a
    /// holder attribute fails.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn eval(&self, row: &Row<'_>) -> Result<Scalar> {
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
                None => Ok(Scalar::Null),
            },
            Kind::And(operands) => {
                let mut unknown = false;
                for operand in operands {
                    match operand.eval(row)?.as_bool() {
                        Some(false) => return Ok(Scalar::Bool(false)),
                        Some(true) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    Scalar::Null
                } else {
                    Scalar::Bool(true)
                })
            }
            Kind::Or(operands) => {
                let mut unknown = false;
                for operand in operands {
                    match operand.eval(row)?.as_bool() {
                        Some(true) => return Ok(Scalar::Bool(true)),
                        Some(false) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    Scalar::Null
                } else {
                    Scalar::Bool(false)
                })
            }
            Kind::Not(inner) => Ok(match inner.eval(row)?.as_bool() {
                Some(held) => Scalar::Bool(!held),
                None => Scalar::Null,
            }),
            Kind::Compare(left, comparison, right) => {
                let left_value = left.eval(row)?;
                let right_value = right.eval(row)?;
                Ok(compare(
                    left.field.dtype(),
                    &left_value,
                    *comparison,
                    &right_value,
                ))
            }
            Kind::In(value, list) => {
                let held = value.eval(row)?;
                if held.is_null() {
                    return Ok(Scalar::Null);
                }
                let mut unknown = false;
                for item in list {
                    let item_value = item.eval(row)?;
                    match compare(value.field.dtype(), &held, Comparison::Eq, &item_value).as_bool()
                    {
                        Some(true) => return Ok(Scalar::Bool(true)),
                        Some(false) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    Scalar::Null
                } else {
                    Scalar::Bool(false)
                })
            }
            Kind::Between(value, low, high) => {
                let dtype = value.field.dtype();
                let held = value.eval(row)?;
                let above = compare(dtype, &held, Comparison::GtEq, &low.eval(row)?);
                let below = compare(dtype, &held, Comparison::LtEq, &high.eval(row)?);
                Ok(kleene_and(&above, &below))
            }
            Kind::IsNull(inner) => Ok(Scalar::Bool(inner.eval(row)?.is_null())),
            Kind::IsNotNull(inner) => Ok(Scalar::Bool(!inner.eval(row)?.is_null())),
            Kind::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => {
                let held = value.eval(row)?;
                let Some(text) = held.as_str() else {
                    return Ok(Scalar::Null);
                };
                Ok(Scalar::Bool(like_matches(
                    text,
                    pattern,
                    *case_insensitive,
                    *escape,
                )))
            }
            Kind::Glob(value, pattern) => {
                let held = value.eval(row)?;
                let Some(text) = held.as_str() else {
                    return Ok(Scalar::Null);
                };
                Ok(Scalar::Bool(crate::uri::pattern::matches_glob_text(
                    text, pattern,
                )))
            }
            Kind::Arithmetic(left, operator, right) => arithmetic(
                self.field.dtype(),
                &left.eval(row)?,
                *operator,
                &right.eval(row)?,
            ),
            Kind::Negate(inner) => {
                let held = inner.eval(row)?;
                held.checked_neg()
            }
            Kind::Function(function, arguments) => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(argument.eval(row)?);
                }
                call(*function, arguments, &values, self.field.dtype())
            }
            Kind::Cast(inner, safety) => {
                let held = inner.eval(row)?;
                match convert(self.field.dtype(), &held, *safety) {
                    Ok(value) => Ok(value),
                    Err(error) if matches!(safety, Safety::Safe) => {
                        let _ = error;
                        Ok(Scalar::Null)
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
                    None => Ok(Scalar::Null),
                }
            }
            Kind::Struct(children) => {
                let mut values = Vec::with_capacity(children.len());
                for child in children {
                    values.push(child.eval(row)?);
                }
                Ok(Scalar::from_sequence(values))
            }
            Kind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(item.eval(row)?);
                }
                Ok(Scalar::from_sequence(values))
            }
            Kind::Map(entries) => {
                let mut pairs = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    pairs.push((key.eval(row)?, value.eval(row)?));
                }
                Scalar::from_mapping(pairs)
            }
        }
    }
}

/// Reach one step into a value, answering null for anything absent.
fn apply_step(field: &Field, value: &Scalar, segment: &Segment) -> Scalar {
    if value.is_null() {
        return Scalar::Null;
    }
    match segment {
        Segment::Field(name) => struct_child(field, value, name),
        Segment::Index(position) => {
            let Some(items) = value.as_sequence() else {
                return Scalar::Null;
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
                .unwrap_or(Scalar::Null)
        }
        Segment::Key(key) => {
            if let Some(entries) = value.as_mapping() {
                let dtype = key.dtype();
                return entries
                    .iter()
                    .find(|(held, _)| {
                        compare(dtype, held, Comparison::IsNotDistinctFrom, key.value()).as_bool()
                            == Some(true)
                    })
                    .map_or(Scalar::Null, |(_, held)| held.clone());
            }
            match key.value().as_str() {
                Some(name) => struct_child(field, value, name),
                None => Scalar::Null,
            }
        }
    }
}

/// Read one struct child from its mapping or schema-ordered sequence spelling.
fn struct_child(field: &Field, value: &Scalar, name: &str) -> Scalar {
    if let Some(entries) = value.as_mapping() {
        return entries
            .iter()
            .find(|(key, _)| {
                key.as_str()
                    .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map_or(Scalar::Null, |(_, held)| held.clone());
    }
    // A struct spelled as a bare sequence takes its order from the schema.
    if let (Some(values), DataType::Struct(fields)) =
        (value.as_sequence(), unwrap_dictionary(field.dtype()))
    {
        return fields
            .as_fields()
            .iter()
            .position(|child| child.name().eq_ignore_ascii_case(name))
            .and_then(|index| values.get(index))
            .cloned()
            .unwrap_or(Scalar::Null);
    }
    Scalar::Null
}

/// Kleene conjunction of two already-evaluated booleans.
fn kleene_and(left: &Scalar, right: &Scalar) -> Scalar {
    match (left.as_bool(), right.as_bool()) {
        (Some(false), _) | (_, Some(false)) => Scalar::Bool(false),
        (Some(true), Some(true)) => Scalar::Bool(true),
        _ => Scalar::Null,
    }
}

/// Answer one comparison, three-valued except for the two distinctness tests.
pub(crate) fn compare(
    dtype: &DataType,
    left: &Scalar,
    comparison: Comparison,
    right: &Scalar,
) -> Scalar {
    if comparison.is_two_valued() {
        let same = match (left.is_null(), right.is_null()) {
            (true, true) => true,
            (true, false) | (false, true) => false,
            (false, false) => order(dtype, left, right) == Some(Ordering::Equal),
        };
        return Scalar::Bool(match comparison {
            Comparison::IsDistinctFrom => !same,
            _ => same,
        });
    }
    if left.is_null() || right.is_null() {
        return Scalar::Null;
    }
    match order(dtype, left, right) {
        Some(ordering) => Scalar::Bool(comparison.answers(ordering)),
        // Two values that share a declared type but no ordering - a struct
        // against a struct - answer unknown rather than an arbitrary yes.
        None => Scalar::Null,
    }
}

/// Order two non-null values that share one declared datatype.
pub(crate) fn order(dtype: &DataType, left: &Scalar, right: &Scalar) -> Option<Ordering> {
    let dtype = unwrap_dictionary(dtype);
    if let Some((_, scale)) = decimal_parts(dtype) {
        return Some(unscaled_at(left, scale)?.cmp(&unscaled_at(right, scale)?));
    }
    if let Some((family, unit)) = temporal_parts(dtype) {
        return Some(temporal_at(left, family, unit)?.cmp(&temporal_at(right, family, unit)?));
    }
    match dtype {
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
    dtype: &DataType,
    left: &Scalar,
    operator: Operator,
    right: &Scalar,
) -> Result<Scalar> {
    if left.is_null() || right.is_null() {
        return Ok(Scalar::Null);
    }
    let operation = match operator {
        Operator::Add => crate::generic::Arithmetic::Add,
        Operator::Sub => crate::generic::Arithmetic::Sub,
        Operator::Mul => crate::generic::Arithmetic::Mul,
        Operator::Div => crate::generic::Arithmetic::Div,
        Operator::Rem => crate::generic::Arithmetic::Rem,
    };
    left.checked_arithmetic_as(right, operation, unwrap_dictionary(dtype))
}

/// This value's unscaled coefficient at `scale`, whatever kind of number it is.
///
/// [`Scalar::decimal_unscaled_at`] restates one exact decimal at another scale;
/// this widens the question to every whole number, because `price > 100` writes
/// the bound as an integer and means it as a decimal.
pub(crate) fn unscaled_at(value: &Scalar, scale: i8) -> Option<i128> {
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
/// A date carries no unit of its own, so [`Scalar::temporal_count_at`] declines
/// it; here the family says it is a day count and the answer is the day.
pub(crate) fn temporal_at(value: &Scalar, family: u8, unit: TimeUnit) -> Option<i64> {
    let _ = family;
    value.temporal_count_at(unit).or_else(|| value.as_i64())
}

/// Put a temporal count back into the exact width, unit, and zone its type declares.
fn temporal_value(dtype: &DataType, count: i64, unit: TimeUnit) -> Result<Scalar> {
    match dtype {
        DataType::Date32 => Scalar::date32_in(
            i32::try_from(count).map_err(|_| missing("a date32 count"))?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Date64 => Scalar::date64_in(count, unit, Timezone::NAIVE),
        DataType::Time32(expected) => {
            if *expected != unit {
                return Err(missing("a time32 count in its declared unit"));
            }
            Scalar::time32(
                i32::try_from(count).map_err(|_| missing("a time32 count"))?,
                unit,
                Timezone::NAIVE,
            )
        }
        DataType::Time64(expected) => {
            if *expected != unit {
                return Err(missing("a time64 count in its declared unit"));
            }
            Scalar::time64(count, unit, Timezone::NAIVE)
        }
        DataType::Timestamp(expected, zone) => {
            if *expected != unit {
                return Err(missing("a datetime64 count in its declared unit"));
            }
            Scalar::datetime64(count, unit, zone.clone().unwrap_or(Timezone::NAIVE))
        }
        DataType::Duration32(expected) => {
            if *expected != unit {
                return Err(missing("a duration32 count in its declared unit"));
            }
            Scalar::duration32(
                i32::try_from(count).map_err(|_| missing("a duration32 count"))?,
                unit,
            )
        }
        DataType::Duration64(expected) => {
            if *expected != unit {
                return Err(missing("a duration64 count in its declared unit"));
            }
            Scalar::duration64(count, unit)
        }
        _ => Err(missing("a temporal datatype")),
    }
}

fn parsed_time(count: i64, unit: TimeUnit) -> Result<Scalar> {
    match unit {
        TimeUnit::Second | TimeUnit::Millisecond => Scalar::time32(
            i32::try_from(count).map_err(|_| missing("a time32 count"))?,
            unit,
            Timezone::NAIVE,
        ),
        TimeUnit::Microsecond | TimeUnit::Nanosecond => {
            Scalar::time64(count, unit, Timezone::NAIVE)
        }
        _ => Err(missing("a fixed-length time unit")),
    }
}

fn parsed_duration(count: i64, unit: TimeUnit) -> Result<Scalar> {
    match unit {
        TimeUnit::Second | TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond => {
            Scalar::duration64(count, unit)
        }
        _ => Err(missing("a fixed-length duration unit")),
    }
}

/// Ten to a non-negative power, as the multiplier a rescale needs.
fn pow10(scale: i8) -> Option<i128> {
    let places = u32::try_from(scale.max(0)).ok()?;
    10_i128.checked_pow(places)
}

/// Put a whole number back into the width its datatype declares.
fn narrow(dtype: &DataType, held: i128) -> Scalar {
    match dtype {
        DataType::Int8 => i8::try_from(held).map_or(Scalar::Null, Scalar::I8),
        DataType::Int16 => i16::try_from(held).map_or(Scalar::Null, Scalar::I16),
        DataType::Int32 => i32::try_from(held).map_or(Scalar::Null, Scalar::I32),
        DataType::UInt8 => u8::try_from(held).map_or(Scalar::Null, Scalar::U8),
        DataType::UInt16 => u16::try_from(held).map_or(Scalar::Null, Scalar::U16),
        DataType::UInt32 => u32::try_from(held).map_or(Scalar::Null, Scalar::U32),
        DataType::UInt64 => u64::try_from(held).map_or(Scalar::Null, Scalar::U64),
        _ => i64::try_from(held).map_or(Scalar::Null, Scalar::I64),
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
    values: &[Scalar],
    dtype: &DataType,
) -> Result<Scalar> {
    let first = values.first().unwrap_or(&Scalar::Null);
    // Coalesce and its two-argument spelling are the only functions that mean
    // something when an argument is null.
    if !matches!(function, Function::Coalesce | Function::IfNull)
        && values.iter().any(Scalar::is_null)
    {
        return Ok(Scalar::Null);
    }
    Ok(match function {
        Function::Lower => text_value(first, str::to_lowercase),
        Function::Upper => text_value(first, str::to_uppercase),
        Function::Trim => text_value(first, |text| text.trim().to_owned()),
        Function::Length => match first {
            Scalar::Bytes(bytes) => Scalar::I64(i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
            other => other.as_str().map_or(Scalar::Null, |text| {
                Scalar::I64(i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
            }),
        },
        Function::Substring => {
            let Some(text) = first.as_str() else {
                return Ok(Scalar::Null);
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
            let written = values.get(1).and_then(Scalar::as_i64).unwrap_or(1);
            // A negative start counts back from the end before the window is
            // taken, which is what a caller who writes one always means.
            let start = if written < 0 {
                length + written + 1
            } else {
                written
            };
            let end = match values.get(2).and_then(Scalar::as_i64) {
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
            Scalar::String(SmolStr::new(
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
                    return Ok(Scalar::Null);
                };
                joined.push_str(text);
            }
            Scalar::String(SmolStr::new(joined))
        }
        Function::Year | Function::Month | Function::Day | Function::Hour => {
            calendar_part(first, function)
        }
        Function::Truncate => truncate(first, values.get(1).unwrap_or(&Scalar::Null), dtype)?,
        Function::Coalesce | Function::IfNull => values
            .iter()
            .find(|value| !value.is_null())
            .cloned()
            .unwrap_or(Scalar::Null),
        Function::Size => match first {
            other if other.is_container() => {
                Scalar::I64(i64::try_from(other.len()).unwrap_or(i64::MAX))
            }
            _ => Scalar::Null,
        },
        Function::Get => {
            let container = arguments
                .first()
                .ok_or_else(|| missing("a container for get"))?;
            let key = values.get(1).cloned().unwrap_or(Scalar::Null);
            let segment = match key.as_i64() {
                Some(index) if !matches!(key, Scalar::String(_)) => Segment::Index(index),
                _ => Segment::Key(crate::TypedScalar::from_value(key)?),
            };
            apply_step(&container.field, first, &segment)
        }
    })
}

fn text_value(value: &Scalar, rewrite: impl Fn(&str) -> String) -> Scalar {
    value.as_str().map_or(Scalar::Null, |text| {
        Scalar::String(SmolStr::new(rewrite(text)))
    })
}

fn text_pair(values: &[Scalar], answer: impl Fn(&str, &str) -> bool) -> Scalar {
    match (
        values.first().and_then(Scalar::as_str),
        values.get(1).and_then(Scalar::as_str),
    ) {
        (Some(text), Some(other)) => Scalar::Bool(answer(text, other)),
        _ => Scalar::Null,
    }
}

/// Read one calendar field off a temporal, through the one ISO formatter.
///
/// Rendering and slicing rather than reimplementing civil-from-days keeps this
/// crate's calendar in exactly one place; the cost is a small allocation per
/// row, which the vectorized tier does not pay.
fn calendar_part(value: &Scalar, function: Function) -> Scalar {
    use crate::generic::iso;

    let text = match value {
        Scalar::Date32(days, _, _) => iso::format_date(*days),
        Scalar::Date64(count, unit, _) => value
            .temporal_count_at(TimeUnit::Day)
            .and_then(|days| i32::try_from(days).ok())
            .and_then(iso::format_date)
            .or_else(|| iso::format_datetime(*count, *unit)),
        Scalar::DateTime64(count, unit, zone) if zone.is_naive() => {
            iso::format_datetime(*count, *unit)
        }
        Scalar::DateTime64(count, unit, zone) => iso::format_timestamp(*count, *unit, zone),
        _ => None,
    };
    let Some(text) = text else {
        return Scalar::Null;
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
    parsed.map_or(Scalar::Null, Scalar::I32)
}

/// Floor a value to a unit or to a multiple.
fn truncate(value: &Scalar, unit: &Scalar, dtype: &DataType) -> Result<Scalar> {
    if let Some((family, held_unit)) = temporal_parts(unwrap_dictionary(dtype)) {
        let Some(name) = unit.as_str() else {
            return Err(missing(
                "a unit name such as 'hour' for a temporal truncate",
            ));
        };
        let Some(count) = value.temporal_count_at(held_unit) else {
            return Ok(Scalar::Null);
        };
        let per_second = match held_unit {
            TimeUnit::Second => 1_i64,
            TimeUnit::Millisecond => 1_000,
            TimeUnit::Microsecond => 1_000_000,
            TimeUnit::Nanosecond => 1_000_000_000,
            _ => return Ok(Scalar::Null),
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
        return temporal_value(unwrap_dictionary(dtype), floored, held_unit).or(Ok(Scalar::Null));
    }
    let Some(width) = unit.as_i64() else {
        return Err(missing("a whole multiple to truncate a number to"));
    };
    if width == 0 {
        return Ok(Scalar::Null);
    }
    let Some(held) = value.as_i128() else {
        return Ok(Scalar::Null);
    };
    let width = i128::from(width);
    Ok(narrow(dtype, held.div_euclid(width) * width))
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
pub(crate) fn convert(target: &DataType, value: &Scalar, safety: Safety) -> Result<Scalar> {
    use crate::generic::iso;

    if value.is_null() || matches!(target, DataType::Null) {
        return Ok(Scalar::Null);
    }
    let target = unwrap_dictionary(target);
    let refuse = |reason: &str| -> Result<Scalar> {
        if safety.is_safe() {
            return Ok(Scalar::Null);
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
        return Ok(match target {
            DataType::Decimal256 { .. } => Scalar::d256(I256::from_i128(unscaled), scale),
            _ => Scalar::d128(unscaled, scale),
        });
    }
    if let Some((family, unit)) = temporal_parts(target) {
        if let Some(text) = value.as_str() {
            let parsed = match family {
                0 => iso::parse_date(text).map(Scalar::date32),
                1 => iso::parse_time(text).and_then(|(count, unit)| parsed_time(count, unit)),
                2 => match target {
                    DataType::Timestamp(_, Some(_)) => iso::parse_timestamp(text)
                        .and_then(|(count, unit, zone)| Scalar::datetime64(count, unit, zone)),
                    _ => iso::parse_datetime(text)
                        .and_then(|(count, unit)| Scalar::datetime64(count, unit, Timezone::NAIVE)),
                },
                _ => {
                    iso::parse_duration(text).and_then(|(count, unit)| parsed_duration(count, unit))
                }
            };
            return match parsed {
                Ok(parsed) => convert(target, &parsed, safety),
                Err(_) => refuse("an ISO 8601 temporal"),
            };
        }
        let Some(count) = temporal_at(value, family, unit) else {
            return refuse("a temporal of the same family");
        };
        return temporal_value(target, count, unit)
            .or_else(|_| refuse("a temporal within the declared width and unit"));
    }
    match target {
        DataType::Boolean => match value {
            Scalar::Bool(_) => Ok(value.clone()),
            other => match other.as_i128() {
                Some(held) => Ok(Scalar::Bool(held != 0)),
                None => refuse("a boolean"),
            },
        },
        DataType::Float16 => match value.as_f64() {
            Some(held) => Ok(Scalar::F16(Float16::from_f16(half::f16::from_f64(held)))),
            None => refuse("a number"),
        },
        DataType::Float32 => match value.as_f64() {
            Some(held) => Ok(Scalar::F32(Float32::from_f32(held as f32))),
            None => refuse("a number"),
        },
        DataType::Float64 => match value.as_f64() {
            Some(held) => Ok(Scalar::F64(Float64::from_f64(held))),
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
                Scalar::Null => refuse("a whole number the declared width can hold"),
                narrowed => Ok(narrowed),
            }
        }
        DataType::Ascii16
        | DataType::Ascii24
        | DataType::Ascii32
        | DataType::Ascii64
        | DataType::Ascii96
        | DataType::Ascii128 => {
            // The row tier enforces the width rule the cast plan enforces on
            // columns, so the two tiers refuse the same values.
            let Some(bytes) = crate::datatype::ascii_bytes(value) else {
                return refuse("ASCII text");
            };
            let width = target.ascii_width().unwrap_or(0);
            match crate::datatype::ascii_text(width, bytes) {
                Ok(text) => Ok(Scalar::from(text)),
                Err(_) if safety.is_safe() => Ok(Scalar::Null),
                Err(error) => Err(error),
            }
        }
        other if is_text(other) => {
            if let Some(text) = value.as_str() {
                return Ok(Scalar::String(SmolStr::new(text)));
            }
            let inferred = crate::TypedScalar::from_value(value.clone())
                .map(|held| held.dtype().clone())
                .unwrap_or(DataType::Null);
            match super::display::literal_text(&inferred, value) {
                Some(text) => Ok(Scalar::String(text)),
                None => refuse("a value with a text form"),
            }
        }
        other if is_binary(other) => match value {
            Scalar::Bytes(_) => Ok(value.clone()),
            Scalar::String(text) => Ok(Scalar::Bytes(Arc::from(text.as_bytes()))),
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
