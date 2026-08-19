//! Canonical text for every expression, chosen so that it re-parses.
//!
//! [`Display`](std::fmt::Display) here is not a debugging convenience: it is
//! the inverse of [`FromStr`](std::str::FromStr), and the property test in
//! [`tests`](super::tests) asserts that for every expression the module can
//! build. That is what lets an expression cross a process boundary as text -
//! into a log line, a Python repr, a manifest property - and come back the same
//! expression.
//!
//! Two rules make the inverse hold.
//!
//! *Parentheses are emitted from precedence, not from the input.* The parser
//! discards grouping, so `(a and b) and c` and `a and (b and c)` are one value
//! and print one way. What prints is whatever re-parses to the same tree.
//!
//! *A literal prints its type when the bare spelling would not recover it.*
//! Bare integer text is `int64`, bare float text is `float64`, bare quoted text
//! is `utf8`; every other datatype prints as `<datatype> '<text>'`, which is
//! the same typed-literal spelling the grammar accepts. A decimal therefore
//! survives as an exact decimal at its own scale rather than degrading into
//! whatever a float would have made of it.

use std::fmt::{self, Write as _};

use smol_str::SmolStr;

use super::parser::{Direction, NullsOrder, Order, Projection, Statement};
use super::{Comparison, Expression, Function, Operator, Safety, Segment};
use crate::{DataType, TypedValue, Value};

/// Binding strength, low to high. Only the levels the grammar distinguishes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum Precedence {
    /// `or`
    Disjunction,
    /// `and`
    Conjunction,
    /// `not`
    Negation,
    /// `=`, `<`, `in`, `between`, `like`, `is null`
    Comparison,
    /// `+`, `-`
    Additive,
    /// `*`, `/`, `%`
    Multiplicative,
    /// Unary `-`
    Prefix,
    /// A literal, a name, a call, a path - anything that never needs bracing.
    Atom,
}

impl Expression {
    /// This node's binding strength, which is what decides its parentheses.
    pub(crate) const fn precedence(&self) -> Precedence {
        match self {
            Self::Or(_) => Precedence::Disjunction,
            Self::And(_) => Precedence::Conjunction,
            Self::Not(_) => Precedence::Negation,
            Self::Compare(..)
            | Self::In(..)
            | Self::Between(..)
            | Self::IsNull(_)
            | Self::IsNotNull(_)
            | Self::Like { .. }
            | Self::Glob(..) => Precedence::Comparison,
            Self::Arithmetic(_, Operator::Add | Operator::Sub, _) => Precedence::Additive,
            Self::Arithmetic(..) => Precedence::Multiplicative,
            Self::Negate(_) => Precedence::Prefix,
            _ => Precedence::Atom,
        }
    }
}

impl fmt::Display for Expression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_at(formatter, self, Precedence::Disjunction)
    }
}

/// Write `expression`, bracing it when it binds more loosely than `outer`.
#[allow(clippy::too_many_lines)]
pub(crate) fn write_at(
    formatter: &mut fmt::Formatter<'_>,
    expression: &Expression,
    outer: Precedence,
) -> fmt::Result {
    let own = expression.precedence();
    if own < outer {
        formatter.write_char('(')?;
        write_at(formatter, expression, Precedence::Disjunction)?;
        return formatter.write_char(')');
    }
    match expression {
        Expression::Literal(held) => write_literal(formatter, held),
        Expression::Column(name) => write_identifier(formatter, name),
        Expression::Path(base, steps) => {
            write_at(formatter, base, Precedence::Atom)?;
            for step in steps.iter() {
                write_segment(formatter, step)?;
            }
            Ok(())
        }
        Expression::Attribute(selector) => write!(formatter, "&holder.{selector}"),
        Expression::Parameter(name) => {
            formatter.write_char(':')?;
            write_identifier(formatter, name)
        }
        Expression::And(operands) => write_joined(formatter, operands, " and ", own),
        Expression::Or(operands) => write_joined(formatter, operands, " or ", own),
        Expression::Not(inner) => {
            formatter.write_str("not ")?;
            write_at(formatter, inner, Precedence::Negation)
        }
        Expression::Compare(left, comparison, right) => {
            write_at(formatter, left, Precedence::Additive)?;
            write!(formatter, " {} ", comparison.as_str())?;
            write_at(formatter, right, Precedence::Additive)
        }
        Expression::In(value, list) => {
            write_at(formatter, value, Precedence::Additive)?;
            formatter.write_str(" in (")?;
            for (index, item) in list.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_at(formatter, item, Precedence::Disjunction)?;
            }
            formatter.write_char(')')
        }
        Expression::Between(value, low, high) => {
            write_at(formatter, value, Precedence::Additive)?;
            formatter.write_str(" between ")?;
            write_at(formatter, low, Precedence::Additive)?;
            formatter.write_str(" and ")?;
            write_at(formatter, high, Precedence::Additive)
        }
        Expression::IsNull(inner) => {
            write_at(formatter, inner, Precedence::Additive)?;
            formatter.write_str(" is null")
        }
        Expression::IsNotNull(inner) => {
            write_at(formatter, inner, Precedence::Additive)?;
            formatter.write_str(" is not null")
        }
        Expression::Like {
            value,
            pattern,
            case_insensitive,
            escape,
        } => {
            write_at(formatter, value, Precedence::Additive)?;
            formatter.write_str(if *case_insensitive {
                " ilike "
            } else {
                " like "
            })?;
            write_at(formatter, pattern, Precedence::Additive)?;
            if let Some(escape) = escape {
                formatter.write_str(" escape ")?;
                write_text_literal(formatter, &escape.to_string())?;
            }
            Ok(())
        }
        Expression::Glob(value, pattern) => {
            write_at(formatter, value, Precedence::Additive)?;
            formatter.write_str(" glob ")?;
            write_at(formatter, pattern, Precedence::Additive)
        }
        Expression::Arithmetic(left, operator, right) => {
            write_at(formatter, left, own)?;
            write!(formatter, " {} ", operator.as_str())?;
            // The right operand binds one level tighter, so `a - (b - c)`
            // keeps its braces and `a - b - c` does not grow any.
            write_at(formatter, right, next_tighter(own))
        }
        Expression::Negate(inner) => {
            formatter.write_char('-')?;
            write_at(formatter, inner, Precedence::Prefix)
        }
        Expression::Function(function, arguments) => {
            formatter.write_str(function.as_str())?;
            write_arguments(formatter, arguments)
        }
        Expression::Cast(inner, data_type, safety) => {
            write!(formatter, "{}(", safety.as_str())?;
            write_at(formatter, inner, Precedence::Disjunction)?;
            write!(formatter, " as {data_type})")
        }
        Expression::Case {
            branches,
            otherwise,
        } => {
            formatter.write_str("case")?;
            for (when, then) in branches.iter() {
                formatter.write_str(" when ")?;
                write_at(formatter, when, Precedence::Disjunction)?;
                formatter.write_str(" then ")?;
                write_at(formatter, then, Precedence::Disjunction)?;
            }
            if let Some(otherwise) = otherwise {
                formatter.write_str(" else ")?;
                write_at(formatter, otherwise, Precedence::Disjunction)?;
            }
            formatter.write_str(" end")
        }
        Expression::Struct(children) => {
            formatter.write_str("struct(")?;
            for (index, (name, value)) in children.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_at(formatter, value, Precedence::Disjunction)?;
                formatter.write_str(" as ")?;
                write_identifier(formatter, name)?;
            }
            formatter.write_char(')')
        }
        Expression::List(items) => {
            formatter.write_char('[')?;
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_at(formatter, item, Precedence::Disjunction)?;
            }
            formatter.write_char(']')
        }
        Expression::Map(entries) => {
            formatter.write_char('{')?;
            for (index, (key, value)) in entries.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_at(formatter, key, Precedence::Disjunction)?;
                formatter.write_str(": ")?;
                write_at(formatter, value, Precedence::Disjunction)?;
            }
            formatter.write_char('}')
        }
    }
}

/// The level one step tighter than `level`, saturating at [`Precedence::Atom`].
const fn next_tighter(level: Precedence) -> Precedence {
    match level {
        Precedence::Disjunction => Precedence::Conjunction,
        Precedence::Conjunction => Precedence::Negation,
        Precedence::Negation => Precedence::Comparison,
        Precedence::Comparison => Precedence::Additive,
        Precedence::Additive => Precedence::Multiplicative,
        Precedence::Multiplicative | Precedence::Prefix => Precedence::Prefix,
        Precedence::Atom => Precedence::Atom,
    }
}

fn write_joined(
    formatter: &mut fmt::Formatter<'_>,
    operands: &[Expression],
    separator: &str,
    own: Precedence,
) -> fmt::Result {
    // An empty conjunction is `true` and an empty disjunction is `false`; both
    // print as the constant they mean rather than as nothing at all.
    if operands.is_empty() {
        return formatter.write_str(if own == Precedence::Conjunction {
            "true"
        } else {
            "false"
        });
    }
    for (index, operand) in operands.iter().enumerate() {
        if index != 0 {
            formatter.write_str(separator)?;
        }
        write_at(formatter, operand, next_tighter(own))?;
    }
    Ok(())
}

fn write_arguments(formatter: &mut fmt::Formatter<'_>, arguments: &[Expression]) -> fmt::Result {
    formatter.write_char('(')?;
    for (index, argument) in arguments.iter().enumerate() {
        if index != 0 {
            formatter.write_str(", ")?;
        }
        write_at(formatter, argument, Precedence::Disjunction)?;
    }
    formatter.write_char(')')
}

fn write_segment(formatter: &mut fmt::Formatter<'_>, segment: &Segment) -> fmt::Result {
    match segment {
        Segment::Field(name) => {
            formatter.write_char('.')?;
            write_identifier(formatter, name)
        }
        Segment::Index(index) => write!(formatter, "[{index}]"),
        Segment::Key(key) => {
            formatter.write_char('[')?;
            write_literal(formatter, key)?;
            formatter.write_char(']')
        }
    }
}

/// Write one identifier, quoting it only when the bare spelling would not
/// come back as itself.
pub(crate) fn write_identifier(formatter: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    if is_bare_identifier(name) {
        return formatter.write_str(name);
    }
    formatter.write_char('"')?;
    for character in name.chars() {
        if character == '"' {
            formatter.write_char('"')?;
        }
        formatter.write_char(character)?;
    }
    formatter.write_char('"')
}

/// Write one text value as a single-quoted literal.
pub(crate) fn write_text_literal(formatter: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    formatter.write_char('\'')?;
    for character in text.chars() {
        if character == '\'' {
            formatter.write_char('\'')?;
        }
        formatter.write_char(character)?;
    }
    formatter.write_char('\'')
}

/// Return whether a name is spelled the way the grammar spells a bare name.
#[must_use]
pub(crate) fn is_bare_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !characters.all(|character| character.is_ascii_alphanumeric() || character == '_') {
        return false;
    }
    !is_reserved(name)
}

/// The words the grammar reads as syntax, which a bare name may not be.
///
/// Reserved rather than contextual on purpose: a contextual keyword makes the
/// error for a mistyped operator arrive as "no such column", which is the
/// wrong sentence at the wrong place.
pub(crate) fn is_reserved(name: &str) -> bool {
    const RESERVED: [&str; 33] = [
        "and", "or", "not", "is", "null", "true", "false", "in", "between", "like", "ilike",
        "glob", "escape", "case", "when", "then", "else", "end", "cast", "try_cast", "as",
        "distinct", "from", "select", "where", "order", "by", "asc", "desc", "nulls", "first",
        "last", "limit",
    ];
    let lowered = name.to_ascii_lowercase();
    RESERVED.contains(&lowered.as_str())
}

/// Write one literal in the spelling that re-parses to it.
fn write_literal(formatter: &mut fmt::Formatter<'_>, held: &TypedValue) -> fmt::Result {
    let data_type = held.data_type();
    let value = held.value();
    if let Some(bare) = bare_literal(data_type, value) {
        return formatter.write_str(&bare);
    }
    if matches!(value, Value::Null) {
        return write!(formatter, "{data_type} null");
    }
    match literal_text(data_type, value) {
        Some(text) => {
            write!(formatter, "{data_type} ")?;
            write_text_literal(formatter, &text)
        }
        // A nested literal has no one-line text form, so it prints as the
        // constructor that builds it. This is the only place Display is not
        // literally the inverse of a single token, and it still re-parses.
        None => write_constructed(formatter, data_type, value),
    }
}

/// The bare spelling of a literal, when the grammar's defaults recover it.
fn bare_literal(data_type: &DataType, value: &Value) -> Option<SmolStr> {
    match (data_type, value) {
        (DataType::Null, Value::Null) => Some(SmolStr::new_static("null")),
        (DataType::Boolean, Value::Bool(held)) => Some(if *held {
            SmolStr::new_static("true")
        } else {
            SmolStr::new_static("false")
        }),
        (DataType::Int64, Value::I64(held)) => Some(SmolStr::new(held.to_string())),
        // A non-finite float has no bare spelling, because `nan` and `inf`
        // are column names as often as they are numbers. It falls through to
        // the typed form, where the text is unambiguous.
        (DataType::Float64, Value::F64(held)) if held.as_f64().is_finite() => {
            Some(SmolStr::new(float_text(held.as_f64())))
        }
        _ => None,
    }
}

/// The inner text of a typed literal, or `None` for a value with no text form.
pub(crate) fn literal_text(data_type: &DataType, value: &Value) -> Option<SmolStr> {
    use crate::generic::iso;

    match value {
        Value::Bool(held) => Some(SmolStr::new(held.to_string())),
        Value::I8(held) => Some(SmolStr::new(held.to_string())),
        Value::I16(held) => Some(SmolStr::new(held.to_string())),
        Value::I32(held) => Some(SmolStr::new(held.to_string())),
        Value::I64(held) => Some(SmolStr::new(held.to_string())),
        Value::I128(held) => Some(SmolStr::new(held.to_string())),
        Value::U8(held) => Some(SmolStr::new(held.to_string())),
        Value::U16(held) => Some(SmolStr::new(held.to_string())),
        Value::U32(held) => Some(SmolStr::new(held.to_string())),
        Value::U64(held) => Some(SmolStr::new(held.to_string())),
        Value::U128(held) => Some(SmolStr::new(held.to_string())),
        Value::F32(held) => Some(SmolStr::new(float_text(f64::from(held.as_f32())))),
        Value::F64(held) => Some(SmolStr::new(float_text(held.as_f64()))),
        Value::Decimal(unscaled, scale) => Some(decimal_text(*unscaled, *scale)),
        Value::String(held) => Some(held.clone()),
        Value::Bytes(held) => Some(SmolStr::new(hex_text(held))),
        Value::Date(days) => iso::format_date(*days),
        Value::Time(count, unit) => iso::format_time(*count, *unit),
        Value::Timestamp(count, unit, zone) => iso::format_timestamp(*count, *unit, zone),
        Value::DateTime(count, unit) => iso::format_datetime(*count, *unit),
        Value::Duration(count, unit) => iso::format_duration(*count, *unit),
        Value::Null => matches!(data_type, DataType::Null).then(|| SmolStr::new_static("null")),
        Value::Sequence(_) | Value::Mapping(_) | Value::Record(..) => None,
    }
}

/// Write a nested constant as the constructor that rebuilds it.
fn write_constructed(
    formatter: &mut fmt::Formatter<'_>,
    data_type: &DataType,
    value: &Value,
) -> fmt::Result {
    // The element type is carried by the cast around the constructor, so a
    // list of nothing still knows what it is a list of.
    write!(formatter, "cast(")?;
    write_constructor_body(formatter, value)?;
    write!(formatter, " as {data_type})")
}

fn write_constructor_body(formatter: &mut fmt::Formatter<'_>, value: &Value) -> fmt::Result {
    match value {
        Value::Sequence(items) => {
            formatter.write_char('[')?;
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_constructor_item(formatter, item)?;
            }
            formatter.write_char(']')
        }
        Value::Mapping(entries) => {
            formatter.write_char('{')?;
            for (index, (key, held)) in entries.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_constructor_item(formatter, key)?;
                formatter.write_str(": ")?;
                write_constructor_item(formatter, held)?;
            }
            formatter.write_char('}')
        }
        Value::Record(data_type, values) => {
            let names: Vec<&str> = data_type
                .as_fields()
                .unwrap_or_default()
                .iter()
                .map(crate::Field::name)
                .collect();
            formatter.write_str("struct(")?;
            for (index, held) in values.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_constructor_item(formatter, held)?;
                formatter.write_str(" as ")?;
                write_identifier(formatter, names.get(index).copied().unwrap_or("_"))?;
            }
            formatter.write_char(')')
        }
        other => write_constructor_item(formatter, other),
    }
}

fn write_constructor_item(formatter: &mut fmt::Formatter<'_>, value: &Value) -> fmt::Result {
    if value.is_container() {
        return write_constructor_body(formatter, value);
    }
    // A leaf inside a constructor is written under the datatype it carries,
    // and the enclosing cast restates it into the declared element type.
    let inferred = TypedValue::from_value(value.clone());
    match inferred {
        Ok(typed) => write_literal(formatter, &typed),
        Err(_) => formatter.write_str("null"),
    }
}

/// The shortest text that reads back as this float, keeping it a float.
///
/// `{:?}` on an `f64` is Rust's round-trip formatting and always leaves a `.`
/// or an exponent behind, which is exactly what stops `2` from coming back as
/// an integer.
pub(crate) fn float_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    format!("{value:?}")
}

/// The text of an exact decimal, at the scale it was stored with.
pub(crate) fn decimal_text(unscaled: i128, scale: i8) -> SmolStr {
    let mut text = String::new();
    let negative = unscaled < 0;
    let digits = unscaled.unsigned_abs().to_string();
    if negative {
        text.push('-');
    }
    match scale {
        // A negative scale multiplies, so the zeros are real digits.
        ..=-1 => {
            text.push_str(&digits);
            for _ in 0..scale.unsigned_abs() {
                text.push('0');
            }
        }
        0 => text.push_str(&digits),
        _ => {
            let places = usize::from(scale.unsigned_abs());
            if digits.len() > places {
                let split = digits.len() - places;
                text.push_str(&digits[..split]);
                text.push('.');
                text.push_str(&digits[split..]);
            } else {
                text.push_str("0.");
                for _ in 0..places - digits.len() {
                    text.push('0');
                }
                text.push_str(&digits);
            }
        }
    }
    SmolStr::new(text)
}

/// Lowercase hex, which is how a binary literal is written and read.
pub(crate) fn hex_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

impl fmt::Display for Segment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_segment(formatter, self)
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for Operator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for Safety {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ascending => "asc",
            Self::Descending => "desc",
        })
    }
}

impl fmt::Display for NullsOrder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::First => "nulls first",
            Self::Last => "nulls last",
        })
    }
}

impl fmt::Display for Order {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_at(formatter, self.expression(), Precedence::Disjunction)?;
        write!(formatter, " {}", self.direction())?;
        if let Some(nulls) = self.nulls() {
            write!(formatter, " {nulls}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Projection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_at(formatter, self.expression(), Precedence::Disjunction)?;
        if let Some(alias) = self.alias() {
            formatter.write_str(" as ")?;
            write_identifier(formatter, alias)?;
        }
        Ok(())
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("select ")?;
        if self.projections().is_empty() {
            formatter.write_char('*')?;
        } else {
            for (index, projection) in self.projections().iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write!(formatter, "{projection}")?;
            }
        }
        if let Some(predicate) = self.predicate() {
            formatter.write_str(" where ")?;
            write_at(formatter, predicate, Precedence::Disjunction)?;
        }
        if !self.ordering().is_empty() {
            formatter.write_str(" order by ")?;
            for (index, order) in self.ordering().iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write!(formatter, "{order}")?;
            }
        }
        if let Some(limit) = self.limit() {
            write!(formatter, " limit {limit}")?;
        }
        Ok(())
    }
}
