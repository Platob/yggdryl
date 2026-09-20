//! Canonical text for every term, chosen so that it re-parses.
//!
//! [`Display`](std::fmt::Display) here is not a debugging convenience: it is
//! the inverse of [`FromStr`](std::str::FromStr), and the property test in
//! [`tests`](super::tests) asserts that for every term the module can build.
//! That is what lets a term cross a process boundary as text - into a log
//! line, a Python repr, a manifest property, a record option - and come back
//! the same term.
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

use super::path::write_segments;
use super::selector::{Projection, Selector};
use super::{Comparison, Expression, Filter, Function, Literal, Operator, Safety, Term};
use crate::code_scalars;
use crate::{DataType, Scalar};

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

impl Term {
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

impl fmt::Display for Term {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_at(formatter, self, Precedence::Disjunction)
    }
}

impl fmt::Display for Filter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_at(formatter, self.term(), Precedence::Disjunction)
    }
}

impl fmt::Display for Projection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_at(formatter, self.term(), Precedence::Disjunction)?;
        if let Some(alias) = self.alias() {
            formatter.write_str(" as ")?;
            write_identifier(formatter, alias)?;
        }
        if let Some(dtype) = self.dtype() {
            write!(formatter, " {dtype}")?;
        }
        match self.nullable() {
            Some(true) => formatter.write_str(" null")?,
            Some(false) => formatter.write_str(" not null")?,
            None => {}
        }
        if !self.metadata().is_empty() {
            formatter.write_str(" with (")?;
            for (index, (key, value)) in self.metadata().iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_identifier(formatter, key)?;
                formatter.write_str(" = ")?;
                write_text_literal(formatter, value)?;
            }
            formatter.write_str(")")?;
        }
        Ok(())
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.projections().is_empty() {
            formatter.write_char('*')?;
            if !self.excluded().is_empty() {
                formatter.write_str(" exclude (")?;
                for (index, name) in self.excluded().iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    write_identifier(formatter, name)?;
                }
                formatter.write_str(")")?;
            }
            return Ok(());
        }
        for (index, projection) in self.projections().iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{projection}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Expression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selector(selector) => write!(formatter, "select {selector}"),
            Self::Filter(filter) => write!(formatter, "where {filter}"),
            Self::Plan(plan) => write!(formatter, "{plan}"),
            Self::Sequence(steps) => {
                for (index, step) in steps.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str("; ")?;
                    }
                    write!(formatter, "{step}")?;
                }
                Ok(())
            }
        }
    }
}

/// Write `term`, bracing it when it binds more loosely than `outer`.
#[allow(clippy::too_many_lines)]
pub(crate) fn write_at(
    formatter: &mut fmt::Formatter<'_>,
    term: &Term,
    outer: Precedence,
) -> fmt::Result {
    let own = term.precedence();
    if own < outer {
        formatter.write_char('(')?;
        write_at(formatter, term, Precedence::Disjunction)?;
        return formatter.write_char(')');
    }
    match term {
        Term::Literal(held) => write_literal(formatter, held),
        Term::Path(steps) => write_segments(formatter, steps),
        Term::Attribute(attribute) => write!(formatter, "&holder.{attribute}"),
        Term::Parameter(name) => {
            formatter.write_char(':')?;
            write_identifier(formatter, name)
        }
        Term::And(operands) => write_joined(formatter, operands, " and ", own),
        Term::Or(operands) => write_joined(formatter, operands, " or ", own),
        Term::Not(inner) => {
            formatter.write_str("not ")?;
            write_at(formatter, inner, Precedence::Negation)
        }
        Term::Compare(left, comparison, right) => {
            write_at(formatter, left, Precedence::Additive)?;
            write!(formatter, " {} ", comparison.as_str())?;
            write_at(formatter, right, Precedence::Additive)
        }
        Term::In(value, list) => {
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
        Term::Between(value, low, high) => {
            write_at(formatter, value, Precedence::Additive)?;
            formatter.write_str(" between ")?;
            write_at(formatter, low, Precedence::Additive)?;
            formatter.write_str(" and ")?;
            write_at(formatter, high, Precedence::Additive)
        }
        Term::IsNull(inner) => {
            write_at(formatter, inner, Precedence::Additive)?;
            formatter.write_str(" is null")
        }
        Term::IsNotNull(inner) => {
            write_at(formatter, inner, Precedence::Additive)?;
            formatter.write_str(" is not null")
        }
        Term::Like {
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
        Term::Glob(value, pattern) => {
            write_at(formatter, value, Precedence::Additive)?;
            formatter.write_str(" glob ")?;
            write_at(formatter, pattern, Precedence::Additive)
        }
        Term::Arithmetic(left, operator, right) => {
            write_at(formatter, left, own)?;
            write!(formatter, " {} ", operator.as_str())?;
            // The right operand binds one level tighter, so `a - (b - c)`
            // keeps its braces and `a - b - c` does not grow any.
            write_at(formatter, right, next_tighter(own))
        }
        Term::Negate(inner) => {
            formatter.write_char('-')?;
            write_at(formatter, inner, Precedence::Prefix)
        }
        Term::Function(function, arguments) => {
            formatter.write_str(function.as_str())?;
            write_arguments(formatter, arguments)
        }
        Term::Cast(inner, dtype, safety) => {
            write!(formatter, "{}(", safety.as_str())?;
            write_at(formatter, inner, Precedence::Disjunction)?;
            write!(formatter, " as {dtype})")
        }
        Term::Case {
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
        Term::Struct(children) => {
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
        Term::List(items) => {
            formatter.write_char('[')?;
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_at(formatter, item, Precedence::Disjunction)?;
            }
            formatter.write_char(']')
        }
        Term::Map(entries) => {
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
    operands: &[Term],
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

fn write_arguments(formatter: &mut fmt::Formatter<'_>, arguments: &[Term]) -> fmt::Result {
    formatter.write_char('(')?;
    for (index, argument) in arguments.iter().enumerate() {
        if index != 0 {
            formatter.write_str(", ")?;
        }
        write_at(formatter, argument, Precedence::Disjunction)?;
    }
    formatter.write_char(')')
}

/// Write one identifier, quoting it only when the bare spelling would not
/// come back as itself.
pub(crate) fn write_identifier(formatter: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    super::path::write_identifier(formatter, name)
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
    const RESERVED: [&str; 25] = [
        "and", "or", "not", "is", "null", "true", "false", "in", "between", "like", "ilike",
        "glob", "escape", "case", "when", "then", "else", "end", "cast", "try_cast", "as",
        "distinct", "from", "select", "where",
    ];
    let lowered = name.to_ascii_lowercase();
    RESERVED.contains(&lowered.as_str())
}

/// Write one literal in the spelling that re-parses to it.
pub(super) fn write_literal(formatter: &mut fmt::Formatter<'_>, held: &Literal) -> fmt::Result {
    let dtype = held.dtype();
    let value = held.value();
    // Text is the one bare spelling that is not a word: it prints as the
    // quoted literal the grammar reads back as `utf8`.
    if *dtype == DataType::utf8() {
        if let Some(text) = value.as_str() {
            return write_text_literal(formatter, text);
        }
    }
    if let Some(bare) = bare_literal(dtype, value) {
        return formatter.write_str(&bare);
    }
    if matches!(value, Scalar::Null) {
        return write!(formatter, "{dtype} null");
    }
    match literal_text(dtype, value) {
        Some(text) => {
            write!(formatter, "{dtype} ")?;
            write_text_literal(formatter, &text)
        }
        // A nested literal has no one-line text form, so it prints as the
        // constructor that builds it. This is the only place Display is not
        // literally the inverse of a single token, and it still re-parses.
        None => write_constructed(formatter, dtype, value),
    }
}

/// The bare spelling of a literal, when the grammar's defaults recover it.
fn bare_literal(dtype: &DataType, value: &Scalar) -> Option<SmolStr> {
    match (dtype, value) {
        (DataType::Null, Scalar::Null) => Some(SmolStr::new_static("null")),
        (DataType::Boolean, Scalar::Boolean(held)) => Some(if held.get() {
            SmolStr::new_static("true")
        } else {
            SmolStr::new_static("false")
        }),
        (DataType::Int64, Scalar::Int64(held)) => Some(SmolStr::new(held.to_string())),
        // A non-finite float has no bare spelling, because `nan` and `inf`
        // are column names as often as they are numbers. It falls through to
        // the typed form, where the text is unambiguous.
        (DataType::Float64, Scalar::Float64(held)) if held.as_f64().is_finite() => {
            Some(SmolStr::new(float_text(held.as_f64())))
        }
        _ => None,
    }
}

/// The inner text of a typed literal, or `None` for a value with no text form.
pub(crate) fn literal_text(dtype: &DataType, value: &Scalar) -> Option<SmolStr> {
    match value {
        Scalar::Boolean(held) => Some(SmolStr::new(held.to_string())),
        // Every integer width spells itself through its own Display.
        Scalar::Int8(_)
        | Scalar::Int16(_)
        | Scalar::Int32(_)
        | Scalar::Int64(_)
        | Scalar::UInt8(_)
        | Scalar::UInt16(_)
        | Scalar::UInt32(_)
        | Scalar::UInt64(_)
        | Scalar::Int128(_)
        | Scalar::UInt128(_) => value
            .leaf_display()
            .map(|held| SmolStr::new(held.to_string())),
        Scalar::Float16(held) => Some(SmolStr::new(float_text(held.as_f64()))),
        Scalar::Float32(held) => Some(SmolStr::new(float_text(held.as_f64()))),
        Scalar::Float64(held) => Some(SmolStr::new(float_text(held.as_f64()))),
        Scalar::Decimal32(_)
        | Scalar::Decimal64(_)
        | Scalar::Decimal128(_)
        | Scalar::Decimal256(_) => value.into_decimal_utf8().map(SmolStr::new),
        Scalar::String(held) => Some(held.storage().clone()),
        code_scalars!() => value.code_storage().cloned(),
        Scalar::Version(held) => Some(SmolStr::new(held.to_string())),
        Scalar::Url(held) => Some(SmolStr::new(held.to_string())),
        Scalar::Urn(held) => Some(SmolStr::new(held.to_string())),
        Scalar::Timezone(held) => Some(SmolStr::new(held.as_str())),
        Scalar::MimeType(held) => Some(SmolStr::new(held.as_str())),
        Scalar::MediaType(held) => Some(SmolStr::new(held.to_string())),
        Scalar::Uuid(held) => {
            let mut slot = [0_u8; crate::Uuid::TEXT_LEN];
            Some(SmolStr::new(held.render(&mut slot)))
        }
        // A geometry literal spells its WKB the way a bytes literal does: the
        // expression grammar reads hex back losslessly, which WKT is not.
        Scalar::Bytes(held) => Some(SmolStr::new(hex_text(held.as_bytes()))),
        Scalar::Geometry(held) => Some(SmolStr::new(hex_text(held.as_bytes()))),
        Scalar::Geography(held) => Some(SmolStr::new(hex_text(held.as_bytes()))),
        // Every temporal spells itself the one classic way, which the Arrow
        // cast leaf renders a whole column with.
        Scalar::Date32(_)
        | Scalar::Date64(_)
        | Scalar::Time32(_)
        | Scalar::Time64(_)
        | Scalar::DateTime64(_)
        | Scalar::Duration32(_)
        | Scalar::Duration64(_)
        | Scalar::Interval(_) => value.into_temporal_text(),
        Scalar::Null => matches!(dtype, DataType::Null).then(|| SmolStr::new_static("null")),
        Scalar::Sequence(_) | Scalar::Mapping(_) | Scalar::Struct(_) => None,
        Scalar::Arrow(_) => None,
    }
}

/// Write a nested constant as the constructor that rebuilds it.
fn write_constructed(
    formatter: &mut fmt::Formatter<'_>,
    dtype: &DataType,
    value: &Scalar,
) -> fmt::Result {
    // The element type is carried by the cast around the constructor, so a
    // list of nothing still knows what it is a list of.
    write!(formatter, "cast(")?;
    if let (Some(fields), Some(values)) = (dtype.as_fields(), value.as_sequence()) {
        write_struct_constructor(formatter, fields, values)?;
    } else {
        write_constructor_body(formatter, value)?;
    }
    write!(formatter, " as {dtype})")
}

fn write_struct_constructor(
    formatter: &mut fmt::Formatter<'_>,
    fields: &[crate::Field],
    values: &[Scalar],
) -> fmt::Result {
    formatter.write_str("struct(")?;
    for (index, (field, held)) in fields.iter().zip(values).enumerate() {
        if index != 0 {
            formatter.write_str(", ")?;
        }
        write_constructor_item(formatter, held)?;
        formatter.write_str(" as ")?;
        write_identifier(formatter, field.name())?;
    }
    formatter.write_char(')')
}

fn write_constructor_body(formatter: &mut fmt::Formatter<'_>, value: &Scalar) -> fmt::Result {
    match value {
        Scalar::Sequence(items) => {
            formatter.write_char('[')?;
            for (index, item) in items.rows().unwrap_or_default().iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_constructor_item(formatter, item)?;
            }
            formatter.write_char(']')
        }
        Scalar::Mapping(entries) => {
            formatter.write_char('{')?;
            for (index, (key, held)) in entries.as_slice().iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write_constructor_item(formatter, key)?;
                formatter.write_str(": ")?;
                write_constructor_item(formatter, held)?;
            }
            formatter.write_char('}')
        }
        other => write_constructor_item(formatter, other),
    }
}

fn write_constructor_item(formatter: &mut fmt::Formatter<'_>, value: &Scalar) -> fmt::Result {
    if value.is_container() {
        return write_constructor_body(formatter, value);
    }
    // A leaf inside a constructor is written under the datatype it carries,
    // and the enclosing cast restates it into the declared element type.
    let inferred = Literal::infer(value.clone());
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

/// Lowercase hex, which is how a binary literal is written and read.
pub(crate) fn hex_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
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
