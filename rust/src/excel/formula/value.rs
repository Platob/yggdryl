//! The evaluator's typed scalar operands and uncomputed result.

use crate::Str;
use crate::excel::entry;

use super::functions::Function;
use super::parser::{BinaryOp, Literal};
use super::shape::Held;
use crate::excel::cell::{DateSystem, ExcelError};

/// One slot in the current context's formula-scoped range arena. It cannot
/// escape an evaluation or become a published cell cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReferenceId(pub(crate) usize);

/// A computed scalar or a resolved range handle; no range is materialized.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Operand {
    Blank,
    Reference(ReferenceId),
    Number(f64),
    Text(Str),
    Boolean(bool),
    Error(ExcelError),
}

impl Operand {
    /// Direct arithmetic/function number coercion. Reference and array
    /// arguments retain their source context and decide inclusion upstream.
    /// Entry owns numeric and temporal text grammar; DateSystem owns the
    /// serial, so the same authored date has the correct 1900/1904 number.
    pub(crate) fn number(self, system: DateSystem) -> Option<Result<f64, ExcelError>> {
        match self {
            Self::Blank => Some(Ok(0.0)),
            Self::Reference(_) => None,
            Self::Number(value) => Some(Ok(value)),
            Self::Boolean(value) => Some(Ok(if value { 1.0 } else { 0.0 })),
            Self::Text(value) => {
                let text = value.as_str().trim();
                if let Some((number, _)) = entry::number(text) {
                    Some(Ok(number))
                } else if let Some((temporal, _)) = entry::temporal(text, system) {
                    system
                        .serial_of(&temporal)
                        .ok()
                        .flatten()
                        .map(|(serial, _)| Ok(serial))
                } else {
                    Some(Err(ExcelError::Value))
                }
            }
            Self::Error(error) => Some(Err(error)),
        }
    }

    pub(crate) fn literal(value: &Literal) -> Self {
        match value {
            Literal::Number(value) => Self::Number(*value),
            Literal::Text(value) => Self::Text(value.clone()),
            Literal::Boolean(value) => Self::Boolean(*value),
            Literal::Error(value) => Self::Error(*value),
        }
    }
}

/// An explicit semantic boundary: the prior cached value stays untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unevaluated {
    Held(Held),
    Reference,
    Function(Function),
    Array,
    Spill,
    Binary(BinaryOp),
    NumericPolicy,
    TemporalSerial,
    Coercion,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Computed(Operand),
    Uncomputed(Unevaluated),
}

impl Outcome {
    pub(crate) fn operand(self) -> std::result::Result<Operand, Unevaluated> {
        match self {
            Self::Computed(value) => Ok(value),
            Self::Uncomputed(reason) => Err(reason),
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Typed direct-operand numeric intake over the Entry-owned grammar.

    use super::Operand;
    use crate::Str;
    use crate::excel::{DateSystem, ExcelError};

    /// Parse one text operand without entering the cell-entry formula grammar.
    pub fn number_text(text: &str, system: DateSystem) -> Option<Result<f64, ExcelError>> {
        Operand::Text(Str::new(text)).number(system)
    }

    /// Coerce an already typed Boolean operand.
    pub fn number_boolean(value: bool) -> Option<Result<f64, ExcelError>> {
        Operand::Boolean(value).number(DateSystem::Year1900)
    }

    /// Coerce a missing referenced cell independently of empty text.
    pub fn number_blank() -> Option<Result<f64, ExcelError>> {
        Operand::Blank.number(DateSystem::Year1900)
    }
}
