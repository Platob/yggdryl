//! Where an expression's output type is decided - and the only such place.
//!
//! [`Term::field`] answers, recursively, the [`Field`] a term produces
//! against a schema. Every other part of the module asks it rather
//! than deciding for itself: [`bind`](super::bind) uses it to know what to
//! coerce a literal into, the scalar evaluator uses it to know how to compare
//! two values, and the vectorized evaluator uses it to know what array to
//! build. One answer, three consumers - which is the only way the three can be
//! made to agree.
//!
//! # Promotion
//!
//! Arithmetic and comparison both need a type the two sides share, and the
//! rules are deliberately narrow:
//!
//! * integers promote to `int64`, which is the width every integer literal is
//!   written in anyway;
//! * a float on either side makes the result `float64`;
//! * two decimals meet at a decimal wide enough for both, and a decimal never
//!   meets a float - an exact number and an approximate one have no common
//!   type that is honest, so the expression is refused and the caller writes
//!   the cast they meant;
//! * a temporal meets a temporal of the same family, at the finer unit;
//! * text meets text, bytes meet bytes, and nothing else meets anything.
//!
//! Everything a rule cannot prove is an error naming both sides, never a
//! silent widening.

use smol_str::{SmolStr, format_smolstr};

use super::path::FieldSegment;
use super::{Function, Literal, Operator, Safety, Term, named};
use crate::{DataType, DataTypeKind, Error, Field, Result, Scalar, StructType, TimeUnit};

/// The widest exact decimal this crate builds by promotion.
const DECIMAL_LIMIT: u8 = 38;

/// The fewest fractional places an exact quotient keeps.
const MIN_QUOTIENT_SCALE: i8 = 6;

impl Term {
    /// The [`Field`] this term produces against a struct root schema.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column, the function, or the two datatypes
    /// when the term cannot be typed against this schema.
    pub fn field(&self, schema: &Field) -> Result<Field> {
        self.check_budget()?;
        resolve(self, schema)
    }

    /// Return whether this term answers a boolean against a schema.
    ///
    /// # Errors
    ///
    /// Returns the same error [`Self::field`] would.
    pub fn is_predicate(&self, schema: &Field) -> Result<bool> {
        Ok(matches!(
            self.field(schema)?.dtype(),
            DataType::Boolean | DataType::Null
        ))
    }
}

fn typing_error(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: reason.into(),
    }
}

#[allow(clippy::too_many_lines)]
fn resolve(expression: &Term, schema: &Field) -> Result<Field> {
    match expression {
        Term::Literal(held) => Ok(named(
            expression,
            held.dtype().clone(),
            held.value().is_null(),
        )),
        Term::Path(steps) => {
            let (first, rest) = steps.split_first().ok_or_else(|| {
                typing_error("expected a path to start at a column, got the row itself")
            })?;
            let FieldSegment::Field(name) = first else {
                return Err(typing_error(format_smolstr!(
                    "expected a path to start at a column, got the step {first}"
                )));
            };
            let mut field = column_field(name, schema)?;
            for step in rest {
                field = step.apply_field(&field)?;
            }
            Ok(if rest.is_empty() {
                field
            } else {
                field.with_name(SmolStr::new(expression.to_string()))
            })
        }
        // A parameter has no type until it is supplied. `bind` substitutes
        // every one before typing, so a parameter reaching here means the
        // caller asked for a type an unbound expression does not have.
        Term::Parameter(name) => Err(typing_error(format_smolstr!(
            "expected parameter :{name} to be supplied before typing"
        ))),
        Term::And(operands) | Term::Or(operands) => {
            let mut nullable = false;
            for operand in operands.iter() {
                let field = resolve(operand, schema)?;
                require_boolean(&field, expression)?;
                nullable |= field.is_nullable();
            }
            Ok(named(expression, DataType::Boolean, nullable))
        }
        Term::Not(inner) => {
            let field = resolve(inner, schema)?;
            require_boolean(&field, expression)?;
            Ok(named(expression, DataType::Boolean, field.is_nullable()))
        }
        Term::Compare(left, comparison, right) => {
            let left = resolve(left, schema)?;
            let right = resolve(right, schema)?;
            common_type(left.dtype(), right.dtype()).ok_or_else(|| {
                typing_error(format_smolstr!(
                    "expected comparable operands, got {} {} {}",
                    left.dtype(),
                    comparison.as_str(),
                    right.dtype()
                ))
            })?;
            // The two distinctness tests answer about nulls, so they are the
            // only comparisons that never produce one.
            let nullable =
                !comparison.is_two_valued() && (left.is_nullable() || right.is_nullable());
            Ok(named(expression, DataType::Boolean, nullable))
        }
        Term::In(value, list) => {
            let value = resolve(value, schema)?;
            let mut nullable = value.is_nullable();
            for item in list.iter() {
                let item = resolve(item, schema)?;
                common_type(value.dtype(), item.dtype()).ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected every `in` value to be comparable with {}, got {}",
                        value.dtype(),
                        item.dtype()
                    ))
                })?;
                nullable |= item.is_nullable();
            }
            Ok(named(expression, DataType::Boolean, nullable))
        }
        Term::Between(value, low, high) => {
            let value = resolve(value, schema)?;
            let low = resolve(low, schema)?;
            let high = resolve(high, schema)?;
            for bound in [&low, &high] {
                common_type(value.dtype(), bound.dtype()).ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected `between` bounds comparable with {}, got {}",
                        value.dtype(),
                        bound.dtype()
                    ))
                })?;
            }
            Ok(named(
                expression,
                DataType::Boolean,
                value.is_nullable() || low.is_nullable() || high.is_nullable(),
            ))
        }
        Term::IsNull(inner) | Term::IsNotNull(inner) => {
            resolve(inner, schema)?;
            Ok(named(expression, DataType::Boolean, false))
        }
        Term::Like { value, pattern, .. } | Term::Glob(value, pattern) => {
            let value = resolve(value, schema)?;
            let pattern = resolve(pattern, schema)?;
            for side in [&value, &pattern] {
                if !is_text(side.dtype()) {
                    return Err(typing_error(format_smolstr!(
                        "expected text on both sides of a pattern match, got {}",
                        side.dtype()
                    )));
                }
            }
            Ok(named(
                expression,
                DataType::Boolean,
                value.is_nullable() || pattern.is_nullable(),
            ))
        }
        Term::Arithmetic(left, operator, right) => {
            let left = resolve(left, schema)?;
            let right = resolve(right, schema)?;
            let dtype =
                arithmetic_type(left.dtype(), *operator, right.dtype()).ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected arithmetic operands that share a type, got {} {} {}",
                        left.dtype(),
                        operator.as_str(),
                        right.dtype()
                    ))
                })?;
            // Arithmetic propagates operand nulls. Overflow, division by zero,
            // and inexact decimal division are errors rather than nulls.
            let nullable = left.is_nullable() || right.is_nullable();
            Ok(named(expression, dtype, nullable))
        }
        Term::Negate(inner) => {
            let field = resolve(inner, schema)?;
            if !is_signed_numeric(field.dtype()) {
                return Err(typing_error(format_smolstr!(
                    "expected a signed number to negate, got {}",
                    field.dtype()
                )));
            }
            Ok(named(
                expression,
                field.dtype().clone(),
                field.is_nullable(),
            ))
        }
        Term::Function(function, arguments) => {
            function_field(expression, function, arguments, schema)
        }
        Term::Cast(inner, dtype, safety) => {
            let field = resolve(inner, schema)?;
            Ok(named(
                expression,
                dtype.clone(),
                field.is_nullable() || matches!(safety, Safety::Safe),
            ))
        }
        Term::Case {
            branches,
            otherwise,
        } => {
            let mut unified: Option<DataType> = None;
            let mut nullable = otherwise.is_none();
            for (when, then) in branches.iter() {
                let when = resolve(when, schema)?;
                require_boolean(&when, expression)?;
                let then = resolve(then, schema)?;
                nullable |= then.is_nullable();
                unified = Some(unify(unified.as_ref(), then.dtype(), expression)?);
            }
            if let Some(otherwise) = otherwise {
                let otherwise = resolve(otherwise, schema)?;
                nullable |= otherwise.is_nullable();
                unified = Some(unify(unified.as_ref(), otherwise.dtype(), expression)?);
            }
            Ok(named(
                expression,
                unified.unwrap_or(DataType::Null),
                nullable,
            ))
        }
        Term::Struct(children) => {
            let mut fields = Vec::with_capacity(children.len());
            for (name, value) in children.iter() {
                fields.push(resolve(value, schema)?.with_name(name.clone()));
            }
            Ok(named(
                expression,
                DataType::from(StructType::from_fields(fields)?),
                false,
            ))
        }
        Term::Serie(items) => {
            let mut unified: Option<DataType> = None;
            let mut nullable = false;
            for item in items.iter() {
                let item = resolve(item, schema)?;
                nullable |= item.is_nullable();
                unified = Some(unify(unified.as_ref(), item.dtype(), expression)?);
            }
            let item = Field::new("item", unified.unwrap_or(DataType::Null), nullable);
            Ok(named(expression, DataType::serie(item), false))
        }
        Term::Map(entries) => {
            let mut keys: Option<DataType> = None;
            let mut values: Option<DataType> = None;
            let mut nullable = false;
            for (key, value) in entries.iter() {
                let key = resolve(key, schema)?;
                let value = resolve(value, schema)?;
                nullable |= value.is_nullable();
                keys = Some(unify(keys.as_ref(), key.dtype(), expression)?);
                values = Some(unify(values.as_ref(), value.dtype(), expression)?);
            }
            let key = Field::new("key", keys.unwrap_or(DataType::utf8()), false);
            let value = Field::new("value", values.unwrap_or(DataType::Null), nullable);
            let entries_field = Field::new(
                "entries",
                DataType::from(StructType::from_fields([key, value])?),
                false,
            );
            Ok(named(
                expression,
                DataType::map(entries_field, false)?,
                false,
            ))
        }
    }
}

/// The top-level column a name resolves to, ASCII case-insensitively.
///
/// A schema that declares two columns differing only in case makes an
/// unquoted reference genuinely ambiguous, and first-match-wins is the one
/// resolution rule nobody can debug. Both names are reported.
pub(crate) fn column_index(name: &str, schema: &Field) -> Result<usize> {
    let matches: Vec<usize> = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, field)| field.name().eq_ignore_ascii_case(name))
        .map(|(index, _)| index)
        .collect();
    if matches.len() > 1 {
        let names: Vec<&str> = matches
            .iter()
            .filter_map(|index| schema.get_field(*index).map(Field::name))
            .collect();
        return Err(typing_error(format_smolstr!(
            "expected {name:?} to name one column, got {}; quote the one meant",
            names.join(" and ")
        )));
    }
    matches
        .first()
        .copied()
        .ok_or_else(|| unknown_column(name, schema))
}

/// The top-level column a name resolves to, as a field.
pub(crate) fn column_field(name: &str, schema: &Field) -> Result<Field> {
    let index = column_index(name, schema)?;
    schema
        .get_field(index)
        .cloned()
        .ok_or_else(|| unknown_column(name, schema))
}

/// The error a column the schema does not declare produces.
///
/// One sentence, shared with [`bind`](super::bind), so the same typo reads the
/// same way whether it was caught while typing or while resolving.
pub(crate) fn unknown_column(name: &str, schema: &Field) -> Error {
    let columns = schema
        .fields()
        .iter()
        .map(Field::name)
        .collect::<Vec<_>>()
        .join(", ");
    typing_error(format_smolstr!(
        "expected a column the schema declares, got {name:?}; it has {}",
        crate::text::elide_display(&columns)
    ))
}

fn require_boolean(field: &Field, expression: &Term) -> Result<()> {
    if matches!(field.dtype(), DataType::Boolean | DataType::Null) {
        return Ok(());
    }
    Err(typing_error(format_smolstr!(
        "expected a boolean operand in {expression}, got {}",
        field.dtype()
    )))
}

/// Unify one more branch type into the type a multi-branch node produces.
fn unify(held: Option<&DataType>, next: &DataType, expression: &Term) -> Result<DataType> {
    let Some(held) = held else {
        return Ok(next.clone());
    };
    common_type(held, next).ok_or_else(|| {
        typing_error(format_smolstr!(
            "expected every branch of {expression} to share a type, got {held} and {next}"
        ))
    })
}

/// Look through a dictionary to the type it encodes.
///
/// A dictionary is a physical encoding, not a logical type: `city = 'Oslo'`
/// means the same thing whether or not the column is dictionary-encoded.
pub(crate) fn unwrap_dictionary(dtype: &DataType) -> &DataType {
    match dtype {
        DataType::Dictionary(dictionary) => unwrap_dictionary(dictionary.value()),
        DataType::RunEndEncoded(encoded) => unwrap_dictionary(encoded.values().dtype()),
        other => other,
    }
}

/// Return whether a datatype holds text.
///
/// The kind is the one source of truth, so a code is text here as everywhere
/// else: its row values are the trimmed string.
pub(crate) fn is_text(dtype: &DataType) -> bool {
    let dtype = unwrap_dictionary(dtype);
    matches!(dtype, DataType::Null)
        || matches!(dtype.kind(), DataTypeKind::Text | DataTypeKind::Code)
}

/// Return whether a datatype holds bytes.
pub(crate) fn is_binary(dtype: &DataType) -> bool {
    matches!(unwrap_dictionary(dtype), crate::bytes_dtypes!())
}

/// Return whether a datatype holds a whole number.
pub(crate) fn is_integer(dtype: &DataType) -> bool {
    matches!(
        unwrap_dictionary(dtype),
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}

/// Return whether a datatype holds an approximate number.
pub(crate) fn is_float(dtype: &DataType) -> bool {
    matches!(
        unwrap_dictionary(dtype),
        DataType::Float16 | DataType::Float32 | DataType::Float64
    )
}

/// The precision and scale of an exact decimal, if it is one.
pub(crate) const fn decimal_parts(dtype: &DataType) -> Option<(u8, i8)> {
    match dtype {
        DataType::Decimal32 { precision, scale }
        | DataType::Decimal64 { precision, scale }
        | DataType::Decimal128 { precision, scale }
        | DataType::Decimal256 { precision, scale } => Some((*precision, *scale)),
        _ => None,
    }
}

/// Return whether a datatype can be negated without changing its type.
fn is_signed_numeric(dtype: &DataType) -> bool {
    matches!(
        unwrap_dictionary(dtype),
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
    ) || decimal_parts(unwrap_dictionary(dtype)).is_some()
        || matches!(
            unwrap_dictionary(dtype),
            DataType::Duration32(_) | DataType::Duration64(_)
        )
}

/// The temporal family and unit of a datatype, if it has one.
pub(crate) const fn temporal_parts(dtype: &DataType) -> Option<(u8, TimeUnit)> {
    match dtype {
        leaf_dtype @ (DataType::Date32 | DataType::Date64) => {
            let leaf = &leaf_dtype
                .date_type()
                .expect("the variant was just matched");
            Some((0, leaf.unit()))
        }
        leaf_dtype @ (DataType::Time32(_) | DataType::Time64(_)) => {
            let leaf = &leaf_dtype
                .time_type()
                .expect("the variant was just matched");
            Some((1, leaf.unit()))
        }
        leaf_dtype @ DataType::DateTime64 { .. } => {
            let leaf = &leaf_dtype
                .datetime_type()
                .expect("the variant was just matched");
            Some((2, leaf.unit()))
        }
        leaf_dtype @ (DataType::Duration32(_) | DataType::Duration64(_)) => {
            let leaf = &leaf_dtype
                .duration_type()
                .expect("the variant was just matched");
            Some((3, leaf.unit()))
        }
        _ => None,
    }
}

/// How fine a unit is, so two temporals can meet at the finer one.
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

/// The type two operands share, or `None` when they share none.
///
/// The whole promotion table is [`DataType::merge_with`]; this is the one
/// caller that wants an `Option` rather than a refusal naming both sides,
/// because an unshared pair here is a typing outcome, not an error to report.
pub(crate) fn common_type(left: &DataType, right: &DataType) -> Option<DataType> {
    unwrap_dictionary(left)
        .merge_exact(unwrap_dictionary(right), crate::Widening::Up)
        .ok()
}

/// The type an arithmetic node produces, or `None` when it has none.
fn arithmetic_type(left: &DataType, operator: Operator, right: &DataType) -> Option<DataType> {
    let left = unwrap_dictionary(left);
    let right = unwrap_dictionary(right);
    if matches!(left, DataType::Duration32(_) | DataType::Duration64(_))
        && is_integer(right)
        && matches!(operator, Operator::Mul | Operator::Div)
    {
        return Some(left.clone());
    }
    if is_integer(left)
        && matches!(right, DataType::Duration32(_) | DataType::Duration64(_))
        && matches!(operator, Operator::Mul)
    {
        return Some(right.clone());
    }
    // A temporal and a duration are the one mixed-family arithmetic that is
    // meaningful, and it is spelled out rather than promoted.
    match (temporal_parts(left), temporal_parts(right), operator) {
        (Some((family, _)), Some((3, _)), Operator::Add | Operator::Sub) if family != 3 => {
            return Some(left.clone());
        }
        (Some((3, _)), Some((family, _)), Operator::Add) if family != 3 => {
            return Some(right.clone());
        }
        (Some((family, left_unit)), Some((other, right_unit)), Operator::Sub)
            if family == other && family != 3 =>
        {
            let unit = if unit_rank(left_unit) >= unit_rank(right_unit) {
                left_unit
            } else {
                right_unit
            };
            return DataType::duration64(unit).ok();
        }
        _ => {}
    }
    let shared = common_type(left, right)?;
    if decimal_parts(&shared).is_some() {
        let (left_precision, left_scale) = exact_parts(left)?;
        let (right_precision, right_scale) = exact_parts(right)?;
        let integral = |precision: u8, scale: i8| {
            precision.saturating_sub(u8::try_from(scale.max(0)).unwrap_or(0))
        };
        // The three shapes an exact result takes. Each says how many
        // fractional places the operation can produce and how many integral
        // ones it needs to hold them, so the declared type can always hold
        // every value the operation can compute.
        let (precision, scale) = match operator {
            Operator::Mul => (
                left_precision.saturating_add(right_precision),
                left_scale.checked_add(right_scale)?,
            ),
            Operator::Div => {
                let scale = left_scale.max(right_scale).max(MIN_QUOTIENT_SCALE);
                let whole = integral(left_precision, left_scale)
                    .saturating_add(u8::try_from(right_scale.max(0)).unwrap_or(0))
                    .max(1);
                (
                    whole.saturating_add(u8::try_from(scale.max(0)).unwrap_or(0)),
                    scale,
                )
            }
            other => {
                let scale = left_scale.max(right_scale);
                let whole = integral(left_precision, left_scale)
                    .max(integral(right_precision, right_scale))
                    // A sum of two n-digit numbers needs one more digit; a
                    // remainder never needs more than its narrower operand.
                    .saturating_add(u8::from(matches!(other, Operator::Add | Operator::Sub)));
                (
                    whole.saturating_add(u8::try_from(scale.max(0)).unwrap_or(0)),
                    scale,
                )
            }
        };
        if scale > i8::try_from(DECIMAL_LIMIT).ok()? {
            return None;
        }
        return DataType::decimal128(
            precision
                .min(DECIMAL_LIMIT)
                .max(u8::try_from(scale.max(0)).unwrap_or(1).max(1)),
            scale,
        )
        .ok();
    }
    if is_integer(&shared)
        || is_float(&shared)
        || (matches!(shared, DataType::Duration32(_) | DataType::Duration64(_))
            && matches!(operator, Operator::Add | Operator::Sub))
    {
        return Some(shared);
    }
    None
}

/// The precision and scale a datatype holds as an exact number.
///
/// A whole number is an exact number of scale zero, and its precision is the
/// digits its width can actually hold - which is what keeps `int32 + decimal`
/// from claiming the 38 digits a decimal could have had.
fn exact_parts(dtype: &DataType) -> Option<(u8, i8)> {
    let dtype = unwrap_dictionary(dtype);
    if let Some(parts) = decimal_parts(dtype) {
        return Some(parts);
    }
    Some(match dtype {
        DataType::Int8 | DataType::UInt8 => (3, 0),
        DataType::Int16 | DataType::UInt16 => (5, 0),
        DataType::Int32 | DataType::UInt32 => (10, 0),
        DataType::Int64 => (19, 0),
        DataType::UInt64 => (20, 0),
        DataType::Null => (1, 0),
        _ => return None,
    })
}

/// The output field of one function call.
fn function_field(
    expression: &Term,
    function: &Function,
    arguments: &[Term],
    schema: &Field,
) -> Result<Field> {
    // A user function is typed by its registered signature, and refused by
    // name when nothing is registered under it.
    if let Function::User(reference) = function {
        let registered = super::user::lookup_function(reference)
            .map_err(|error| typing_error(format_smolstr!("{error}")))?;
        let mut fields = Vec::with_capacity(arguments.len());
        for argument in arguments {
            fields.push(resolve(argument, schema)?);
        }
        return registered
            .signature()
            .output_field(&fields, expression)
            .map_err(|error| typing_error(format_smolstr!("{error}")));
    }
    let (least, most) = function.arity();
    if arguments.len() < least || arguments.len() > most {
        return Err(typing_error(format_smolstr!(
            "expected {} to take {least} to {most} arguments, got {}",
            function.as_str(),
            arguments.len()
        )));
    }
    let mut fields = Vec::with_capacity(arguments.len());
    for argument in arguments {
        fields.push(resolve(argument, schema)?);
    }
    let nullable = fields.iter().any(Field::is_nullable);
    let first = fields
        .first()
        .map(Field::dtype)
        .cloned()
        .unwrap_or(DataType::Null);
    let dtype = match function {
        Function::Lower | Function::Upper | Function::Trim | Function::Substring => {
            if !is_text(&first) {
                return Err(typing_error(format_smolstr!(
                    "expected text for {}, got {first}",
                    function.as_str()
                )));
            }
            DataType::utf8()
        }
        Function::Concat => {
            for field in &fields {
                if !is_text(field.dtype()) {
                    return Err(typing_error(format_smolstr!(
                        "expected text for concat, got {}",
                        field.dtype()
                    )));
                }
            }
            DataType::utf8()
        }
        Function::Length => {
            if !is_text(&first) && !is_binary(&first) {
                return Err(typing_error(format_smolstr!(
                    "expected text or bytes for length, got {first}"
                )));
            }
            DataType::Int64
        }
        Function::StartsWith | Function::EndsWith | Function::Contains => {
            for field in &fields {
                if !is_text(field.dtype()) {
                    return Err(typing_error(format_smolstr!(
                        "expected text for {}, got {}",
                        function.as_str(),
                        field.dtype()
                    )));
                }
            }
            DataType::Boolean
        }
        Function::Year | Function::Month | Function::Day | Function::Hour => {
            if temporal_parts(unwrap_dictionary(&first)).is_none() {
                return Err(typing_error(format_smolstr!(
                    "expected a date or a timestamp for {}, got {first}",
                    function.as_str()
                )));
            }
            DataType::Int32
        }
        Function::Truncate => first.clone(),
        Function::User(_) => unreachable!("a user function returned above"),
        Function::Coalesce | Function::IfNull => {
            let mut unified: Option<DataType> = None;
            for field in &fields {
                unified = Some(unify(unified.as_ref(), field.dtype(), expression)?);
            }
            unified.unwrap_or(DataType::Null)
        }
        Function::Size => {
            if !matches!(
                unwrap_dictionary(&first),
                DataType::Serie(_)
                    | DataType::SerieView(_)
                    | DataType::FixedSizeSerie(..)
                    | DataType::LargeSerie(_)
                    | DataType::LargeSerieView(_)
                    | DataType::Map(_)
                    | DataType::SortedMap(_)
            ) {
                return Err(typing_error(format_smolstr!(
                    "expected a serie or a map for size, got {first}"
                )));
            }
            DataType::Int64
        }
        Function::Get => {
            let key = fields
                .get(1)
                .ok_or_else(|| typing_error("expected a key for get"))?;
            // A constant key names the child it reaches, which is what typing
            // a struct child needs; a computed key can only say its type.
            let segment = match arguments.get(1).and_then(Term::as_literal) {
                Some(literal) if !literal.is_null() => match literal.value().as_i64() {
                    Some(index) if literal.value().as_str().is_none() => FieldSegment::Index(index),
                    _ => FieldSegment::Key(literal.clone()),
                },
                _ => match key.dtype() {
                    dtype if is_integer(dtype) => FieldSegment::Index(0),
                    _ => FieldSegment::Key(
                        Literal::new(key.dtype().clone(), Scalar::Null)
                            .map_err(|error| typing_error(format_smolstr!("{error}")))?,
                    ),
                },
            };
            return Ok(segment
                .apply_field(&fields[0])?
                .with_name(SmolStr::new(expression.to_string()))
                .with_nullable(true));
        }
        Function::Slice => {
            for bound in fields.iter().skip(1) {
                if !is_integer(bound.dtype()) && !matches!(bound.dtype(), DataType::Null) {
                    return Err(typing_error(format_smolstr!(
                        "expected a whole position to slice at, got {}",
                        bound.dtype()
                    )));
                }
            }
            return Ok(FieldSegment::range(None, None)
                .apply_field(&fields[0])?
                .with_name(SmolStr::new(expression.to_string()))
                .with_nullable(true));
        }
    };
    // Every function answers null for a null argument. The two that take a
    // fallback are the exception: they answer null only when the last argument
    // - the one that runs out of alternatives - can itself be null.
    let nullable = match function {
        Function::Coalesce | Function::IfNull => fields.last().is_none_or(Field::is_nullable),
        _ => nullable,
    };
    Ok(named(expression, dtype, nullable))
}
