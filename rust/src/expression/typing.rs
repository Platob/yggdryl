//! Where an expression's output type is decided - and the only such place.
//!
//! [`Expression::field`] answers, recursively, the [`Field`] an expression
//! produces against a schema. Every other part of the module asks it rather
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

use super::{Expression, Function, Operator, Safety, Segment};
use crate::{DataType, Error, Field, Result, TimeUnit, Value};

/// The widest exact decimal this crate builds by promotion.
const DECIMAL_LIMIT: u8 = 38;

/// The fewest fractional places an exact quotient keeps.
const MIN_QUOTIENT_SCALE: i8 = 6;

impl Expression {
    /// The [`Field`] this expression produces against a struct root schema.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column, the function, or the two datatypes
    /// when the expression cannot be typed against this schema.
    pub fn field(&self, schema: &Field) -> Result<Field> {
        self.check_budget()?;
        resolve(self, schema)
    }

    /// Return whether this expression answers a boolean against a schema.
    ///
    /// # Errors
    ///
    /// Returns the same error [`Self::field`] would.
    pub fn is_predicate(&self, schema: &Field) -> Result<bool> {
        Ok(matches!(
            self.field(schema)?.data_type(),
            DataType::Boolean | DataType::Null
        ))
    }
}

/// Name one output field after the text that produced it.
fn named(expression: &Expression, data_type: DataType, nullable: bool) -> Field {
    Field::new(SmolStr::new(expression.to_string()), data_type, nullable)
}

fn typing_error(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: reason.into(),
    }
}

#[allow(clippy::too_many_lines)]
fn resolve(expression: &Expression, schema: &Field) -> Result<Field> {
    match expression {
        Expression::Literal(held) => Ok(named(
            expression,
            held.data_type().clone(),
            held.value().is_null(),
        )),
        Expression::Column(name) => schema
            .get_field_by_name(name)
            .cloned()
            .ok_or_else(|| unknown_column(name, schema)),
        Expression::Path(base, steps) => {
            let mut field = resolve(base, schema)?;
            for step in steps.iter() {
                field = step_field(&field, step)?;
            }
            Ok(field.with_name(SmolStr::new(expression.to_string())))
        }
        Expression::Attribute(selector) => Ok(selector.field()),
        // A parameter has no type until it is supplied. `bind` substitutes
        // every one before typing, so a parameter reaching here means the
        // caller asked for a type an unbound expression does not have.
        Expression::Parameter(name) => Err(typing_error(format_smolstr!(
            "expected parameter :{name} to be supplied before typing"
        ))),
        Expression::And(operands) | Expression::Or(operands) => {
            let mut nullable = false;
            for operand in operands.iter() {
                let field = resolve(operand, schema)?;
                require_boolean(&field, expression)?;
                nullable |= field.is_nullable();
            }
            Ok(named(expression, DataType::Boolean, nullable))
        }
        Expression::Not(inner) => {
            let field = resolve(inner, schema)?;
            require_boolean(&field, expression)?;
            Ok(named(expression, DataType::Boolean, field.is_nullable()))
        }
        Expression::Compare(left, comparison, right) => {
            let left = resolve(left, schema)?;
            let right = resolve(right, schema)?;
            common_type(left.data_type(), right.data_type()).ok_or_else(|| {
                typing_error(format_smolstr!(
                    "expected comparable operands, got {} {} {}",
                    left.data_type(),
                    comparison.as_str(),
                    right.data_type()
                ))
            })?;
            // The two distinctness tests answer about nulls, so they are the
            // only comparisons that never produce one.
            let nullable =
                !comparison.is_two_valued() && (left.is_nullable() || right.is_nullable());
            Ok(named(expression, DataType::Boolean, nullable))
        }
        Expression::In(value, list) => {
            let value = resolve(value, schema)?;
            let mut nullable = value.is_nullable();
            for item in list.iter() {
                let item = resolve(item, schema)?;
                common_type(value.data_type(), item.data_type()).ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected every `in` value to be comparable with {}, got {}",
                        value.data_type(),
                        item.data_type()
                    ))
                })?;
                nullable |= item.is_nullable();
            }
            Ok(named(expression, DataType::Boolean, nullable))
        }
        Expression::Between(value, low, high) => {
            let value = resolve(value, schema)?;
            let low = resolve(low, schema)?;
            let high = resolve(high, schema)?;
            for bound in [&low, &high] {
                common_type(value.data_type(), bound.data_type()).ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected `between` bounds comparable with {}, got {}",
                        value.data_type(),
                        bound.data_type()
                    ))
                })?;
            }
            Ok(named(
                expression,
                DataType::Boolean,
                value.is_nullable() || low.is_nullable() || high.is_nullable(),
            ))
        }
        Expression::IsNull(inner) | Expression::IsNotNull(inner) => {
            resolve(inner, schema)?;
            Ok(named(expression, DataType::Boolean, false))
        }
        Expression::Like { value, pattern, .. } | Expression::Glob(value, pattern) => {
            let value = resolve(value, schema)?;
            let pattern = resolve(pattern, schema)?;
            for side in [&value, &pattern] {
                if !is_text(side.data_type()) {
                    return Err(typing_error(format_smolstr!(
                        "expected text on both sides of a pattern match, got {}",
                        side.data_type()
                    )));
                }
            }
            Ok(named(
                expression,
                DataType::Boolean,
                value.is_nullable() || pattern.is_nullable(),
            ))
        }
        Expression::Arithmetic(left, operator, right) => {
            let left = resolve(left, schema)?;
            let right = resolve(right, schema)?;
            let data_type = arithmetic_type(left.data_type(), *operator, right.data_type())
                .ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected arithmetic operands that share a type, got {} {} {}",
                        left.data_type(),
                        operator.as_str(),
                        right.data_type()
                    ))
                })?;
            // Division and remainder produce null on a zero divisor rather
            // than failing a whole batch, so their output is always nullable.
            let nullable = left.is_nullable()
                || right.is_nullable()
                || matches!(operator, Operator::Div | Operator::Rem);
            Ok(named(expression, data_type, nullable))
        }
        Expression::Negate(inner) => {
            let field = resolve(inner, schema)?;
            if !is_signed_numeric(field.data_type()) {
                return Err(typing_error(format_smolstr!(
                    "expected a signed number to negate, got {}",
                    field.data_type()
                )));
            }
            Ok(named(
                expression,
                field.data_type().clone(),
                field.is_nullable(),
            ))
        }
        Expression::Function(function, arguments) => {
            function_field(expression, *function, arguments, schema)
        }
        Expression::Cast(inner, data_type, safety) => {
            let field = resolve(inner, schema)?;
            Ok(named(
                expression,
                data_type.clone(),
                field.is_nullable() || matches!(safety, Safety::Safe),
            ))
        }
        Expression::Case {
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
                unified = Some(unify(unified.as_ref(), then.data_type(), expression)?);
            }
            if let Some(otherwise) = otherwise {
                let otherwise = resolve(otherwise, schema)?;
                nullable |= otherwise.is_nullable();
                unified = Some(unify(unified.as_ref(), otherwise.data_type(), expression)?);
            }
            Ok(named(
                expression,
                unified.unwrap_or(DataType::Null),
                nullable,
            ))
        }
        Expression::Struct(children) => {
            let mut fields = Vec::with_capacity(children.len());
            for (name, value) in children.iter() {
                fields.push(resolve(value, schema)?.with_name(name.clone()));
            }
            Ok(named(expression, DataType::from_fields(fields)?, false))
        }
        Expression::List(items) => {
            let mut unified: Option<DataType> = None;
            let mut nullable = false;
            for item in items.iter() {
                let item = resolve(item, schema)?;
                nullable |= item.is_nullable();
                unified = Some(unify(unified.as_ref(), item.data_type(), expression)?);
            }
            let item = Field::new("item", unified.unwrap_or(DataType::Null), nullable);
            Ok(named(expression, DataType::list(item), false))
        }
        Expression::Map(entries) => {
            let mut keys: Option<DataType> = None;
            let mut values: Option<DataType> = None;
            let mut nullable = false;
            for (key, value) in entries.iter() {
                let key = resolve(key, schema)?;
                let value = resolve(value, schema)?;
                nullable |= value.is_nullable();
                keys = Some(unify(keys.as_ref(), key.data_type(), expression)?);
                values = Some(unify(values.as_ref(), value.data_type(), expression)?);
            }
            let key = Field::new("key", keys.unwrap_or(DataType::Utf8), false);
            let value = Field::new("value", values.unwrap_or(DataType::Null), nullable);
            let entries_field = Field::new("entries", DataType::from_fields([key, value])?, false);
            Ok(named(
                expression,
                DataType::map(entries_field, false)?,
                false,
            ))
        }
    }
}

fn unknown_column(name: &str, schema: &Field) -> Error {
    let mut available: Vec<&str> = schema.fields().iter().map(Field::name).collect();
    available.sort_unstable();
    typing_error(format_smolstr!(
        "expected one of the columns {}, got {name:?}",
        available.join(", ")
    ))
}

fn require_boolean(field: &Field, expression: &Expression) -> Result<()> {
    if matches!(field.data_type(), DataType::Boolean | DataType::Null) {
        return Ok(());
    }
    Err(typing_error(format_smolstr!(
        "expected a boolean operand in {expression}, got {}",
        field.data_type()
    )))
}

/// Unify one more branch type into the type a multi-branch node produces.
fn unify(held: Option<&DataType>, next: &DataType, expression: &Expression) -> Result<DataType> {
    let Some(held) = held else {
        return Ok(next.clone());
    };
    common_type(held, next).ok_or_else(|| {
        typing_error(format_smolstr!(
            "expected every branch of {expression} to share a type, got {held} and {next}"
        ))
    })
}

/// Step one path segment through a field's datatype.
pub(crate) fn step_field(field: &Field, segment: &Segment) -> Result<Field> {
    let data_type = unwrap_dictionary(field.data_type());
    match segment {
        Segment::Field(name) => match data_type {
            DataType::Struct(fields) => fields
                .as_fields()
                .iter()
                .find(|child| child.name().eq_ignore_ascii_case(name))
                .cloned()
                // A struct child reached through a path is nullable even when
                // the child is declared required, because the parent may be
                // null and then the whole path is.
                .map(|child| child.with_nullable(true))
                .ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected a child of {}, got {name:?}",
                        field.data_type()
                    ))
                }),
            DataType::Map(map) => Ok(map_value_field(map)?.with_nullable(true)),
            other => Err(typing_error(format_smolstr!(
                "expected a struct or a map to reach .{name} through, got {other}"
            ))),
        },
        Segment::Index(_) => match data_type {
            DataType::List(item)
            | DataType::ListView(item)
            | DataType::FixedSizeList(item, _)
            | DataType::LargeList(item)
            | DataType::LargeListView(item) => Ok(item.as_ref().clone().with_nullable(true)),
            other => Err(typing_error(format_smolstr!(
                "expected a list to index into, got {other}"
            ))),
        },
        Segment::Key(key) => match data_type {
            DataType::Map(map) => {
                let keys = map_key_field(map)?;
                common_type(keys.data_type(), key.data_type()).ok_or_else(|| {
                    typing_error(format_smolstr!(
                        "expected a key comparable with {}, got {}",
                        keys.data_type(),
                        key.data_type()
                    ))
                })?;
                Ok(map_value_field(map)?.with_nullable(true))
            }
            DataType::Struct(fields) => {
                let Some(name) = key.value().as_str() else {
                    return Err(typing_error(format_smolstr!(
                        "expected a text key to reach a struct child, got {}",
                        key.data_type()
                    )));
                };
                fields
                    .as_fields()
                    .iter()
                    .find(|child| child.name().eq_ignore_ascii_case(name))
                    .cloned()
                    .map(|child| child.with_nullable(true))
                    .ok_or_else(|| {
                        typing_error(format_smolstr!(
                            "expected a child of {}, got {name:?}",
                            field.data_type()
                        ))
                    })
            }
            other => Err(typing_error(format_smolstr!(
                "expected a map or a struct to key into, got {other}"
            ))),
        },
    }
}

fn map_key_field(map: &crate::MapType) -> Result<Field> {
    map.entries()
        .get_field(0)
        .cloned()
        .ok_or_else(|| typing_error("expected a map whose entries carry a key field"))
}

fn map_value_field(map: &crate::MapType) -> Result<Field> {
    map.entries()
        .get_field(1)
        .cloned()
        .ok_or_else(|| typing_error("expected a map whose entries carry a value field"))
}

/// Look through a dictionary to the type it encodes.
///
/// A dictionary is a physical encoding, not a logical type: `city = 'Oslo'`
/// means the same thing whether or not the column is dictionary-encoded.
pub(crate) fn unwrap_dictionary(data_type: &DataType) -> &DataType {
    match data_type {
        DataType::Dictionary(dictionary) => unwrap_dictionary(dictionary.value()),
        DataType::RunEndEncoded(encoded) => unwrap_dictionary(encoded.values().data_type()),
        other => other,
    }
}

/// Return whether a datatype holds text.
pub(crate) fn is_text(data_type: &DataType) -> bool {
    matches!(
        unwrap_dictionary(data_type),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Null
    )
}

/// Return whether a datatype holds bytes.
pub(crate) fn is_binary(data_type: &DataType) -> bool {
    matches!(
        unwrap_dictionary(data_type),
        DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_)
    )
}

/// Return whether a datatype holds a whole number.
pub(crate) fn is_integer(data_type: &DataType) -> bool {
    matches!(
        unwrap_dictionary(data_type),
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
pub(crate) fn is_float(data_type: &DataType) -> bool {
    matches!(
        unwrap_dictionary(data_type),
        DataType::Float16 | DataType::Float32 | DataType::Float64
    )
}

/// The precision and scale of an exact decimal, if it is one.
pub(crate) const fn decimal_parts(data_type: &DataType) -> Option<(u8, i8)> {
    match data_type {
        DataType::Decimal32 { precision, scale }
        | DataType::Decimal64 { precision, scale }
        | DataType::Decimal128 { precision, scale }
        | DataType::Decimal256 { precision, scale } => Some((*precision, *scale)),
        _ => None,
    }
}

/// Return whether a datatype can be negated without changing its type.
fn is_signed_numeric(data_type: &DataType) -> bool {
    matches!(
        unwrap_dictionary(data_type),
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
    ) || decimal_parts(unwrap_dictionary(data_type)).is_some()
        || matches!(unwrap_dictionary(data_type), DataType::Duration(_))
}

/// The temporal family and unit of a datatype, if it has one.
pub(crate) const fn temporal_parts(data_type: &DataType) -> Option<(u8, TimeUnit)> {
    match data_type {
        DataType::Date32 => Some((0, TimeUnit::Second)),
        DataType::Date64 => Some((0, TimeUnit::Millisecond)),
        DataType::Time32(unit) | DataType::Time64(unit) => Some((1, *unit)),
        DataType::Timestamp(unit, _) => Some((2, *unit)),
        DataType::Duration(unit) => Some((3, *unit)),
        _ => None,
    }
}

/// How fine a unit is, so two temporals can meet at the finer one.
const fn unit_rank(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Second => 0,
        TimeUnit::Millisecond => 1,
        TimeUnit::Microsecond => 2,
        TimeUnit::Nanosecond => 3,
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => 4,
    }
}

/// The type two operands share, or `None` when they share none.
///
/// This is the whole promotion table, and it is deliberately short.
#[allow(clippy::too_many_lines)]
pub(crate) fn common_type(left: &DataType, right: &DataType) -> Option<DataType> {
    let left = unwrap_dictionary(left);
    let right = unwrap_dictionary(right);
    if left == right {
        return Some(left.clone());
    }
    // A null literal is comparable with anything: `x = null` is unknown, not
    // a type error, and refusing it here would make three-valued logic
    // unspellable.
    if matches!(left, DataType::Null) {
        return Some(right.clone());
    }
    if matches!(right, DataType::Null) {
        return Some(left.clone());
    }
    if is_text(left) && is_text(right) {
        return Some(DataType::Utf8);
    }
    if is_binary(left) && is_binary(right) {
        return Some(DataType::Binary);
    }
    if matches!(left, DataType::Boolean) || matches!(right, DataType::Boolean) {
        return None;
    }
    let left_decimal = decimal_parts(left);
    let right_decimal = decimal_parts(right);
    if left_decimal.is_some() || right_decimal.is_some() {
        // An exact number and an approximate one have no honest meeting point.
        if is_float(left) || is_float(right) {
            return None;
        }
        let (left_precision, left_scale) = left_decimal.unwrap_or((DECIMAL_LIMIT, 0));
        let (right_precision, right_scale) = right_decimal.unwrap_or((DECIMAL_LIMIT, 0));
        // Only a decimal and a whole number meet at a decimal; anything else
        // paired with one has no exact common type.
        if left_decimal.is_none() && !is_integer(left) {
            return None;
        }
        if right_decimal.is_none() && !is_integer(right) {
            return None;
        }
        let scale = left_scale.max(right_scale);
        let integral = left_precision
            .saturating_sub(u8::try_from(left_scale.max(0)).unwrap_or(0))
            .max(right_precision.saturating_sub(u8::try_from(right_scale.max(0)).unwrap_or(0)));
        let precision = integral
            .saturating_add(u8::try_from(scale.max(0)).unwrap_or(0))
            .min(DECIMAL_LIMIT);
        return DataType::decimal128(precision.max(1), scale).ok();
    }
    if is_float(left) && (is_float(right) || is_integer(right)) {
        return Some(DataType::Float64);
    }
    if is_float(right) && is_integer(left) {
        return Some(DataType::Float64);
    }
    if is_integer(left) && is_integer(right) {
        return Some(DataType::Int64);
    }
    match (temporal_parts(left), temporal_parts(right)) {
        (Some((left_family, left_unit)), Some((right_family, right_unit)))
            if left_family == right_family =>
        {
            let unit = if unit_rank(left_unit) >= unit_rank(right_unit) {
                left_unit
            } else {
                right_unit
            };
            Some(match left_family {
                // Two dates meet at the wider of the two spellings.
                0 => {
                    if matches!(left, DataType::Date64) || matches!(right, DataType::Date64) {
                        DataType::Date64
                    } else {
                        DataType::Date32
                    }
                }
                1 => DataType::time(unit).ok()?,
                2 => {
                    let zone = match (left, right) {
                        (DataType::Timestamp(_, Some(zone)), _)
                        | (_, DataType::Timestamp(_, Some(zone))) => Some(zone.clone()),
                        _ => None,
                    };
                    DataType::Timestamp(unit, zone)
                }
                _ => DataType::Duration(unit),
            })
        }
        _ => None,
    }
}

/// The type an arithmetic node produces, or `None` when it has none.
fn arithmetic_type(left: &DataType, operator: Operator, right: &DataType) -> Option<DataType> {
    let left = unwrap_dictionary(left);
    let right = unwrap_dictionary(right);
    // A temporal and a duration are the one mixed-family arithmetic that is
    // meaningful, and it is spelled out rather than promoted.
    match (temporal_parts(left), temporal_parts(right), operator) {
        (Some((family, _)), Some((3, _)), Operator::Add | Operator::Sub) if family != 3 => {
            return Some(left.clone());
        }
        (Some((3, _)), Some((family, _)), Operator::Add) if family != 3 => {
            return Some(right.clone());
        }
        (Some((family, unit)), Some((other, _)), Operator::Sub)
            if family == other && family != 3 =>
        {
            return Some(DataType::Duration(unit));
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
    if is_integer(&shared) || is_float(&shared) || matches!(shared, DataType::Duration(_)) {
        return Some(shared);
    }
    None
}

/// The precision and scale a datatype holds as an exact number.
///
/// A whole number is an exact number of scale zero, and its precision is the
/// digits its width can actually hold - which is what keeps `int32 + decimal`
/// from claiming the 38 digits a decimal could have had.
fn exact_parts(data_type: &DataType) -> Option<(u8, i8)> {
    let data_type = unwrap_dictionary(data_type);
    if let Some(parts) = decimal_parts(data_type) {
        return Some(parts);
    }
    Some(match data_type {
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
    expression: &Expression,
    function: Function,
    arguments: &[Expression],
    schema: &Field,
) -> Result<Field> {
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
        .map(Field::data_type)
        .cloned()
        .unwrap_or(DataType::Null);
    let data_type = match function {
        Function::Lower | Function::Upper | Function::Trim | Function::Substring => {
            if !is_text(&first) {
                return Err(typing_error(format_smolstr!(
                    "expected text for {}, got {first}",
                    function.as_str()
                )));
            }
            DataType::Utf8
        }
        Function::Concat => {
            for field in &fields {
                if !is_text(field.data_type()) {
                    return Err(typing_error(format_smolstr!(
                        "expected text for concat, got {}",
                        field.data_type()
                    )));
                }
            }
            DataType::Utf8
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
                if !is_text(field.data_type()) {
                    return Err(typing_error(format_smolstr!(
                        "expected text for {}, got {}",
                        function.as_str(),
                        field.data_type()
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
        Function::Coalesce | Function::IfNull => {
            let mut unified: Option<DataType> = None;
            for field in &fields {
                unified = Some(unify(unified.as_ref(), field.data_type(), expression)?);
            }
            unified.unwrap_or(DataType::Null)
        }
        Function::Size => {
            if !matches!(
                unwrap_dictionary(&first),
                DataType::List(_)
                    | DataType::ListView(_)
                    | DataType::FixedSizeList(..)
                    | DataType::LargeList(_)
                    | DataType::LargeListView(_)
                    | DataType::Map(_)
            ) {
                return Err(typing_error(format_smolstr!(
                    "expected a list or a map for size, got {first}"
                )));
            }
            DataType::Int64
        }
        Function::Get => {
            let key = fields
                .get(1)
                .ok_or_else(|| typing_error("expected a key for get"))?;
            let segment = match key.data_type() {
                data_type if is_integer(data_type) => Segment::Index(0),
                _ => Segment::Key(
                    crate::TypedValue::from_parts(key.data_type().clone(), Value::Null)
                        .map_err(|error| typing_error(format_smolstr!("{error}")))?,
                ),
            };
            return Ok(step_field(&fields[0], &segment)?
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
    Ok(named(expression, data_type, nullable))
}
